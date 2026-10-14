// SPDX-License-Identifier: AGPL-3.0-only
//! 系统软件源管理：发行版识别、镜像站目录、第三方源预设与「换源」内容生成。
//!
//! `routers/system_repo.rs`：
//! 1. 认识系统：解析 `/etc/os-release`，映射到 apt 系（debian / ubuntu）或
//!    rpm 系（rocky / almalinux / centos 等）；
//! 2. 生成内容：镜像站目录 + 换源时的 sources / repo 内容
//!

use serde::Serialize;
use std::path::Path;

/// 镜像站（国内 / 国外）。
#[derive(Debug, Clone, Serialize)]
pub struct MirrorSite {
    pub id: &'static str,
    pub label: &'static str,
    /// `cn` = 国内镜像，`intl` = 官方 / 海外
    pub region: &'static str,
    /// 镜像根 URL（结尾带 `/`），如 `https://mirrors.aliyun.com/debian/`
    pub url: &'static str,
}

/// 第三方源预设。
#[derive(Debug, Clone, Serialize)]
pub struct ThirdPartyPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    /// `apt` / `rpm`：不适配当前发行版的预设不会下发给前端
    pub family: String,
    /// 生成的源文件名（不含目录）
    pub file: String,
    /// 生成的源文件内容
    pub content: String,
}

/// 发行版识别结果。
#[derive(Debug, Clone, Serialize)]
pub struct DistroInfo {
    pub id: String,
    pub version: String,
    pub pretty_name: String,
    /// `debian` / `rhel`：未知发行版为空，前端据此禁用换源
    pub family: String,
    /// apt 系代号（bookworm / jammy…）；rpm 系为空
    pub suite: String,
}

/// 解析 `/etc/os-release` 的 (ID, VERSION_ID, PRETTY_NAME)。
pub fn os_release() -> (String, String, String) {
    let content = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut id = String::new();
    let mut version = String::new();
    let mut pretty = String::new();
    for line in content.lines() {
        let val = |line: &str, key: &str| -> Option<String> {
            line.strip_prefix(key)
                .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        };
        if let Some(v) = val(line, "ID=") {
            id = v;
        } else if let Some(v) = val(line, "VERSION_ID=") {
            version = v;
        } else if let Some(v) = val(line, "PRETTY_NAME=") {
            pretty = v;
        }
    }
    (id, version, pretty)
}

/// debian 版本号 → 代号。
pub fn debian_suite(version: &str) -> Option<&'static str> {
    match version.trim() {
        "13" => Some("trixie"),
        "12" => Some("bookworm"),
        "11" => Some("bullseye"),
        "10" => Some("buster"),
        _ => None,
    }
}

/// ubuntu 版本号 → 代号。
pub fn ubuntu_suite(version: &str) -> Option<&'static str> {
    match version.trim() {
        "24.04" => Some("noble"),
        "22.04" => Some("jammy"),
        "20.04" => Some("focal"),
        "18.04" => Some("bionic"),
        _ => None,
    }
}

/// 发行版归类：apt 系还是 rpm 系。
pub fn family_of(id: &str) -> &'static str {
    match id {
        "debian" | "ubuntu" | "linuxmint" | "kali" | "deepin" | "uos" => "debian",
        "rhel" | "centos" | "rocky" | "almalinux" | "fedora" | "openeuler" | "anolis" | "amzn"
        | "oracle" | "opencloudos" | "tencentos" => "rhel",
        _ => "",
    }
}

/// 识别当前系统（拿不到 os-release 时返回空 family，前端禁用换源）。
pub fn detect() -> DistroInfo {
    let (id, version, pretty) = os_release();
    let family = family_of(&id).to_string();
    let suite = match id.as_str() {
        "debian" => debian_suite(&version).unwrap_or_default().to_string(),
        "ubuntu" => ubuntu_suite(&version).unwrap_or_default().to_string(),
        _ => String::new(),
    };
    DistroInfo {
        id,
        version,
        pretty_name: pretty,
        family,
        suite,
    }
}

// ── 镜像站目录 ────────────────────────────────────────────

/// apt 系（debian / ubuntu）镜像站。
///
/// `url` 是**镜像根**（不带 `debian/`、`ubuntu/` 段），生成内容时按发行版拼接，
/// 这样同一批国内镜像对 debian / ubuntu 都能用。
///
/// 海外镜像按发行版分开选（地址取自 debian.org/mirror/list 与 Ubuntu 官方
/// archive 国家镜像）：日 / 新没有 Debian 国家别名，取列表里较稳的
/// `ftp.jaist.ac.jp` 与 `mirror.djvg.sg`；Ubuntu 侧用 `xx.archive.ubuntu.com`。
pub fn apt_mirrors(distro: &str) -> Vec<MirrorSite> {
    let cn = vec![
        MirrorSite {
            id: "aliyun",
            label: "Aliyun",
            region: "cn",
            url: "https://mirrors.aliyun.com/",
        },
        MirrorSite {
            id: "tuna",
            label: "Tsinghua University",
            region: "cn",
            url: "https://mirrors.tuna.tsinghua.edu.cn/",
        },
        MirrorSite {
            id: "ustc",
            label: "USTC University",
            region: "cn",
            url: "https://mirrors.ustc.edu.cn/",
        },
        MirrorSite {
            id: "tencent",
            label: "Tencent Cloud",
            region: "cn",
            url: "https://mirrors.cloud.tencent.com/",
        },
        MirrorSite {
            id: "huawei",
            label: "Huawei Cloud",
            region: "cn",
            url: "https://repo.huaweicloud.com/",
        },
        MirrorSite {
            id: "netease",
            label: "NetEase",
            region: "cn",
            url: "https://mirrors.163.com/",
        },
    ];
    let intl: Vec<MirrorSite> = if distro == "ubuntu" {
        vec![
            MirrorSite {
                id: "official",
                label: "Official Source",
                region: "intl",
                url: "http://archive.ubuntu.com/",
            },
            MirrorSite {
                id: "us",
                label: "United States",
                region: "intl",
                url: "http://us.archive.ubuntu.com/",
            },
            MirrorSite {
                id: "jp",
                label: "Japan",
                region: "intl",
                url: "http://jp.archive.ubuntu.com/",
            },
            MirrorSite {
                id: "sg",
                label: "Singapore",
                region: "intl",
                url: "http://sg.archive.ubuntu.com/",
            },
            MirrorSite {
                id: "uk",
                label: "United Kingdom",
                region: "intl",
                url: "http://gb.archive.ubuntu.com/",
            },
        ]
    } else {
        vec![
            MirrorSite {
                id: "official",
                label: "Official Source",
                region: "intl",
                url: "https://deb.debian.org/",
            },
            MirrorSite {
                id: "us",
                label: "United States",
                region: "intl",
                url: "http://ftp.us.debian.org/",
            },
            MirrorSite {
                id: "jp",
                label: "Japan",
                region: "intl",
                url: "http://ftp.jaist.ac.jp/",
            },
            MirrorSite {
                id: "sg",
                label: "Singapore",
                region: "intl",
                url: "http://mirror.djvg.sg/",
            },
            MirrorSite {
                id: "uk",
                label: "United Kingdom",
                region: "intl",
                url: "http://ftp.uk.debian.org/",
            },
        ]
    };
    [cn, intl].concat()
}

/// rpm 系官方源。
/// centos / tencentos 的官方源目录不是 `{distro}/` 形式（分别是
/// `9-stream/`、`tlinux/`），无法套用下面的生成规则，同样不提供。
pub fn rpm_official(distro: &str) -> Option<&'static str> {
    match distro {
        "rocky" => Some("https://dl.rockylinux.org/pub/"),
        "almalinux" => Some("https://repo.almalinux.org/almalinux/"),
        "openeuler" => Some("https://repo.openeuler.org/"),
        "fedora" => Some("https://dl.fedoraproject.org/pub/fedora/"),
        "oracle" => Some("https://yum.oracle.com/"),
        "amzn" => Some("https://cdn.amazonlinux.com/"),
        _ => None,
    }
}

/// 该发行版是否支持在线换源。
///
/// 前提是它的 repo 目录结构与 Rocky 系一致：
/// `{mirror}/{distro}/$releasever/{Section}/$basearch/os/`。Anolis
/// （`mirrors.aliyun.com/anolis/...`）与 OpenCloudOS（`.../opencloudos/...`）
/// 实测符合；Fedora / openEuler / TencentOS / Amazon Linux / RHEL（订阅源）
/// 结构不同，强行拼只会写出 404 的源，因此禁用换源（仍可查看 / 编辑文件）。
pub fn supports_mirror_switch(distro_id: &str) -> bool {
    matches!(
        distro_id,
        "rocky" | "almalinux" | "centos" | "anolis" | "opencloudos"
    )
}

/// rpm 系镜像站（rocky / almalinux / centos 同路径规则）。
///
/// **加新镜像就往下边加一条**：`url` 填镜像根（结尾带 `/`），该根下必须存在
/// 对应发行版目录（`{url}rocky/`、`{url}almalinux/`…），否则拼出来的 baseurl
/// 会 404 —— 换源预览里能直接点开验证。`id` 全库唯一，`label` 是下拉展示名。
pub fn rpm_mirrors(distro: &str) -> Vec<MirrorSite> {
    let mut sites = vec![
        MirrorSite {
            id: "aliyun",
            label: "Aliyun",
            region: "cn",
            url: "https://mirrors.aliyun.com/",
        },
        MirrorSite {
            id: "tuna",
            label: "Tsinghua University",
            region: "cn",
            url: "https://mirrors.tuna.tsinghua.edu.cn/",
        },
        MirrorSite {
            id: "ustc",
            label: "USTC University",
            region: "cn",
            url: "https://mirrors.ustc.edu.cn/",
        },
        MirrorSite {
            id: "tencent",
            label: "Tencent Cloud",
            region: "cn",
            url: "https://mirrors.cloud.tencent.com/",
        },
        MirrorSite {
            id: "huawei",
            label: "Huawei Cloud",
            region: "cn",
            url: "https://repo.huaweicloud.com/",
        },
    ];
    // 官方源：只在该发行版确实有可用地址时才出现
    if let Some(url) = rpm_official(distro) {
        sites.push(MirrorSite {
            id: "official",
            label: "Official Source",
            region: "intl",
            url,
        });
    }
    sites
}

pub fn mirrors_for(family: &str, distro_id: &str) -> Vec<MirrorSite> {
    match family {
        "debian" => apt_mirrors(distro_id),
        "rhel" => rpm_mirrors(distro_id),
        _ => vec![],
    }
}

fn mirror_by_id<'a>(sites: &'a [MirrorSite], id: &str) -> Option<&'a MirrorSite> {
    sites.iter().find(|m| m.id == id)
}

/// 把镜像根拼成某发行版的 apt 源地址（`https://mirrors.aliyun.com/` → `.../debian/`）。
pub fn apt_base(mirror_url: &str, distro_id: &str) -> String {
    format!("{}/{}/", mirror_url.trim_end_matches('/'), distro_id)
}

// ── apt 系换源内容生成 ─────────────────────────────────────

/// 生成 debian 的 sources.list 内容（`mirror_base` 需已含 `debian/` 段）。
pub fn debian_sources(mirror_base: &str, suite: &str) -> String {
    // 安全源在镜像根的上一级：.../debian/ → .../debian-security/
    let root = mirror_base.trim_end_matches('/');
    let parent = root.rsplit_once('/').map(|(h, _)| h).unwrap_or(root);
    let security = format!("{parent}/debian-security/");
    format!(
        "# Generated by ZAP Panel: {suite} (original file backed up in data/backups/osrepo) \n\
         deb {mirror_base} {suite} main contrib non-free non-free-firmware\n\
         # deb-src {mirror_base} {suite} main contrib non-free non-free-firmware\n\
         deb {mirror_base} {suite}-updates main contrib non-free non-free-firmware\n\
         deb {security} {suite}-security main contrib non-free non-free-firmware\n"
    )
}

/// 生成 ubuntu 的 sources.list 内容。
pub fn ubuntu_sources(mirror_url: &str, suite: &str) -> String {
    format!(
        "# Generated by ZAP Panel: {suite} (original file backed up in data/backups/osrepo) \n\
         deb {mirror_url} {suite} main restricted universe multiverse\n\
         # deb-src {mirror_url} {suite} main restricted universe multiverse\n\
         deb {mirror_url} {suite}-updates main restricted universe multiverse\n\
         deb {mirror_url} {suite}-backports main restricted universe multiverse\n\
         deb {mirror_url} {suite}-security main restricted universe multiverse\n"
    )
}

// ── rpm 系换源内容生成（原地改 baseurl）────────────────────

/// rpm repo 文件里可识别的仓库段名。
const RPM_SECTIONS: &[&str] = &[
    "BaseOS",
    "AppStream",
    "CRB",
    "PowerTools",
    "extras",
    "devel",
    "highavailability",
    "nfv",
    "resilientstorage",
    "rt",
    "sap",
    "saphana",
];

/// 从 baseurl 里解析仓库段名，如
/// `https://dl.rockylinux.org/pub/pub/rocky/9/BaseOS/x86_64/os/` → `BaseOS`。
fn section_from_url(url: &str) -> Option<&'static str> {
    RPM_SECTIONS
        .iter()
        .find(|s| url.contains(&format!("/{s}/")))
        .copied()
}

/// 从 metalink / mirrorlist URL 里解析仓库段名，如
/// `...mirrorlist?arch=$basearch&repo=AppStream-$releasever&...` → `AppStream`。
///
/// Rocky / AlmaLinux / EPEL 默认都用 metalink 自动选源，`baseurl` 是注释掉的 ——
/// 只改 baseurl 对它们完全无效，必须认这条路径。
fn section_from_metalink(url: &str) -> Option<String> {
    let idx = url.find("repo=")?;
    let rest = &url[idx + "repo=".len()..];
    let value = rest.split('&').next().unwrap_or(rest);
    // `AppStream-$releasever` → `AppStream`；`epel-8` 这类整段不在名单里，返回 None
    let value = value.split("-$releasever").next().unwrap_or(value);
    if value.is_empty() {
        return None;
    }
    RPM_SECTIONS
        .iter()
        .find(|s| s.eq_ignore_ascii_case(value))
        .map(|s| s.to_string())
}

/// 源地址布局：发行版源与 EPEL 的目录规则不同。
#[derive(Debug, Clone)]
pub enum RepoLayout {
    /// `{mirror}/{distro}/$releasever/{Section}/x86_64/os/`
    Distro(String),
    /// `{mirror}/epel/$releasever/Everything/$basearch/`
    Epel,
}

impl RepoLayout {
    fn base(&self, mirror_url: &str) -> String {
        match self {
            RepoLayout::Distro(d) => format!("{}/{}/", mirror_url.trim_end_matches('/'), d),
            RepoLayout::Epel => format!("{}/epel/", mirror_url.trim_end_matches('/')),
        }
    }

    /// 按段名生成直连镜像的 baseurl；段名未知时 EPEL 也能给（它不需要段名）。
    ///
    /// 架构用 `$basearch` 而不是写死 `x86_64`：发行版自己的 repo 文件就是
    /// `$basearch`，写死会让 arm64 机器换完源全是 404。
    fn baseurl_for(&self, base: &str, section: Option<&str>) -> Option<String> {
        match self {
            RepoLayout::Distro(_) => {
                section.map(|s| format!("{base}$releasever/{s}/$basearch/os/"))
            }
            RepoLayout::Epel => Some(format!("{base}$releasever/Everything/$basearch/")),
        }
    }
}

/// 重写一个 rpm repo 文件的「源行」：
/// - `baseurl=`（未注释）→ 换成直连镜像；
/// - `metalink=` / `mirrorlist=` → 注释原行并补一条 `baseurl=`（发行版默认
///   走 metalink 自动选源，不处理它就等于没换）；
/// - 其它行（gpgkey / enabled / 注释…）原样保留 —— 与 sshd_config 的「原地改键」
///   同一思路，整文件重渲染必然丢信息。
///
/// 同一文件里已有改写过的 baseurl 时，metalink 只注释不再补 baseurl，避免重复。
pub fn rewrite_repo_lines(content: &str, mirror_url: &str, layout: &RepoLayout) -> String {
    let base = layout.base(mirror_url);
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if let Some(rest) = trimmed.strip_prefix("baseurl=") {
            let section = section_from_url(rest.trim()).map(|s| s.to_string());
            if let Some(new_url) = layout.baseurl_for(&base, section.as_deref()) {
                if let Some(s) = &section {
                    seen.push(s.clone());
                }
                out.push(format!("{indent}baseurl={new_url}"));
                continue;
            }
            out.push(line.to_string());
            continue;
        }

        if let Some(rest) = trimmed
            .strip_prefix("metalink=")
            .or_else(|| trimmed.strip_prefix("mirrorlist="))
        {
            let section = section_from_metalink(rest.trim());
            match layout.baseurl_for(&base, section.as_deref()) {
                Some(new_url) => {
                    let already = section
                        .as_deref()
                        .map(|s| seen.iter().any(|x| x.eq_ignore_ascii_case(s)))
                        .unwrap_or(false);
                    out.push(format!("{indent}#{trimmed}"));
                    if !already {
                        if let Some(s) = &section {
                            seen.push(s.clone());
                        }
                        out.push(format!("{indent}baseurl={new_url}"));
                    }
                }
                // 认不出段名时不敢乱补 baseurl，原样保留
                None => out.push(line.to_string()),
            }
            continue;
        }
        out.push(line.to_string());
    }
    if content.ends_with('\n') {
        out.push(String::new());
    }
    out.join("\n")
}

/// 用新镜像重写发行版 repo 文件（BaseOS / AppStream / CRB…）。
pub fn rewrite_repo_baseurls(content: &str, distro: &str, mirror_url: &str) -> String {
    rewrite_repo_lines(content, mirror_url, &RepoLayout::Distro(distro.to_string()))
}

/// 用新镜像重写 EPEL repo 文件（`Everything` 布局，无 `os/` 后缀）。
pub fn rewrite_epel_baseurls(content: &str, mirror_url: &str) -> String {
    rewrite_repo_lines(content, mirror_url, &RepoLayout::Epel)
}

/// 该 rpm repo 文件是否属于发行版自带源（可按换源重写）。
pub fn is_distro_repo_file(path: &str, distro_id: &str) -> bool {
    let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.to_lowercase();
    match distro_id {
        "rocky" => name.starts_with("rocky"),
        "almalinux" => name.starts_with("almalinux"),
        "centos" => name.starts_with("centos"),
        "fedora" => name.starts_with("fedora"),
        "openeuler" => name.starts_with("openeuler"),
        "anolis" => name.starts_with("anolis"),
        _ => false,
    }
}

/// 该 rpm repo 文件是否是 EPEL 源。
pub fn is_epel_repo_file(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_lowercase().contains("epel"))
        .unwrap_or(false)
}

// ── 第三方源预设 ──────────────────────────────────────────
pub const PRESET_META: &[(&str, &str, &str)] = &[
    (
        "epel",
        "EPEL",
        "Extra Packages for Enterprise Linux, support RHEL / CentOS / Rocky / AlmaLinux",
    ),
    (
        "backports",
        "Debian Backports",
        "Debian Official Backports: Newer packages from testing, use with caution",
    ),
];

/// 从 sources.list 内容里解析当前 apt 镜像根（`https://mirrors.aliyun.com/debian/`）。
///
/// 兼容 `deb [arch=amd64 signed-by=...] <url> <suite> ...` 的选项形式。
pub fn apt_base_from_content(content: &str) -> Option<String> {
    for line in content.lines() {
        let Some(rest) = line.trim().strip_prefix("deb ") else {
            continue;
        };
        for token in rest.split_whitespace() {
            if token.starts_with('[') {
                continue; // [arch=amd64 signed-by=…] 之类的选项
            }
            if token.starts_with("http") {
                return Some(token.to_string());
            }
            break; // 第一个非选项段不是 URL（如 cdrom:），这一行没戏
        }
    }
    None
}

/// 当前 apt 镜像根：读 `/etc/apt/sources.list`（全局可读，无需 root），
/// 读不到时回退阿里云 —— Backports 等附加源应与主源同镜像。
pub fn current_apt_base() -> String {
    std::fs::read_to_string("/etc/apt/sources.list")
        .ok()
        .as_deref()
        .and_then(apt_base_from_content)
        .unwrap_or_else(|| "https://mirrors.aliyun.com/debian/".to_string())
}

/// 生成第三方源预设内容；`family` 不匹配时返回 None（前端不展示）。
pub fn preset_content(id: &str, distro: &DistroInfo, family: &str) -> Option<(String, String)> {
    let suite = distro.suite.as_str();
    let major = distro.version.split('.').next().unwrap_or_default();
    let (file, content): (String, String) = match (id, family) {
        // ── rpm 系 ──
        ("epel", "rhel") => (
            "epel-aliyun.repo".into(),
            format!(
                "[epel]\n\
                 name=Extra Packages for Enterprise Linux {major} - $basearch\n\
                 baseurl=https://mirrors.aliyun.com/epel/$releasever/Everything/$basearch/\n\
                 enabled=1\n\
                 gpgcheck=1\n\
                 gpgkey=https://mirrors.aliyun.com/epel/RPM-GPG-KEY-EPEL-$releasever\n"
            ),
        ),
        ("remi", "rhel") => (
            "remi-php.repo".into(),
            "[remi-safe]\n\
             name=Remi's RPM repository for Enterprise Linux - safe\n\
             baseurl=https://mirrors.aliyun.com/remi/enterprise/$releasever/safe/$basearch/\n\
             enabled=1\n\
             gpgcheck=0\n\
             \n\
             [remi-php83]\n\
             name=Remi's PHP 8.3 repository\n\
             baseurl=https://mirrors.aliyun.com/remi/enterprise/$releasever/php83/$basearch/\n\
             enabled=0\n\
             gpgcheck=0\n"
                .into(),
        ),
        // ── apt 系：Backports（与主源同镜像） ──
        ("backports", "debian") if !suite.is_empty() => (
            format!("{suite}-backports.list"),
            format!(
                "# Generated by ZAP Panel: {suite}-backports (newer packages from testing)\n\
                 deb {} {suite}-backports main contrib non-free non-free-firmware\n",
                current_apt_base()
            ),
        ),
        _ => return None,
    };
    Some((file, content))
}

/// 当前发行版可用的第三方源预设（含生成好的内容）。
pub fn presets_for(distro: &DistroInfo) -> Vec<ThirdPartyPreset> {
    PRESET_META
        .iter()
        .filter_map(|(id, label, desc)| {
            preset_content(id, distro, &distro.family).map(|(file, content)| ThirdPartyPreset {
                id,
                label,
                desc,
                family: distro.family.clone(),
                file,
                content,
            })
        })
        .collect()
}

/// 生成换源预览：apt 系整文件替换，rpm 系逐文件原地改 baseurl。
///
/// `contents` 是调用方（router）通过 exec 读到的候选文件内容，
/// 键为文件路径 —— 读文件需要 root，这里保持纯函数以便单测。
pub fn switch_preview(
    distro: &DistroInfo,
    mirror_id: &str,
    contents: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    // rpm 系里目录结构与 Rocky 不一致的发行版不换源，避免拼出 404 地址
    // （apt 系是整文件生成，不受此限）
    if distro.family == "rhel" && !supports_mirror_switch(&distro.id) {
        return None;
    }
    let sites = mirrors_for(&distro.family, &distro.id);
    let mirror = mirror_by_id(&sites, mirror_id)?;
    match distro.family.as_str() {
        "debian" => {
            let base = apt_base(mirror.url, &distro.id);
            let content = match distro.id.as_str() {
                "debian" if !distro.suite.is_empty() => debian_sources(&base, &distro.suite),
                "ubuntu" if !distro.suite.is_empty() => ubuntu_sources(&base, &distro.suite),
                _ => return None,
            };
            Some(vec![("/etc/apt/sources.list".to_string(), content)])
        }
        "rhel" => {
            let out: Vec<(String, String)> = contents
                .iter()
                .filter(|(path, _)| is_distro_repo_file(path, &distro.id))
                .map(|(path, content)| {
                    (
                        path.clone(),
                        rewrite_repo_baseurls(content, &distro.id, mirror.url),
                    )
                })
                // 重写后与原内容一致 = 没识别出这个文件的源行格式（如 Fedora /
                // openEuler 的目录结构与 Rocky 系不同），不写入、也不让用户误解
                .filter(|(path, new)| {
                    contents
                        .iter()
                        .find(|(p, _)| p == path)
                        .map(|(_, old)| old != new)
                        .unwrap_or(true)
                })
                .collect();
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}

/// 换源时是否同时把 EPEL 指向同一镜像（仅 rpm 系有意义）。
pub fn epel_switch_preview(
    mirror_url: &str,
    contents: &[(String, String)],
) -> Vec<(String, String)> {
    contents
        .iter()
        .filter(|(path, _)| is_epel_repo_file(path))
        .map(|(path, content)| (path.clone(), rewrite_epel_baseurls(content, mirror_url)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distro(id: &str, version: &str, suite: &str) -> DistroInfo {
        DistroInfo {
            id: id.into(),
            version: version.into(),
            pretty_name: format!("{id} {version}"),
            family: family_of(id).into(),
            suite: suite.into(),
        }
    }

    #[test]
    fn suite_mapping() {
        assert_eq!(debian_suite("12"), Some("bookworm"));
        assert_eq!(debian_suite("13"), Some("trixie"));
        assert_eq!(debian_suite("9"), None);
        assert_eq!(ubuntu_suite("22.04"), Some("jammy"));
        assert_eq!(ubuntu_suite("24.04"), Some("noble"));
        assert_eq!(ubuntu_suite("25.04"), None);
    }

    #[test]
    fn family_mapping() {
        assert_eq!(family_of("ubuntu"), "debian");
        assert_eq!(family_of("debian"), "debian");
        assert_eq!(family_of("rocky"), "rhel");
        assert_eq!(family_of("openeuler"), "rhel");
        assert_eq!(family_of("arch"), "");
    }

    #[test]
    fn debian_sources_has_security() {
        let s = debian_sources("https://mirrors.aliyun.com/debian/", "bookworm");
        assert!(s.contains("deb https://mirrors.aliyun.com/debian/ bookworm main"));
        assert!(s.contains("deb https://mirrors.aliyun.com/debian-security/ bookworm-security"));
        // 每行一条，不能挤成一行
        assert_eq!(s.lines().count(), 5);
    }

    #[test]
    fn ubuntu_sources_pockets() {
        let s = ubuntu_sources("https://mirrors.tuna.tsinghua.edu.cn/ubuntu/", "jammy");
        assert!(s.contains("jammy-updates main restricted universe multiverse"));
        assert!(s.contains("jammy-security main restricted universe multiverse"));
        assert_eq!(s.lines().count(), 6);
    }

    #[test]
    fn rewrite_only_baseurl_lines() {
        let content = "\
[baseos]
name=Rocky Linux 9 - BaseOS
baseurl=https://dl.rockylinux.org/pub/pub/rocky/9/BaseOS/x86_64/os/
gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-Official
enabled=1
";
        let out = rewrite_repo_baseurls(content, "rocky", "https://mirrors.aliyun.com/");
        assert!(
            out.contains(
                "baseurl=https://mirrors.aliyun.com/rocky/$releasever/BaseOS/$basearch/os/"
            )
        );
        // 非 baseurl 行原样保留
        assert!(out.contains("gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-Official"));
        assert!(out.contains("name=Rocky Linux 9 - BaseOS"));
    }

    #[test]
    fn rewrite_handles_metalink_default_repos() {
        // Rocky 8/9 默认形态：baseurl 被注释，真正生效的是 metalink
        let content = "\
[appstream]
name=Rocky Linux $releasever - AppStream
metalink=https://mirrors.rockylinux.org/mirrorlist?arch=$basearch&repo=AppStream-$releasever&arch=$basearch
#baseurl=https://dl.rockylinux.org/contentdir/$releasever/AppStream/$basearch/os/
gpgcheck=1
enabled=1
countme=1
gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-rockyofficial
";
        let out = rewrite_repo_baseurls(content, "rocky", "https://mirrors.aliyun.com/");
        // metalink 被注释，并补上直连 baseurl
        assert!(out.contains("#metalink=https://mirrors.rockylinux.org/mirrorlist?"));
        assert!(out.contains(
            "baseurl=https://mirrors.aliyun.com/rocky/$releasever/AppStream/$basearch/os/"
        ));
        // 其余行原样保留
        assert!(out.contains("gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-rockyofficial"));
        assert!(out.contains("countme=1"));
        // 原本注释掉的 baseurl 不该被激活
        assert!(out.contains("#baseurl=https://dl.rockylinux.org/contentdir/"));
    }

    #[test]
    fn rewrite_metalink_plus_active_baseurl_dedupes() {
        // 少数文件 metalink 与 baseurl 同时生效：只保留一条改写后的 baseurl
        let content = "\
[baseos]
name=Rocky Linux 9 - BaseOS
baseurl=https://dl.rockylinux.org/pub/pub/rocky/9/BaseOS/x86_64/os/
metalink=https://mirrors.rockylinux.org/mirrorlist?arch=$basearch&repo=BaseOS-$releasever
enabled=1
";
        let out = rewrite_repo_baseurls(content, "rocky", "https://mirrors.aliyun.com/");
        assert_eq!(
            out.matches("baseurl=https://mirrors.aliyun.com/rocky/")
                .count(),
            1,
            "同一段只应有一条生效 baseurl"
        );
        assert!(out.contains("#metalink="));
    }

    #[test]
    fn rewrite_unknown_metalink_section_keeps_line() {
        let content = "[custom]\nmetalink=https://example.com/mirrorlist?repo=weird-9\nenabled=1\n";
        let out = rewrite_repo_baseurls(content, "rocky", "https://mirrors.aliyun.com/");
        assert!(out.contains("metalink=https://example.com/mirrorlist?repo=weird-9"));
    }

    #[test]
    fn epel_rewrite_handles_metalink() {
        let content = "\
[epel]
name=Extra Packages for Enterprise Linux 8 - $basearch
metalink=https://mirrors.fedoraproject.org/metalink?repo=epel-8&arch=$basearch
enabled=1
gpgcheck=1
gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-8
";
        let out = rewrite_epel_baseurls(content, "https://mirrors.aliyun.com/");
        assert!(out.contains("#metalink="));
        assert!(
            out.contains(
                "baseurl=https://mirrors.aliyun.com/epel/$releasever/Everything/$basearch/"
            )
        );
        assert!(out.contains("gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-8"));
    }

    #[test]
    fn rewrite_skips_unknown_sections() {
        let content = "[custom]\nbaseurl=https://example.com/foo/\nenabled=1\n";
        let out = rewrite_repo_baseurls(content, "rocky", "https://mirrors.aliyun.com/");
        assert!(out.contains("baseurl=https://example.com/foo/"));
    }

    #[test]
    fn rpm_switch_preview_only_distro_files() {
        let d = distro("rocky", "9", "");
        let contents = vec![
            (
                "/etc/yum.repos.d/Rocky-BaseOS.repo".into(),
                "[baseos]\nbaseurl=https://dl.rockylinux.org/pub/pub/rocky/9/BaseOS/x86_64/os/\n"
                    .into(),
            ),
            (
                "/etc/yum.repos.d/docker-ce.repo".into(),
                "[docker]\nbaseurl=https://download.docker.com/linux/centos/9/x86_64/stable/\n"
                    .into(),
            ),
        ];
        let preview = switch_preview(&d, "aliyun", &contents).expect("应有预览");
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].0, "/etc/yum.repos.d/Rocky-BaseOS.repo");
        assert!(preview[0].1.contains("mirrors.aliyun.com/rocky"));
    }

    #[test]
    fn rpm_switch_preview_skips_unrewritable_files() {
        // 目录结构不认识（Fedora 形态）：重写后无变化，不该进预览
        let d = distro("fedora", "41", "");
        let contents = vec![(
            "/etc/yum.repos.d/fedora.repo".into(),
            "[fedora]\nmetalink=https://mirrors.fedoraproject.org/mirrorlist?repo=fedora-41&arch=x86_64\n"
                .into(),
        )];
        assert!(
            switch_preview(&d, "aliyun", &contents).is_none(),
            "认不出源行格式时应返回 None（接口层据此报错）"
        );
    }

    #[test]
    fn rpm_official_is_distro_specific() {
        assert_eq!(
            rpm_official("rocky"),
            Some("https://dl.rockylinux.org/pub/")
        );
        assert_eq!(
            rpm_official("almalinux"),
            Some("https://repo.almalinux.org/almalinux/")
        );
        assert_eq!(
            rpm_official("openeuler"),
            Some("https://repo.openeuler.org/")
        );
        // 拿不准路径规则的一律 None：绝不兜底到 Rocky
        assert_eq!(rpm_official("anolis"), None);
        assert_eq!(rpm_official("tencentos"), None);
        assert_eq!(
            rpm_official("centos"),
            None,
            "centos 官方目录是 9-stream/，不符合约定"
        );
        assert_eq!(rpm_official("rhel"), None, "RHEL 官方源需订阅");
        assert_eq!(rpm_official("whatever"), None);
        // 下拉里「官方源」只在有地址时出现
        assert!(rpm_mirrors("rocky").iter().any(|m| m.id == "official"));
        assert!(!rpm_mirrors("anolis").iter().any(|m| m.id == "official"));
        assert!(!rpm_mirrors("tencentos").iter().any(|m| m.id == "official"));
        // 国内镜像对所有 rpm 发行版都在
        for d in ["rocky", "almalinux", "centos", "anolis", "opencloudos"] {
            assert!(rpm_mirrors(d).iter().any(|m| m.id == "aliyun"));
        }
    }

    #[test]
    fn switchable_only_for_rocky_like_layout() {
        // 目录结构与 Rocky 系一致（{mirror}/{distro}/$releasever/{Section}/$basearch/os/）
        for d in ["rocky", "almalinux", "centos", "anolis", "opencloudos"] {
            assert!(supports_mirror_switch(d), "{d} 应支持换源");
        }
        // 结构不同：不换源，避免拼出 404
        for d in [
            "fedora",
            "openeuler",
            "tencentos",
            "amzn",
            "oracle",
            "rhel",
            "arch",
        ] {
            assert!(!supports_mirror_switch(d), "{d} 不该支持换源");
        }
        // 不支持的发行版连预览都不给
        let d = distro("openeuler", "22.03", "");
        assert!(switch_preview(&d, "aliyun", &[]).is_none());
    }

    #[test]
    fn apt_switch_preview() {
        let d = distro("ubuntu", "22.04", "jammy");
        let preview = switch_preview(&d, "ustc", &[]).expect("应有预览");
        assert_eq!(preview[0].0, "/etc/apt/sources.list");
        assert!(
            preview[0]
                .1
                .contains("mirrors.ustc.edu.cn/ubuntu/ jammy main")
        );
    }

    #[test]
    fn presets_are_distro_aware() {
        let ubuntu = distro("ubuntu", "22.04", "jammy");
        let ids: Vec<&str> = presets_for(&ubuntu).iter().map(|p| p.id).collect();
        assert!(ids.contains(&"backports"), "apt 系应有 Backports 预设");
        assert!(!ids.contains(&"epel"), "EPEL 不该出现在 debian 系");
        // 应用级源（面板自己会装）不该再作为预设下发
        for app_level in ["docker-ce", "nodesource", "nginx", "mariadb", "postgresql"] {
            assert!(!ids.contains(&app_level), "{app_level} 不该再是预设");
        }
        // Backports 内容带发行版代号，且与主源同镜像
        let backports = presets_for(&ubuntu)
            .into_iter()
            .find(|p| p.id == "backports")
            .expect("backports 预设应存在");
        assert_eq!(backports.file, "jammy-backports.list");
        assert!(backports.content.contains("jammy-backports main"));

        let rocky = distro("rocky", "9", "");
        let ids: Vec<&str> = presets_for(&rocky).iter().map(|p| p.id).collect();
        assert!(ids.contains(&"epel"));
        assert!(!ids.contains(&"backports"), "rpm 系没有 Backports");
        assert!(
            !ids.contains(&"remi"),
            "PHP 源由面板应用商店管理，不再作为预设"
        );
    }

    #[test]
    fn intl_mirrors_are_distro_aware() {
        let debian = apt_mirrors("debian");
        let intl: Vec<&MirrorSite> = debian.iter().filter(|m| m.region == "intl").collect();
        let ids: Vec<&str> = intl.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["official", "us", "jp", "sg", "uk"]);
        let url = |id: &str| debian.iter().find(|m| m.id == id).unwrap().url;
        assert_eq!(url("us"), "http://ftp.us.debian.org/");
        assert_eq!(url("jp"), "http://ftp.jaist.ac.jp/");
        assert_eq!(url("sg"), "http://mirror.djvg.sg/");
        assert_eq!(url("uk"), "http://ftp.uk.debian.org/");
        // 国内镜像两种发行版通用
        assert!(debian.iter().any(|m| m.id == "aliyun"));

        let ubuntu = apt_mirrors("ubuntu");
        let ubuntu_url = |id: &str| ubuntu.iter().find(|m| m.id == id).unwrap().url;
        assert_eq!(ubuntu_url("official"), "http://archive.ubuntu.com/");
        assert_eq!(ubuntu_url("uk"), "http://gb.archive.ubuntu.com/");
        // 拼接后必须落在正确的发行版目录下
        assert_eq!(
            apt_base(ubuntu_url("jp"), "ubuntu"),
            "http://jp.archive.ubuntu.com/ubuntu/"
        );
        assert_eq!(
            apt_base(url("us"), "debian"),
            "http://ftp.us.debian.org/debian/"
        );
    }

    #[test]
    fn apt_base_from_sources_list() {
        let content = "\
# comment
deb https://mirrors.tuna.tsinghua.edu.cn/debian/ bookworm main contrib
deb https://mirrors.tuna.tsinghua.edu.cn/debian/ bookworm-updates main
";
        assert_eq!(
            apt_base_from_content(content).as_deref(),
            Some("https://mirrors.tuna.tsinghua.edu.cn/debian/")
        );
        assert_eq!(apt_base_from_content("# 只有注释\n"), None);
        assert_eq!(
            apt_base_from_content("deb [arch=amd64] https://a.b/c/ jammy main\n").as_deref(),
            Some("https://a.b/c/")
        );
    }

    #[test]
    fn epel_rewrite_uses_everything_layout() {
        let content = "\
[epel]
name=Extra Packages for Enterprise Linux 8 - x86_64
baseurl=https://dl.fedoraproject.org/pub/epel/8/Everything/x86_64/
enabled=1
gpgcheck=1
";
        let out = rewrite_epel_baseurls(content, "https://mirrors.aliyun.com/");
        assert!(
            out.contains(
                "baseurl=https://mirrors.aliyun.com/epel/$releasever/Everything/$basearch/"
            )
        );
        // 非 baseurl 行原样保留
        assert!(out.contains("gpgcheck=1"));
    }

    #[test]
    fn epel_file_detection() {
        assert!(is_epel_repo_file("/etc/yum.repos.d/epel.repo"));
        assert!(is_epel_repo_file("/etc/yum.repos.d/epel-aliyun.repo"));
        assert!(!is_epel_repo_file("/etc/yum.repos.d/Rocky-BaseOS.repo"));
        assert!(is_distro_repo_file(
            "/etc/yum.repos.d/Rocky-BaseOS.repo",
            "rocky"
        ));
        assert!(!is_distro_repo_file(
            "/etc/yum.repos.d/docker-ce.repo",
            "rocky"
        ));
    }
}
