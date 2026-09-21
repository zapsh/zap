//! 环境能力门禁（feature gate）。
//!
//! 问题背景：菜单存在 `menus` 表里，但**能不能用某个菜单取决于宿主机当前装了什么**。
//! 典型是「容器管理」——没装 Docker 的机器不需要这个入口，装了以后又希望它自己出现。
//!
//! 于是把菜单的两件事拆开：
//!
//! - **结构**（`menus` 表）：路径、标题、图标、排序、授权，照旧支持线上热改，不用发版；
//! - **可见性**（`menus.feature`）：空串表示常显；填了值的交给本模块判定，
//!   能力不可用则该条目（连同子树，见 `build_menu_tree`）消失。
//!
//! ## 与 `hidden` 的分工
//!
//! - `hidden`：**人工开关**——这个入口在页面里另有去处（如 SSL 的 DNS 服务商改成抽屉）；
//! - `feature`：**环境开关**——组件没装就根本没有入口。
//!
//! 两者是 AND 关系，别混用，否则后人只能靠注释猜这条为什么看不见。
//!
//! ## 缓存
//!
//! 探测要连 Docker daemon，绝不能挂在每次请求菜单上，所以这里进程内缓存 + TTL；
//! 安装 / 卸载属于低频动作，由调用方显式 `invalidate_all()` 收口（见各 install handler）。
//!
//! 缓存还有一层职责：**stale-while-error**。探测失败要区分两种——
//! exec 明确回答「没装」要立刻生效（卸载可被感知），而「连不上 exec / 超时」
//! 不代表能力消失（zapexec 重启、IPC 抖动都会碰上），此时沿用上一次的结论，
//! 且快照只缓存较短的时间，免得把入口跟着抖动一起藏掉，也免得恢复后迟迟不收敛。
//! 完全没有历史可沿用时才 fail-closed。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use zap_proto::Request;

use crate::zapexec;

/// 探测成功结果的有效期：装包 / 卸载是分钟级的动作，60s 足够感知。
const CACHE_TTL: Duration = Duration::from_secs(60);

/// 「这一轮没探到」时的有效期：明显短于正常值。
///
/// 场景是 zapexec 重启 / IPC 抖动：此时值是沿用旧值得来的，不算可靠，
/// 缓存太久会让 exec 恢复后迟迟才重新收敛。
const DEGRADED_TTL: Duration = Duration::from_secs(10);

/// 单次探测超时：daemon 卡住时不能把登录、菜单请求一起拖死。
const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 容器管理（`/docker`）：宿主机装了 Docker 才给入口。
pub const FEATURE_DOCKER: &str = "docker";

/// 所有已实现的能力。菜单管理页的「环境门禁」下拉据此出选项，
/// 新增能力只要在这里登记，不必再改前端。
pub const ALL: &[&str] = &[FEATURE_DOCKER];

type FeatureMap = HashMap<String, bool>;

/// 探测结果：`Some` 表示探到了；`None` 表示**链路不通**——exec 没起、IPC 失败、超时。
///
/// 「exec 明确回答没装」和「压根连不上 exec」必须分开：后者不代表能力消失，
/// 按 fail-closed 处理会让 Docker 入口随着 zapexec 重启而闪没。
type ProbeResult = Option<bool>;

/// 一轮结果的快照。
struct Snapshot {
    at: Instant,
    map: FeatureMap,
    /// 本轮至少有一项是「沿用旧值 / 兜底」得来的，结果可靠性打折 → 缓存时间也打折。
    degraded: bool,
}

impl Snapshot {
    fn ttl(&self) -> Duration {
        if self.degraded {
            DEGRADED_TTL
        } else {
            CACHE_TTL
        }
    }
}

static CACHE: OnceLock<RwLock<Option<Snapshot>>> = OnceLock::new();

fn cache_slot() -> &'static RwLock<Option<Snapshot>> {
    CACHE.get_or_init(|| RwLock::new(None))
}

/// 该能力当前是否可用。`feature` 为空串一律放行（绝大多数菜单没有门禁）。
pub async fn available(feature: &str) -> bool {
    let feature = feature.trim();
    if feature.is_empty() {
        return true;
    }
    // 未知 feature 放行而不是藏起来：写错名字的后果应该是「多显示一个入口」，
    // 而不是「某个功能彻底消失却查不出原因」。
    snapshot().await.get(feature).copied().unwrap_or(true)
}

/// 能力清单及其当前可用性：给菜单管理页的「环境门禁」下拉用。
///
/// 顺带把可用性一并返回，管理员才能在下拉旁看到「当前环境未安装」——
/// 否则配完一脸困惑：侧栏为什么没有。
pub async fn catalog() -> Vec<(String, bool)> {
    let snap = snapshot().await;
    ALL.iter()
        .map(|k| (k.to_string(), snap.get(*k).copied().unwrap_or(true)))
        .collect()
}

/// 能力变更后立即失效缓存：下次请求菜单时重新探测，不等 TTL。
pub fn invalidate_all() {
    if let Ok(mut guard) = cache_slot().write() {
        *guard = None;
    }
}

/// 只失效某一个能力的记录（`invalidate_all` 的细化版本）。
pub fn invalidate(feature: &str) {
    if let Ok(mut guard) = cache_slot().write()
        && let Some(s) = guard.as_mut()
        && s.map.remove(feature.trim()).is_some()
    {
        // 抠掉一项就当作整体过期：留着半张 map 会让 `available` 对这个「未知 key」
        // fail-open 地返回 true，然后在 TTL 内一直错下去。ALL 目前只有一项，整体重探成本可忽略。
        s.at = Instant::now() - s.ttl();
    }
}

async fn snapshot() -> FeatureMap {
    if let Ok(guard) = cache_slot().read()
        && let Some(s) = guard.as_ref()
        && s.at.elapsed() < s.ttl()
    {
        return s.map.clone();
    }
    refresh().await
}

/// 上一份快照（可能已过期）：拿它是为了在探不到时兜底，而不是为了跳过探测。
fn previous_map() -> Option<FeatureMap> {
    cache_slot().read().ok()?.as_ref().map(|s| s.map.clone())
}

async fn refresh() -> FeatureMap {
    let prev = previous_map();

    let mut results: Vec<(String, ProbeResult)> = Vec::with_capacity(ALL.len());
    for key in ALL {
        let value = match *key {
            FEATURE_DOCKER => probe_docker().await,
            // 没实现探测的能力不算「降格」，交给 `available` 的 fail-open 放行
            _ => None,
        };
        results.push((key.to_string(), value));
    }

    let (map, degraded) = merge_results(prev.as_ref(), &results);
    if let Ok(mut guard) = cache_slot().write() {
        *guard = Some(Snapshot {
            at: Instant::now(),
            map: map.clone(),
            degraded,
        });
    }
    map
}

/// 合并本轮探测结果与上一份快照（stale-while-error）。
///
/// 抽成纯函数是为了能单测——这块逻辑的坑在于「探测失败」有两种，
/// 混在一起就会让菜单跟着 exec 抖动。
fn merge_results(
    prev: Option<&FeatureMap>,
    results: &[(String, ProbeResult)],
) -> (FeatureMap, bool) {
    let mut map = FeatureMap::new();
    let mut degraded = false;
    for (key, value) in results {
        match value {
            Some(v) => {
                map.insert(key.clone(), *v);
            }
            None => {
                degraded = true;
                // 沿用上一次的结论；连历史都没有才 fail-closed
                let fallback = prev.and_then(|p| p.get(key)).copied().unwrap_or(false);
                map.insert(key.clone(), fallback);
            }
        }
    }
    (map, degraded)
}

/// Docker 是否可用：复用 zapexec 的 `/docker/status` 探测（一次 IPC call）。
///
/// 只看 `installed`（**是否装过**），不看 `daemon`：
/// 装了但没起 daemon 属于运行态问题，容器页自己会提示去启动，入口不该跟着消失
/// ——否则用户会以为 Docker 没了而重装一遍。
///
/// exec 侧的 `installed` 已与 socket 解耦（CLI 二进制 / 服务单元 / 包记录
/// 任一命中即算装过），因为 daemon 一停 socket 就没了，拿它当「装没装」必然误判。
///
/// 响应里没有 installed 字段视为链路异常（不是「没装」），交给 `merge_results` 兜底。
async fn probe_docker() -> ProbeResult {
    match tokio::time::timeout(DETECT_TIMEOUT, zapexec::call(Request::DockerStatus)).await {
        Ok(Ok(resp)) => resp
            .data
            .as_ref()
            .and_then(|d| d.get("installed"))
            .and_then(|v| v.as_bool()),
        // exec 没起 / IPC 不通 / 超时：不是「没装」，交给 `merge_results` 兜底
        Ok(Err(_)) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没打标记的菜单（绝大多数）必须恒可见：feature 是「能力门禁」，
    /// 绝不能因为加了这一层导致普通菜单整片消失。
    #[tokio::test]
    async fn empty_feature_is_always_available() {
        assert!(available("").await);
    }

    /// 写了但没实现的能力放行而不是隐藏：写错 key 的后果应该是「多一个入口」，
    /// 而不是「功能彻底消失且查不出为什么」。
    #[tokio::test]
    async fn unknown_feature_fails_open() {
        assert!(available("nonexistent-feature").await);
    }

    fn results(kv: &[(&str, ProbeResult)]) -> Vec<(String, ProbeResult)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    /// 核心诉求：zapexec 重启期间 Docker 入口不能闪没。探不到时沿用上一次的结论。
    #[test]
    fn probe_failure_keeps_previous_value() {
        let mut prev = FeatureMap::new();
        prev.insert(FEATURE_DOCKER.to_string(), true);
        let (map, degraded) = merge_results(Some(&prev), &results(&[(FEATURE_DOCKER, None)]));

        assert!(map[FEATURE_DOCKER]);
        // 降格后只缓存 DEGRADED_TTL，exec 恢复能尽快重新收敛
        assert!(degraded);
        assert_eq!(DEGRADED_TTL.as_secs(), 10);
        assert!(DEGRADED_TTL < CACHE_TTL);
    }

    /// 探测明确回答「没装」时，哪怕历史是 true 也要跟着变——卸载必须能被感知。
    #[test]
    fn explicit_not_installed_overrides_history() {
        let mut prev = FeatureMap::new();
        prev.insert(FEATURE_DOCKER.to_string(), true);
        let (map, degraded) =
            merge_results(Some(&prev), &results(&[(FEATURE_DOCKER, Some(false))]));

        assert!(!map[FEATURE_DOCKER]);
        assert!(!degraded);
    }

    /// 冷启动第一次就探不到（exec 还没起）：没有历史可沿用，只能 fail-closed。
    #[test]
    fn no_history_fails_closed() {
        let (map, degraded) = merge_results(None, &results(&[(FEATURE_DOCKER, None)]));
        assert!(!map[FEATURE_DOCKER]);
        assert!(degraded);
    }
}
