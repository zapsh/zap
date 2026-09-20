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

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use zap_proto::Request;

use crate::zapexec;

/// 探测结果有效期：装包 / 卸载是分钟级的动作，60s 足够感知。
const CACHE_TTL: Duration = Duration::from_secs(60);

/// 单次探测超时：daemon 卡住时不能把登录、菜单请求一起拖死。
const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 容器管理（`/docker`）：宿主机装了 Docker 才给入口。
pub const FEATURE_DOCKER: &str = "docker";

type FeatureMap = HashMap<String, bool>;
static CACHE: OnceLock<RwLock<Option<(Instant, FeatureMap)>>> = OnceLock::new();

fn cache_slot() -> &'static RwLock<Option<(Instant, FeatureMap)>> {
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

/// 能力变更后立即失效缓存：下次请求菜单时重新探测，不等 TTL。
pub fn invalidate_all() {
    if let Ok(mut guard) = cache_slot().write() {
        *guard = None;
    }
}

/// 只失效某一个能力的记录（`invalidate_all` 的细化版本）。
pub fn invalidate(feature: &str) {
    if let Ok(mut guard) = cache_slot().write()
        && let Some((_, map)) = guard.as_mut()
    {
        map.remove(feature.trim());
    }
}

async fn snapshot() -> FeatureMap {
    if let Ok(guard) = cache_slot().read()
        && let Some((at, map)) = guard.as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return map.clone();
    }
    refresh().await
}

async fn refresh() -> FeatureMap {
    let mut map = FeatureMap::new();
    map.insert(FEATURE_DOCKER.to_string(), probe_docker().await);
    if let Ok(mut guard) = cache_slot().write() {
        *guard = Some((Instant::now(), map.clone()));
    }
    map
}

/// Docker 是否可用：复用 zapexec 的 `/docker/status` 探测（一次 IPC call）。
///
/// 取 `installed`（socket 存在 ≈ 装过）即可：daemon 临时没起属于运行态问题，
/// 由容器页自己提示，不该把入口藏掉——否则用户会以为 Docker 没了而重复安装。
async fn probe_docker() -> bool {
    match tokio::time::timeout(DETECT_TIMEOUT, zapexec::call(Request::DockerStatus)).await {
        Ok(Ok(resp)) => resp
            .data
            .as_ref()
            .and_then(|d| d.get("installed"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        // 连不通 exec / 探测失败：当成「没有该能力」，入口先不出现，
        // 但结果不进长期缓存由 TTL 兜底，装好后会重新出现。
        Ok(Err(_)) | Err(_) => false,
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
}
