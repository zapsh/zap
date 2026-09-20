//! 应用商店的「槽位」路径规则 —— `zapd`（非特权）与 `zapexec`（root）共用同一份。
//!
//! 背景：早期一个包只有一个登记目录 `apps/<category>/<name>/`，于是
//! - 多版本 PHP：第二次安装覆盖第一次的 `meta.yaml` / `info.yaml`；
//! - 多站点 WordPress：第二个站点装完，第一个站点的 `site_root` / `db_name` 就没了。
//!
//! 现在把「实例（instance）」提升为一等公民：一个包的每个安装实例独占一个槽位，
//! 槽位路径由这里**唯一**决定，两边都不许再自己拼，否则会出现「装在一处、卸载找另一处」。

use std::path::{Path, PathBuf};

/// 单实例包的默认实例名。
pub const DEFAULT_INSTANCE: &str = "default";

/// 站点类应用的分类名（包目录 `webapps/`）。
pub const WEBAPPS_CATEGORY: &str = "webapps";

/// 站点实例的 instance 前缀：`site:3` 表示装在 3 号站点。
pub const SITE_INSTANCE_PREFIX: &str = "site:";

/// 已安装软件根目录：`{data}/apps`
pub fn apps_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("apps")
}

/// 用户私有数据根目录：`{data}/users/<user>`
pub fn user_dir(data_dir: &Path, user: &str) -> PathBuf {
    data_dir.join("users").join(user)
}

/// 用户私有 webapps 根目录：`{data}/users/<user>/webapps`
pub fn user_webapps_dir(data_dir: &Path, user: &str) -> PathBuf {
    user_dir(data_dir, user).join(WEBAPPS_CATEGORY)
}

/// 实例标识：决定一个安装落在哪个目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot<'a> {
    /// 包分类（`application` / `webapps` / `library` / `database` / `infra` …）
    pub category: &'a str,
    /// 包名
    pub name: &'a str,
    /// 实例名（全局类：`default` / `74` / 用户自定义；站点类：`site:<id>`）
    pub instance: &'a str,
    /// 归属面板用户（Linux 账号名）；站点类必填
    pub owner: Option<&'a str>,
    /// 站点 id；站点类必填
    pub site_id: Option<&'a str>,
}

impl<'a> Slot<'a> {
    /// 全局类实例（装在 `apps/` 下）。
    pub fn global(category: &'a str, name: &'a str, instance: &'a str) -> Self {
        Self {
            category,
            name,
            instance,
            owner: None,
            site_id: None,
        }
    }

    /// 站点类实例（装在**用户私有目录**下）。
    ///
    /// webapps 的站点、程序文件、数据库都归站点账号所有，登记信息跟着账号走，
    /// 天然带上「属于谁」这一维度，也避免不同用户同名站点互相覆盖。
    pub fn site(name: &'a str, owner: &'a str, site_id: &'a str) -> Self {
        Self {
            category: WEBAPPS_CATEGORY,
            name,
            instance: "", // 占位：站点槽位用 site_id 定位，instance 由下面统一派生
            owner: Some(owner),
            site_id: Some(site_id),
        }
    }

    /// 站点实例的实例名：`site:<id>`
    pub fn site_instance(site_id: &str) -> String {
        format!("{SITE_INSTANCE_PREFIX}{site_id}")
    }

    /// 是否为站点类槽位
    pub fn is_site(&self) -> bool {
        self.category == WEBAPPS_CATEGORY && self.owner.is_some() && self.site_id.is_some()
    }

    /// 稳定主键：`<category>/<name>@<instance>`。
    ///
    /// 前端与 API 用它定位实例（而不是只有 `pkg_path`，后者在多实例时不再唯一）。
    pub fn key(&self) -> String {
        if self.is_site() {
            let site_id = self.site_id.unwrap_or_default();
            format!(
                "{}/{}@{}",
                self.category,
                self.name,
                Self::site_instance(site_id)
            )
        } else {
            format!("{}/{}@{}", self.category, self.name, self.instance)
        }
    }

    /// 槽位目录。
    pub fn dir(&self, data_dir: &Path) -> PathBuf {
        if self.is_site() {
            // {data}/users/<owner>/webapps/<name>/<site_id>/
            user_webapps_dir(data_dir, self.owner.unwrap_or_default())
                .join(self.name)
                .join(self.site_id.unwrap_or_default())
        } else {
            // {data}/apps/<category>/<name>/<instance>/
            apps_dir(data_dir)
                .join(self.category)
                .join(self.name)
                .join(self.instance)
        }
    }
}

/// 站点槽位的 instance 是否形如 `site:<id>`，是则返回 id。
pub fn parse_site_instance(instance: &str) -> Option<&str> {
    instance.strip_prefix(SITE_INSTANCE_PREFIX)
}

/// 从实例目录反推「归属用户」（仅站点类有）：`…/users/<owner>/webapps/…`
pub fn owner_from_dir(dir: &Path) -> Option<String> {
    let mut cur = dir.parent()?;
    while let Some(parent) = cur.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some("users") {
            return cur.file_name().and_then(|n| n.to_str()).map(String::from);
        }
        cur = parent;
    }
    None
}

/// 是否为旧布局槽位：`{data}/apps/<category>/<name>/`（meta.yaml 直接在包目录下）。
///
/// 旧版本安装的包（php、wordpress…）都落在这里，必须继续识别为 `instance=default`，
/// 否则升级面板后已装应用会「消失」。
pub fn is_legacy_slot(dir: &Path) -> bool {
    dir.join("meta.yaml").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_slots_do_not_overlap() {
        let data = Path::new("/srv/zap/data");
        let a = Slot::global("application", "php", "74").dir(data);
        let b = Slot::global("application", "php", "83").dir(data);
        assert_ne!(a, b);
        assert!(a.ends_with("apps/application/php/74"));
        assert!(b.ends_with("apps/application/php/83"));
    }

    #[test]
    fn site_slots_live_under_user_dir() {
        let data = Path::new("/srv/zap/data");
        let s = Slot::site("wordpress", "admin", "3").dir(data);
        assert!(s.ends_with("users/admin/webapps/wordpress/3"));
        assert_eq!(owner_from_dir(&s).as_deref(), Some("admin"));
        assert_eq!(
            Slot::site("wordpress", "admin", "3").key(),
            "webapps/wordpress@site:3"
        );
    }
}
