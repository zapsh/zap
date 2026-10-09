// SPDX-License-Identifier: AGPL-3.0-only
//! 服务器运行环境探测（root 执行，只读）。
//!
//! 探测 OS / 主机名 / Web 服务器（nginx|openresty）/ PHP（含 FPM socket）/
//! 数据库实例 / 常用工具链，供面板「运行环境」页展示与全局默认配置使用。
//! 单项探测失败以空串 / 空数组表示，不影响整体成功返回。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zap_proto::Response;

use super::root_cmd;
use super::site;

pub async fn detect() -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> { Ok(detect_inner()) })
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

/// fnm 安装目录：固定放全局，装一次所有用户都能用（各自只需挑版本）。
pub const FNM_DIR: &str = "/usr/local/fnm";

/// uv 解释器的共享安装目录：root 统一装到这里，所有用户（站点应用）都能用。
///
/// 不设 `UV_PYTHON_INSTALL_DIR` 时 uv 默认装进执行者家目录（~/.local/share/uv/python），
/// root 装的就只有 root 能用：面板显示「安装成功」，zap / 站点用户的 `uv python list`
/// 里却根本没有，建 .venv 时还得各下一份。
pub const UV_PYTHON_INSTALL_DIR: &str = "/usr/local/share/uv/python";

/// 一键装 uv 的命令：pip（走已配 PyPI 源）优先，其次官方脚本。
const INSTALL_UV_CMD: &str = "if command -v pip3 >/dev/null 2>&1; then \
     pip3 install --break-system-packages uv 2>/dev/null || pip3 install uv; \
   elif python3 -m pip --version >/dev/null 2>&1; then \
     python3 -m pip install --break-system-packages uv 2>/dev/null \
       || python3 -m pip install uv; \
   fi; \
   command -v uv >/dev/null 2>&1 \
     || curl -LsSf https://astral.sh/uv/install.sh | UV_INSTALL_DIR=/usr/local/bin sh";

/// Node 版本下载镜像：nodejs.org 直连慢时走 npmmirror。
const NODE_DIST_MIRROR: &str = "https://npmmirror.com/mirrors/node/";

/// GitHub 下载加速前缀（fnm 二进制在 GitHub Releases）。
const GH_MIRROR_PREFIX: &str = "https://ghfast.top/";

/// npm 全局配置：/etc/npmrc 对所有用户生效。
fn write_npm_registry(url: &str) -> Result<(), String> {
    std::fs::write("/etc/npmrc", format!("registry={url}\n"))
        .map_err(|e| format!("写 /etc/npmrc 失败：{e}"))
}

/// 当前 npm registry（没配过就是官方源）。
fn current_npm_registry() -> String {
    if let Ok(txt) = std::fs::read_to_string("/etc/npmrc") {
        for line in txt.lines() {
            if let Some(v) = line.trim().strip_prefix("registry=") {
                let v = v.trim().to_string();
                if !v.is_empty() {
                    return v;
                }
            }
        }
    }
    "https://registry.npmjs.org/".to_string()
}

/// Node.js 运行时管理入口（fnm）：detect / install_fnm / install / default / uninstall。
pub async fn nodejs(action: &str, version: &str, mirror: &str) -> Response {
    let action = action.to_string();
    let version = version.to_string();
    let mirror = mirror.to_string();
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        Ok(nodejs_inner(&action, &version, &mirror))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

fn nodejs_inner(action: &str, version: &str, mirror: &str) -> Response {
    let fresh = || json!({ "nodejs": detect_nodejs() });
    match action {
        "detect" => Response::ok("Node.js 运行时探测完成", Some(fresh())),
        // 一键安装 fnm 到全局目录（其他用户只管用，不用自己装管理器）。
        // 国内镜像模式下直接拉 GitHub Release 的 zip：
        // 官方脚本会去连 GitHub API，国内环境经常卡在那里超时。
        "install_fnm" => {
            let cmd = if mirror == "china" {
                format!(
                    "mkdir -p {FNM_DIR} && \
                     curl -fsSL {GH_MIRROR_PREFIX}https://github.com/Schniz/fnm/releases/latest/download/fnm-linux.zip \
                       -o /tmp/fnm.zip && \
                     (unzip -o -q /tmp/fnm.zip -d {FNM_DIR} 2>/dev/null || \
                      python3 -c \"import zipfile; zipfile.ZipFile('/tmp/fnm.zip').extractall('{FNM_DIR}')\") && \
                     chmod +x {FNM_DIR}/fnm && rm -f /tmp/fnm.zip"
                )
            } else {
                "curl -fsSL https://fnm.vercel.app/install | bash -s -- --install-dir /usr/local/fnm --skip-shell"
                    .to_string()
            };
            run_shell(&cmd, "fnm 安装完成", Some(fresh()))
        }
        // npm 源：写 /etc/npmrc，所有用户 npm install 都走它
        "set_registry" => {
            let url = version.trim();
            if url.is_empty() || !url.starts_with("http") {
                return Response::err(-1, "registry 地址不合法（需以 http 开头）".to_string());
            }
            match write_npm_registry(url) {
                Ok(()) => Response::ok(format!("npm 源已切换：{url}"), Some(fresh())),
                Err(e) => Response::err(-1, format!("写 npm 源配置失败：{e}")),
            }
        }
        "install" | "uninstall" | "default" => {
            let v = version.trim();
            if v.is_empty() {
                return Response::err(-1, "请指定 Node 版本（如 20）".to_string());
            }
            if !is_safe_version(v) {
                return Response::err(-1, format!("版本号不合法：{v}"));
            }
            if !fnm_bin().is_some() {
                return Response::err(-1, "未找到 fnm：请先一键安装 fnm".to_string());
            }
            // 装完/切换后把 node、npm、npx 软链到 /usr/local/bin，
            // 这样其它用户不用配 fnm 也能直接 node/npm（他们只需选版本号）。
            let link = r#"BIN=$(ls -d $FNM_DIR/node-versions/v{VER}*/installation/bin 2>/dev/null | tail -1); \
                 if [ -n "$BIN" ]; then for b in node npm npx; do \
                   [ -x "$BIN/$b" ] && ln -sf "$BIN/$b" /usr/local/bin/$b; done; fi"#;
            let cmd = match action {
                "install" => format!(
                    "fnm install {v} && if [ ! -e /usr/local/bin/node ]; then {link}; fi",
                    link = link.replace("{VER}", v)
                ),
                "default" => format!("fnm default {v} && {}", link.replace("{VER}", v)),
                _ => format!(
                    "fnm uninstall {v} && if [ ! -e /usr/local/bin/node ]; then \
                     rm -f /usr/local/bin/node /usr/local/bin/npm /usr/local/bin/npx; fi"
                ),
            };
            let script = fnm_script(&cmd, mirror);
            match root_cmd("bash").arg("-lc").arg(&script).output() {
                Ok(o) => {
                    let tail = String::from_utf8_lossy(&o.stdout).to_string()
                        + &String::from_utf8_lossy(&o.stderr);
                    if o.status.success() {
                        Response::ok(
                            format!(
                                "{} {v} 完成",
                                match action {
                                    "install" => "安装",
                                    "default" => "设为默认",
                                    _ => "卸载",
                                }
                            ),
                            Some(fresh()),
                        )
                    } else {
                        Response::err(-1, format!("{cmd} 失败：{}", tail.trim()))
                    }
                }
                Err(e) => Response::err(-1, format!("执行 {cmd} 失败：{e}")),
            }
        }
        other => Response::err(-1, format!("不支持的 nodejs 操作：{other}")),
    }
}

/// 拼一段带 fnm 环境的脚本：fnm 装在全局目录，普通用户只要 source 一下就能用。
/// 国内镜像模式下额外指定 Node 发行版镜像（nodejs.org 直连很慢）。
fn fnm_script(cmd: &str, mirror: &str) -> String {
    let dist = if mirror == "china" {
        format!("export FNM_NODE_DIST_MIRROR={NODE_DIST_MIRROR}; ")
    } else {
        String::new()
    };
    format!(
        "export FNM_DIR={FNM_DIR}; {dist} \\
         export PATH=$FNM_DIR:$PATH; \\
         if [ -f $FNM_DIR/fnm ]; then . <($FNM_DIR/fnm env --use-on-cd 2>/dev/null) 2>/dev/null || true; fi; \\
         {cmd}"
    )
}

fn fnm_bin() -> Option<String> {
    let c = Path::new(FNM_DIR).join("fnm");
    if c.is_file() {
        return Some(c.to_string_lossy().to_string());
    }
    which("fnm")
}

/// 探测 Node.js 运行时：fnm 状态 + 已装版本（fnm 管理的 + 系统自带的）。
pub fn detect_nodejs() -> Value {
    let fnm_path = fnm_bin();
    let fnm_version = match &fnm_path {
        Some(p) => probe_first_line(p, &["--version"]).unwrap_or_default(),
        None => String::new(),
    };
    let mut versions: Vec<Value> = Vec::new();
    let default = default_node_version();
    let root = Path::new(FNM_DIR).join("node-versions");

    // fnm 装的版本：/usr/local/fnm/node-versions/v20.11.1/installation/bin/node
    if let Ok(rd) = std::fs::read_dir(&root) {
        let mut dirs: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.strip_prefix('v').map(|s| s.to_string())
            })
            .collect();
        dirs.sort();
        for v in dirs {
            let bin = root.join(format!("v{v}")).join("installation/bin/node");
            versions.push(json!({
                "version": v,
                "path": bin.to_string_lossy(),
                "source": "fnm",
                "installed": bin.is_file(),
            }));
        }
    }

    // 系统自带（非 fnm 管理）
    if let Some(line) = probe_first_line("node", &["--version"]) {
        let v = line.trim_start_matches('v').to_string();
        if !v.is_empty()
            && !versions
                .iter()
                .any(|x| x.get("version").and_then(|y| y.as_str()) == Some(v.as_str()))
        {
            versions.push(json!({
                "version": v,
                "path": which("node").unwrap_or_default(),
                "source": "system",
                "installed": true,
            }));
        }
    }
    json!({
        "fnm": fnm_path.is_some(),
        "fnm_path": fnm_path.unwrap_or_default(),
        "fnm_version": fnm_version,
        "fnm_dir": FNM_DIR,
        "default": default,
        "global_link": Path::new("/usr/local/bin/node").exists(),
        "npm_registry": current_npm_registry(),
        "versions": versions,
    })
}

/// 全局默认 Node 版本：看 /usr/local/bin/node 软链指向哪个 installation。
fn default_node_version() -> String {
    let link = Path::new("/usr/local/bin/node");
    if let Ok(target) = std::fs::read_link(link) {
        let s = target.to_string_lossy().to_string();
        // .../node-versions/v20.11.1/installation/bin/node
        for part in s.split('/') {
            if let Some(v) = part.strip_prefix('v')
                && v.chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    return v.to_string();
                }
        }
    }
    String::new()
}

/// 指定版本对应的 node 可执行文件（fnm 目录里找），供部署时拼命令用。
///
/// 版本可写 `20` 或 `20.11.1`：先精确匹配，再按主版本前缀匹配最新的一个。
pub fn node_bin_for(version: &str) -> Option<PathBuf> {
    if version.is_empty() {
        return None;
    }
    let root = Path::new(FNM_DIR).join("node-versions");
    let rd = std::fs::read_dir(&root).ok()?;
    let mut cands: Vec<(String, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let v = n.strip_prefix('v')?.to_string();
        if v == version || v.starts_with(&format!("{version}.")) {
            let bin = root.join(&n).join("installation/bin/node");
            if bin.is_file() {
                cands.push((v, bin));
            }
        }
    }
    cands.sort_by(|a, b| b.0.cmp(&a.0));
    cands.into_iter().next().map(|(_, p)| p)
}

/// Python 运行时管理入口：detect / install / uninstall。
///
/// 全局由 uv 统一管理解释器版本，应用目录里再各建自己的 `.venv`（local），
/// 这样不同应用想要不同 Python 版本时互不影响。
pub async fn python(action: &str, version: &str, extra: &str) -> Response {
    let action = action.to_string();
    let version = version.to_string();
    let extra = extra.to_string();
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        Ok(python_inner(&action, &version, &extra))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

fn python_inner(action: &str, version: &str, extra: &str) -> Response {
    match action {
        "detect" => Response::ok(
            "Python 运行时探测完成",
            Some(json!({ "python": detect_python() })),
        ),
        // 一键安装 uv。优先用 pip 装：它读 /etc/pip.conf，
        // 也就是说选了清华/阿里源之后，装 uv 本身就走国内源，快得多；
        // 机器上没有 pip 时才回退官方安装脚本。
        "install_uv" => run_shell(
            INSTALL_UV_CMD,
            "uv 安装完成",
            Some(json!({ "python": detect_python() })),
        ),
        // 切换 PyPI 源：系统级写 uv 配置 + pip 配置，所有用户与应用都生效
        "set_index" => {
            let url = extra.trim();
            if url.is_empty() || !url.starts_with("http") {
                return Response::err(-1, "源地址不合法（需以 http 开头）".to_string());
            }
            match write_python_index(url) {
                Ok(()) => Response::ok(
                    format!("已切换 PyPI 源：{url}"),
                    Some(json!({ "python": detect_python() })),
                ),
                Err(e) => Response::err(-1, format!("写源配置失败：{e}")),
            }
        }
        "install" | "uninstall" => {
            let v = version.trim();
            if v.is_empty() {
                return Response::err(-1, "请指定 Python 版本（如 3.12）".to_string());
            }
            if !is_safe_version(v) {
                return Response::err(-1, format!("版本号不合法：{v}"));
            }
            let Some(uv) = uv_bin() else {
                return Response::err(
                    -1,
                    "未找到 uv：请先安装（curl -LsSf https://astral.sh/uv/install.sh | sh），\
                     或改用系统自带的 python3"
                        .to_string(),
                );
            };
            let cmd = if action == "install" {
                format!(
                    "{} python install {v} && chmod -R a+rX {} && {}",
                    uv,
                    UV_PYTHON_INSTALL_DIR,
                    write_uv_env_cmd()
                )
            } else {
                format!("{} python uninstall {v}", uv)
            };
            let out = root_cmd("bash")
                .arg("-lc")
                .arg(&cmd)
                .env("UV_PYTHON_INSTALL_DIR", UV_PYTHON_INSTALL_DIR)
                .output()
                .map_err(|e| Response::err(-1, format!("执行 {cmd} 失败：{e}")));
            match out {
                Ok(o) => {
                    let tail = String::from_utf8_lossy(&o.stdout).to_string()
                        + &String::from_utf8_lossy(&o.stderr);
                    if o.status.success() {
                        Response::ok(
                            format!(
                                "{} {v} 完成",
                                if action == "install" {
                                    "安装"
                                } else {
                                    "卸载"
                                }
                            ),
                            Some(json!({ "python": detect_python(), "log": tail })),
                        )
                    } else {
                        Response::err(-1, format!("{cmd} 失败：{}", tail.trim()))
                    }
                }
                Err(r) => r,
            }
        }
        other => Response::err(-1, format!("不支持的 python 操作：{other}")),
    }
}

/// 版本号允许 `3` / `3.11` / `3.11.9`，也允许预发布形式 `3.15.0rc2`（含字母后缀），
/// 还要拼进 shell 命令，必须严格（仅字母数字、点、加号）。
fn is_safe_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '+')
}

/// 生成「把共享解释器目录写进全局登录环境」的 shell 片段：
/// 落一份 /etc/profile.d/zap-uv-python.sh，所有交互式登录 shell 的 uv
/// 都能看到 root 统一安装的版本（面板 / run_as 侧走 .env 注入，不依赖它）。
fn write_uv_env_cmd() -> String {
    format!(
        "printf 'export UV_PYTHON_INSTALL_DIR={UV_PYTHON_INSTALL_DIR}\\n' > /etc/profile.d/zap-uv-python.sh"
    )
}

/// 确保共享目录里有指定小版本的 Python（站点用户建 .venv 前调用）。
///
/// 共享目录归 root、站点用户只读：缺版本时必须先由 root（这里）装好，
/// 否则站点用户的 uv 会试图往共享目录下载，因无写权限而失败。
/// 版本匹配用前缀：`3.15` 命中 `cpython-3.15.0rc2-…` / `cpython-3.15.0-…` 都算有。
pub(crate) fn ensure_python_shared(version: &str) -> Result<(), String> {
    if !is_safe_version(version) {
        return Err(format!("版本号不合法：{version}"));
    }
    let prefix = format!("cpython-{version}");
    let have = std::fs::read_dir(UV_PYTHON_INSTALL_DIR)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        })
        .unwrap_or(false);
    if have {
        return Ok(());
    }
    let Some(uv) = uv_bin() else {
        return Err("未找到 uv，无法补装 Python 解释器".to_string());
    };
    let cmd = format!(
        "{uv} python install {version} && chmod -R a+rX {UV_PYTHON_INSTALL_DIR} && {}",
        write_uv_env_cmd()
    );
    let out = root_cmd("bash")
        .arg("-lc")
        .arg(&cmd)
        .env("UV_PYTHON_INSTALL_DIR", UV_PYTHON_INSTALL_DIR)
        .output()
        .map_err(|e| format!("执行 {cmd} 失败：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let tail = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        Err(format!("{cmd} 失败：{}", tail.trim()))
    }
}

/// PyPI 系统级配置：uv 读 /etc/uv/uv.toml，pip 读 /etc/pip.conf。
/// 两处都写，uv pip 与传统 pip 就走同一个源。
fn write_python_index(url: &str) -> Result<(), String> {
    let uv_dir = Path::new("/etc/uv");
    std::fs::create_dir_all(uv_dir).map_err(|e| format!("创建 {uv_dir:?} 失败：{e}"))?;
    let body = format!("[[index]]\nurl = \"{url}\"\ndefault = true\n");
    std::fs::write(uv_dir.join("uv.toml"), body)
        .map_err(|e| format!("写 /etc/uv/uv.toml 失败：{e}"))?;
    let _ = std::fs::write("/etc/pip.conf", format!("[global]\nindex-url = {url}\n"));
    Ok(())
}

/// 当前生效的 PyPI 源（读 /etc/uv/uv.toml；没配过就是官方源）。
fn current_python_index() -> String {
    if let Ok(txt) = std::fs::read_to_string("/etc/uv/uv.toml") {
        for line in txt.lines() {
            if let Some(rest) = line.trim().strip_prefix("url")
                && let Some(v) = rest.split('=').nth(1) {
                    let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                    if !v.is_empty() {
                        return v;
                    }
                }
        }
    }
    "https://pypi.org/simple".to_string()
}

/// 跑一条 shell 命令（root），成功返回 ok + 可选数据。
fn run_shell(cmd: &str, ok_msg: &str, data: Option<Value>) -> Response {
    match root_cmd("bash").arg("-lc").arg(cmd).output() {
        Ok(o) => {
            let tail = String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr);
            if o.status.success() {
                Response::ok(ok_msg.to_string(), data)
            } else {
                Response::err(-1, format!("{cmd} 失败：{}", tail.trim()))
            }
        }
        Err(e) => Response::err(-1, format!("执行 {cmd} 失败：{e}")),
    }
}

/// uv 可执行文件位置。uv 常装在 root 家目录，PATH 里未必有，故按常见路径兜底找。
pub fn uv_bin() -> Option<String> {
    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/root/.local/bin"),
        PathBuf::from("/root/.cargo/bin"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    for d in dirs {
        let c = d.join("uv");
        if c.is_file() {
            return Some(c.to_string_lossy().to_string());
        }
    }
    which("uv")
}

fn which(name: &str) -> Option<String> {
    let o = root_cmd("bash")
        .arg("-lc")
        .arg(format!("command -v {name}"))
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// 探测 Python 运行时：uv 是否可用 + 可用版本（uv 管理的 / 系统自带的）。
pub fn detect_python() -> Value {
    let uv_path = uv_bin();
    let uv_version = match &uv_path {
        Some(p) => probe_first_line(p, &["--version"]).unwrap_or_default(),
        None => String::new(),
    };
    let mut versions: Vec<Value> = Vec::new();
    // uv 管理的版本：`uv python list` 每行形如
    //   cpython-3.12.4-linux-x86_64-gnu    /usr/local/share/uv/python/.../bin/python3.12
    if let Some(uv) = &uv_path {
        // 只要已安装的；老版本 uv 不认 --only-installed（输出为空）时退回完整列表，
        // 再靠 uv_list_installed 的路径判断把未安装项剔掉。
        // 带上 UV_PYTHON_INSTALL_DIR：装在共享目录里的解释器才能被列出来
        //（不设的话 uv 只看执行者家目录，root 探测就漏掉全局版本）。
        let mut txt = String::new();
        if let Ok(o) = root_cmd(uv)
            .args(["python", "list", "--only-installed"])
            .env("UV_PYTHON_INSTALL_DIR", UV_PYTHON_INSTALL_DIR)
            .output()
        {
            txt = String::from_utf8_lossy(&o.stdout).to_string();
        }
        if txt.trim().is_empty()
            && let Ok(o) = root_cmd(uv)
                .args(["python", "list"])
                .env("UV_PYTHON_INSTALL_DIR", UV_PYTHON_INSTALL_DIR)
                .output()
            {
                txt = String::from_utf8_lossy(&o.stdout).to_string();
            }
        for line in txt.lines() {
            if let Some((ver, path)) = uv_list_installed(line) {
                versions.push(json!({
                    "version": ver,
                    "path": path,
                    "source": "uv",
                }));
            }
        }
    }
    // 系统自带：扫描 /usr/bin/python3.*
    for cand in system_pythons() {
        let short = short_version(&cand);
        if versions
            .iter()
            .any(|v| v.get("version").and_then(|x| x.as_str()) == Some(short.as_str()))
        {
            continue;
        }
        versions.push(json!({
            "version": short,
            "path": format!("/usr/bin/python{short}"),
            "source": "system",
        }));
    }
    versions.sort_by(|a, b| {
        b.get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(a.get("version").and_then(|v| v.as_str()).unwrap_or(""))
    });
    json!({
        "uv": uv_path.is_some(),
        "uv_path": uv_path.unwrap_or_default(),
        "uv_version": uv_version,
        "index_url": current_python_index(),
        "versions": versions,
    })
}

/// 从 `cpython-3.12.4-linux-x86_64-gnu` 里取出 `3.12.4`。
/// 解析 `uv python list` 的一行，只接受**已安装**的解释器。
///
/// 未安装的行第二列是 `<download available>` 之类而不是路径，形如：
///   cpython-3.13.0-linux-x86_64-gnu    <download available>
/// 以前只看第一列版本就收，导致部署向导里能选到根本没装的版本（一部署就失败）。
fn uv_list_installed(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if t.is_empty() || t.starts_with("Installed") || t.starts_with("Available") {
        return None;
    }
    let ver = uv_list_version(t)?;
    let path = t.split_whitespace().nth(1).unwrap_or("").to_string();
    if !path.starts_with('/') {
        return None;
    }
    Some((ver, path))
}

#[cfg(test)]
mod uv_list_tests {
    use super::*;

    /// 已安装的行：第二列是解释器绝对路径
    #[test]
    fn installed_line_is_kept() {
        let line = "cpython-3.12.4-linux-x86_64-gnu    /root/.local/share/uv/python/cpython-3.12.4/bin/python3.12";
        let (ver, path) = uv_list_installed(line).expect("已安装行应保留");
        assert_eq!(ver, "3.12.4");
        assert!(path.starts_with('/'), "{path}");
    }

    /// 未安装的行（可下载）：不能出现在版本下拉里
    #[test]
    fn downloadable_line_is_dropped() {
        for line in [
            "cpython-3.13.0-linux-x86_64-gnu    <download available>",
            "cpython-3.14.0-linux-x86_64-gnu",
        ] {
            assert!(uv_list_installed(line).is_none(), "{line} 不应算已安装");
        }
    }

    /// 分组标题与空行
    #[test]
    fn headers_and_blank_are_dropped() {
        for line in ["", "   ", "Installed versions:", "Available for download:"] {
            assert!(uv_list_installed(line).is_none(), "{line}");
        }
    }

    /// 预发布版本（带字母后缀）不能被当非法行丢掉：
    /// `uv python install 3.15` 装到的可能是 `cpython-3.15.0rc2`。
    #[test]
    fn prerelease_line_is_kept() {
        let line = "cpython-3.15.0rc2-linux-x86_64-gnu    /usr/local/share/uv/python/cpython-3.15.0rc2-linux-x86_64-gnu/bin/python3.15";
        let (ver, path) = uv_list_installed(line).expect("预发布已安装行应保留");
        assert_eq!(ver, "3.15.0rc2");
        assert!(path.starts_with('/'), "{path}");
        assert!(is_safe_version("3.15.0rc2"), "rc 版本号应可通过校验");
    }
}

fn uv_list_version(line: &str) -> Option<String> {
    let tok = line.split_whitespace().next()?;
    let ver = tok.split('-').nth(1)?;
    // 预发布版本带字母后缀（如 `cpython-3.15.0rc2-linux-…` 的 `3.15.0rc2`），
    // 只认数字会把它当非法行丢掉，装完的版本在面板里就「消失」了。
    if ver
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '+')
    {
        Some(ver.to_string())
    } else {
        None
    }
}

fn system_pythons() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/usr/bin") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(rest) = n.strip_prefix("python3.")
                && rest.chars().all(|c| c.is_ascii_digit()) {
                    out.push(format!("3.{rest}"));
                }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn detect_inner() -> Response {
    let (os_id, os_name, os_ver) = os_release();
    let data = json!({
        "os": {
            "id": os_id,
            "name": os_name,
            "version": os_ver,
            "arch": std::env::consts::ARCH,
            "kernel": kernel_version(),
        },
        "hostname": hostname_detect(),
        "webserver": detect_webserver(),
        "php": detect_php(),
        "python": detect_python(),
        "nodejs": detect_nodejs(),
        "databases": detect_databases(),
        "tools": detect_tools(),
        "network": detect_network(),
    });
    Response::ok("服务器运行环境探测完成", Some(data))
}

// ── 通用小工具 ────────────────────────────────────────────────

/// 运行命令并返回首个非空行（优先 stdout，其次 stderr），失败返回 None。
fn probe_first_line(program: &str, args: &[&str]) -> Option<String> {
    let o = root_cmd(program).args(args).output().ok()?;
    let text = if !o.stdout.is_empty() {
        String::from_utf8_lossy(&o.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&o.stderr).into_owned()
    };
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(180).collect())
}

/// 进程是否在运行（精确匹配进程名）。
fn proc_running(name: &str) -> bool {
    root_cmd("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 在 marker 之后的第一个空白分隔 token，如失败返回 None。
fn token_after(line: &str, marker: &str) -> Option<String> {
    let rest = line.split_once(marker)?.1;
    let tok = rest
        .split_whitespace()
        .next()?
        .trim_matches(['(', ')', ';', ',']);
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

// ── OS / 主机 ────────────────────────────────────────────────

fn os_release() -> (String, String, String) {
    let mut id = "linux".to_string();
    let mut name = "Linux".to_string();
    let mut ver = String::new();
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches('"').to_string();
            match k {
                "ID" => id = v,
                "NAME" => name = v,
                "VERSION_ID" => ver = v,
                _ => {}
            }
        }
    }
    (id, name, ver)
}

fn kernel_version() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| probe_first_line("uname", &["-r"]))
        .unwrap_or_default()
}

fn hostname_detect() -> String {
    let from_file = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty());
    from_file
        .or_else(|| probe_first_line("hostname", &[]))
        .unwrap_or_default()
}

// ── Web 服务器（nginx / openresty）────────────────────────────

fn detect_webserver() -> Value {
    if let Some(conf) = site::find_nginx_conf_file() {
        let conf_s = conf.to_string_lossy().into_owned();
        let bin = site::nginx_bin(&conf);
        let bin_s = bin.to_string_lossy().into_owned();
        let raw = probe_first_line(&bin_s, &["-v"]).unwrap_or_default();
        let flavor = if raw.contains("openresty")
            || conf_s.contains("openresty")
            || bin_s.contains("openresty")
        {
            "openresty"
        } else {
            "nginx"
        };
        let sites_dir = site::vhosts_dir(&conf)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        return json!({
            "flavor": flavor,
            "version": nginx_version(&raw),
            "binary": bin_s,
            "conf": conf_s,
            "sites_dir": sites_dir,
            "running": site::nginx_running(),
        });
    }
    // 系统自装 nginx（apt/yum），未纳入 data/apps
    if let Some(raw) = probe_first_line("nginx", &["-v"]) {
        let conf = if Path::new("/etc/nginx/nginx.conf").is_file() {
            "/etc/nginx/nginx.conf"
        } else {
            ""
        };
        let sites_dir = if Path::new("/etc/nginx/sites-enabled").is_dir() {
            "/etc/nginx/sites-enabled"
        } else if Path::new("/etc/nginx/conf.d").is_dir() {
            "/etc/nginx/conf.d"
        } else {
            ""
        };
        return json!({
            "flavor": "nginx",
            "version": nginx_version(&raw),
            "binary": "nginx",
            "conf": conf,
            "sites_dir": sites_dir,
            "running": site::nginx_running(),
        });
    }
    json!({ "flavor": "none", "version": "", "binary": "", "conf": "", "sites_dir": "", "running": false })
}

/// 从 `nginx version: nginx/1.24.0` / `nginx version: openresty/1.25.3.2` 中取版本号。
fn nginx_version(raw: &str) -> String {
    let Some(idx) = raw.rfind('/') else {
        return String::new();
    };
    raw[idx + 1..]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .trim_end_matches(';')
        .to_string()
}

// ── PHP ──────────────────────────────────────────────────────

fn detect_php() -> Value {
    // key: 短版本号（如 8.3）
    let mut instances: BTreeMap<String, Value> = BTreeMap::new();

    // 1) AppStore 安装：/usr/local/apps/php*
    if let Ok(rd) = std::fs::read_dir("/usr/local/apps") {
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .map(|n| n.to_string_lossy().starts_with("php"))
                        .unwrap_or(false)
            })
            .collect();
        dirs.sort();
        for d in dirs {
            let bin = d.join("bin/php");
            if bin.is_file() {
                push_php_bin(&mut instances, &bin);
            }
        }
    }

    // 2) 系统 PHP：/usr/bin/phpX*、/usr/local/bin/phpX*
    for base in [PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")] {
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                    continue;
                };
                let rest = name.strip_prefix("php").unwrap_or("");
                if rest.is_empty() || !rest.chars().next().unwrap().is_ascii_digit() {
                    continue;
                }
                push_php_bin(&mut instances, &p);
            }
        }
    }

    // 3) FPM socket：为实例补 socket / running；未匹配 socket 生成独立条目
    for sock in fpm_sockets() {
        let running = sock.exists();
        let Some(raw_tok) = socket_version_token(&sock) else {
            continue;
        };
        let v2 = normalize_version_token(&raw_tok);
        if v2.is_empty() {
            continue;
        }
        if let Some(ins) = instances.get_mut(&v2) {
            if running {
                if ins["socket"].as_str().unwrap_or("").is_empty() {
                    ins["socket"] = json!(sock.to_string_lossy());
                }
                ins["running"] = json!(true);
            }
            continue;
        }
        let entry = instances.entry(v2.clone()).or_insert_with(
            || json!({ "version": v2.clone(), "binary": "", "socket": "", "running": false }),
        );
        if running {
            entry["socket"] = json!(sock.to_string_lossy());
            entry["running"] = json!(true);
        }
    }

    // 4) 默认 PHP（PATH 上的 php；无则取最高版本）
    let mut default_v2 = String::new();
    if let Some(line) = probe_first_line("php", &["-v"])
        && let Some(ver) = php_version(&line)
    {
        default_v2 = short_version(&ver);
    }
    if default_v2.is_empty() {
        default_v2 = instances.keys().next_back().cloned().unwrap_or_default();
    }

    let mut list: Vec<Value> = Vec::with_capacity(instances.len());
    for (v2, mut ins) in instances {
        ins["default"] = json!(v2 == default_v2);
        list.push(ins);
    }

    json!({ "default": default_v2, "instances": list })
}

fn push_php_bin(map: &mut BTreeMap<String, Value>, bin: &Path) {
    let bstr = bin.to_string_lossy().into_owned();
    let Some(line) = probe_first_line(&bstr, &["-v"]) else {
        return;
    };
    let Some(ver) = php_version(&line) else {
        return;
    };
    let v2 = short_version(&ver);
    if map.contains_key(&v2) {
        return;
    }
    map.insert(
        v2.clone(),
        json!({
            "version": ver,
            "binary": bstr,
            "socket": "",
            "running": false,
            "default": false,
        }),
    );
}

/// 从 `PHP 8.3.7 (cli)` 第一行中提取完整版本号（8.3.7）。
fn php_version(line: &str) -> Option<String> {
    let tok = line.trim().strip_prefix("PHP")?.split_whitespace().next()?;
    if tok.starts_with(|c: char| c.is_ascii_digit()) {
        Some(tok.to_string())
    } else {
        None
    }
}

fn short_version(ver: &str) -> String {
    ver.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// 常见 FPM unix socket 位置扫描（系统 pool 与单实例均可）。
fn fpm_sockets() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in [
        PathBuf::from("/var/run"),
        PathBuf::from("/run"),
        PathBuf::from("/var/run/php-fpm"),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if name.starts_with("php-fpm") && name.ends_with(".sock") {
                out.push(p);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 从 socket 文件名中取版本 token：`php-fpm-8.3.sock` → `8.3`；`php-fpm83.sock` → `83`。
fn socket_version_token(sock: &Path) -> Option<String> {
    let name = sock.file_name()?.to_string_lossy();
    let name = name.strip_suffix(".sock")?;
    let tok = name.strip_prefix("php-fpm")?.trim_start_matches(['-', '_']);
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

/// 版本 token 规范化：`83` → `8.3`；含点的原样保留。
fn normalize_version_token(t: &str) -> String {
    let t = t.trim().trim_matches(['-', '_', '.']);
    if t.is_empty() {
        return String::new();
    }
    if t.contains('.') {
        return t.to_string();
    }
    let is_digit2 = t.len() == 2 && t.bytes().all(|b| b.is_ascii_digit());
    if is_digit2 {
        return format!("{}.{}", &t[..1], &t[1..]);
    }
    t.to_string()
}

// ── 数据库 ───────────────────────────────────────────────────

fn detect_databases() -> Value {
    let mut out = Vec::new();
    for (name, bins, prefixes) in [
        ("mysql", &["mysqld"][..], &["mysql"][..]),
        ("mariadb", &["mariadbd", "mysqld"][..], &["mariadb"][..]),
        ("postgresql", &["postgres"][..], &["postgres", "pgsql"][..]),
        ("redis", &["redis-server"][..], &["redis"][..]),
        ("mongodb", &["mongod"][..], &["mongo"][..]),
    ] {
        if let Some((binary, line)) = probe_database(bins, prefixes) {
            let proc = Path::new(&binary)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| bins[0].to_string());
            out.push(json!({
                "name": name,
                "version": db_version(name, &line),
                "binary": binary,
                "running": proc_running(&proc),
            }));
        }
    }
    json!(out)
}

/// 探测数据库守护进程：安装根（AppStore，含版本目录）优先，其次 PATH。
/// 返回 (二进制路径, 版本首行)。
fn probe_database(bins: &[&str], prefixes: &[&str]) -> Option<(String, String)> {
    for bin in bins {
        for cand in db_bin_candidates(bin, prefixes) {
            let path = cand.to_string_lossy().into_owned();
            if let Some(line) = probe_first_line(&path, &["--version"]) {
                return Some((path, line));
            }
        }
    }
    // 系统包管理器安装（在 PATH 上）
    for bin in bins {
        if let Some(line) = probe_first_line(bin, &["--version"]) {
            return Some((bin.to_string(), line));
        }
    }
    None
}

/// 安装根（默认 /usr/local/apps，`ZAP_APPS_DIR` 可覆盖）下查找守护进程：
/// 兼容 `<root>/mysql-8.4` 与 `<root>/<分类>/mysql-8.4` 两种布局，
/// 多个版本时优先版本号高者。
fn db_bin_candidates(bin: &str, prefixes: &[&str]) -> Vec<PathBuf> {
    let root = super::install_root();
    let matched = |name: &str| prefixes.iter().any(|pre| name.starts_with(pre));

    let Ok(top) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for e in top.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if matched(&e.file_name().to_string_lossy()) {
            dirs.push(p.clone());
        }
        // 分类二级目录（database / server / ...）
        if let Ok(sub) = std::fs::read_dir(&p) {
            for e2 in sub.flatten() {
                let q = e2.path();
                if q.is_dir() && matched(&e2.file_name().to_string_lossy()) {
                    dirs.push(q);
                }
            }
        }
    }

    dirs.sort_by_key(|d| {
        d.file_name()
            .map(|n| dir_version_segments(&n.to_string_lossy()))
            .unwrap_or_default()
    });
    dirs.reverse();
    dirs.dedup();

    let mut out = Vec::new();
    for d in dirs {
        for sub in ["bin", "sbin"] {
            let c = d.join(sub).join(bin);
            if c.is_file() {
                out.push(c);
                break;
            }
        }
    }
    out
}

/// 提取目录名中的数字段（`mysql-8.4` → `[8, 4]`），用于版本排序。
fn dir_version_segments(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

fn db_version(name: &str, line: &str) -> String {
    let raw = match name {
        "mysql" | "mariadb" => token_after(line, "Ver ").unwrap_or_default(),
        "postgresql" => token_after(line, "PostgreSQL) ").unwrap_or_default(),
        "redis" => token_after(line, "v=").unwrap_or_default(),
        "mongodb" => token_after(line, "version v").unwrap_or_default(),
        _ => String::new(),
    };
    // 去掉发行版/插件后缀：8.0.39-0ubuntu... → 8.0.39；11.4.4-MariaDB → 11.4.4
    let head = raw.split('-').next().unwrap_or(&raw).to_string();
    let head = head.trim_end_matches(['(', ')', ';', ',']).to_string();
    if head.is_empty() {
        line.chars().take(80).collect()
    } else {
        head
    }
}

// ── 常用工具链 ───────────────────────────────────────────────

fn detect_tools() -> Value {
    let mut out = Vec::new();
    for (name, args) in [
        ("git", &["--version"][..]),
        ("node", &["--version"][..]),
        ("docker", &["--version"][..]),
        ("python3", &["--version"][..]),
        ("composer", &["--version"][..]),
        ("php", &["--version"][..]),
    ] {
        if let Some(line) = probe_first_line(name, args) {
            out.push(json!({ "name": name, "version": line }));
        }
    }
    json!(out)
}

// ── 网络接口 / IP ────────────────────────────────────────────

/// 需要排除的虚拟/容器接口前缀（面板建站不会绑定这些地址）。
const SKIP_IFACE_PREFIXES: [&str; 8] = [
    "lo", "docker", "br-", "veth", "virbr", "cni", "flannel", "tunl",
];

fn skip_iface(name: &str) -> bool {
    name == "lo" || SKIP_IFACE_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// 解析 `ip -o -4 addr show` 输出，得到「接口 → IPv4 列表」。
///
/// 例：`2: eth0    inet 10.0.0.5/24 brd ...` → `("eth0", "10.0.0.5")`。
fn parse_ipv4_lines(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        // 形如 `2: eth0    inet 10.0.0.5/24 ...`
        let Some(idx) = it.next() else { continue };
        let Some(iface) = it.next() else { continue };
        let iface = iface.trim_end_matches(':').to_string();
        if !idx.ends_with(':') || skip_iface(&iface) {
            continue;
        }
        // 找到 `inet` 后取地址段
        let mut it = line.split_whitespace().skip_while(|t| *t != "inet");
        let _ = it.next();
        let Some(addr) = it.next() else { continue };
        let ip = addr.split('/').next().unwrap_or("").to_string();
        if ip.is_empty() {
            continue;
        }
        out.push((iface, ip));
    }
    out
}

/// 解析 `ip -o -6 addr show` 输出，得到「接口 → IPv6 列表」（去掉 scope link）。
fn parse_ipv6_lines(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.contains(" inet6 ") {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(idx) = it.next() else { continue };
        let Some(iface) = it.next() else { continue };
        let iface = iface.trim_end_matches(':').to_string();
        if !idx.ends_with(':') || skip_iface(&iface) {
            continue;
        }
        let mut it = line.split_whitespace().skip_while(|t| *t != "inet6");
        let _ = it.next();
        let Some(addr) = it.next() else { continue };
        let ip = addr.split('/').next().unwrap_or("").to_string();
        // 链路本地地址（fe80::）不作为对外服务地址候选
        if ip.is_empty() || ip.to_ascii_lowercase().starts_with("fe80") {
            continue;
        }
        out.push((iface, ip));
    }
    out
}

fn run_ip(family: &str) -> String {
    root_cmd("ip")
        .args(["-o", family, "addr", "show"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// 网卡与地址探测：供面板「基础设置 → 默认 IPv4/IPv6/网络设备」下拉选择。
///
/// 结构：`{ interfaces: [{name, mac, state, ipv4[], ipv6[]}], default_ipv4, default_ipv6 }`。
/// `default_*` 取首个 UP 且带地址的网卡的对应地址（`ip route get` 失败时兜底）。
fn detect_network() -> Value {
    use std::collections::BTreeMap;

    let v4 = parse_ipv4_lines(&run_ip("-4"));
    let v6 = parse_ipv6_lines(&run_ip("-6"));

    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    for (iface, ip) in v4.iter() {
        if !order.contains(iface) {
            order.push(iface.clone());
        }
        let e = map.entry(iface.clone()).or_insert_with(|| {
            json!({
                "name": iface,
                "mac": iface_mac(iface),
                "state": iface_state(iface),
                "ipv4": Vec::<String>::new(),
                "ipv6": Vec::<String>::new(),
            })
        });
        if let Some(arr) = e.get_mut("ipv4").and_then(|v| v.as_array_mut())
            && !arr.iter().any(|x| x == ip)
        {
            arr.push(json!(ip));
        }
    }
    for (iface, ip) in v6.iter() {
        let e = map.entry(iface.clone()).or_insert_with(|| {
            json!({
                "name": iface,
                "mac": iface_mac(iface),
                "state": iface_state(iface),
                "ipv4": Vec::<String>::new(),
                "ipv6": Vec::<String>::new(),
            })
        });
        if let Some(arr) = e.get_mut("ipv6").and_then(|v| v.as_array_mut())
            && !arr.iter().any(|x| x == ip)
        {
            arr.push(json!(ip));
        }
    }

    let interfaces: Vec<Value> = map.values().cloned().collect();
    // 默认出口地址：`ip route get 1.1.1.1` 的 src；失败时取首个网卡的首个地址
    let default_ipv4 = default_route_ip("-4").or_else(|| v4.first().map(|(_, ip)| ip.clone()));
    let default_ipv6 = default_route_ip("-6").or_else(|| v6.first().map(|(_, ip)| ip.clone()));

    json!({
        "interfaces": interfaces,
        "ipv4_all": v4.iter().map(|(_, ip)| ip.clone()).collect::<Vec<_>>(),
        "ipv6_all": v6.iter().map(|(_, ip)| ip.clone()).collect::<Vec<_>>(),
        "default_ipv4": default_ipv4.unwrap_or_default(),
        "default_ipv6": default_ipv6.unwrap_or_default(),
    })
}

fn iface_mac(iface: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/address"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn iface_state(iface: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/operstate"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// `ip -o route get` 输出里的 `src <ip>`。
fn default_route_ip(family: &str) -> Option<String> {
    let out = root_cmd("ip")
        .args(["-o", family, "route", "get", "1.1.1.1"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut it = text.split_whitespace().skip_while(|t| *t != "src");
    let _ = it.next();
    it.next().map(|s| s.to_string())
}

// ── 单测（纯函数部分）────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nginx_version_lines() {
        assert_eq!(nginx_version("nginx version: nginx/1.24.0"), "1.24.0");
        assert_eq!(
            nginx_version("nginx version: openresty/1.25.3.2"),
            "1.25.3.2"
        );
        assert_eq!(nginx_version("nginx: [emerg] unknown directive"), "");
    }

    #[test]
    fn parse_php_version_lines() {
        assert_eq!(
            php_version("PHP 8.3.7 (cli) (built: Jun 27 2024)").unwrap(),
            "8.3.7"
        );
        assert_eq!(php_version("PHP 7.4.33").unwrap(), "7.4.33");
        assert!(php_version("Usage: php [options]").is_none());
        assert_eq!(short_version("8.3.7"), "8.3");
        assert_eq!(short_version("7.4"), "7.4");
    }

    #[test]
    fn parse_ip_addr_lines() {
        let v4 = "2: eth0    inet 10.0.0.5/24 brd 10.0.0.255 scope global eth0\\       valid_lft forever preferred_lft forever\n\
                  3: eth1    inet 192.168.1.7/24 scope global eth1\n\
                  1: lo    inet 127.0.0.1/8 scope host lo\n";
        assert_eq!(
            parse_ipv4_lines(v4),
            vec![
                ("eth0".to_string(), "10.0.0.5".to_string()),
                ("eth1".to_string(), "192.168.1.7".to_string())
            ]
        );

        let v6 = "2: eth0    inet6 2408:4005:xxx::1/64 scope global \n\
                  2: eth0    inet6 fe80::1/64 scope link \n\
                  1: lo    inet6 ::1/128 scope host \n";
        assert_eq!(
            parse_ipv6_lines(v6),
            vec![("eth0".to_string(), "2408:4005:xxx::1".to_string())]
        );
    }

    #[test]
    fn socket_tokens_normalized() {
        assert_eq!(
            socket_version_token(Path::new("/var/run/php-fpm-8.3.sock")).unwrap(),
            "8.3"
        );
        assert_eq!(
            normalize_version_token(
                &socket_version_token(Path::new("/var/run/php-fpm-8.3.sock")).unwrap()
            ),
            "8.3"
        );
        assert_eq!(
            normalize_version_token(
                &socket_version_token(Path::new("/var/run/php-fpm83.sock")).unwrap()
            ),
            "8.3"
        );
        // 无版本 token 的 socket 文件名返回 None → 归一化空串（调用方跳过）
        assert!(socket_version_token(Path::new("/var/run/php-fpm.sock")).is_none());
        assert_eq!(normalize_version_token(""), "");
    }

    #[test]
    fn dir_version_ordering() {
        assert_eq!(dir_version_segments("mysql-8.4"), vec![8, 4]);
        assert_eq!(dir_version_segments("php-74"), vec![74]);
        assert!(dir_version_segments("nginx").is_empty());
        // 数值比较：10.0 高于 9.0（字符串排序会判反）
        assert!(dir_version_segments("mysql-10.0") > dir_version_segments("mysql-9.0"));
    }

    #[test]
    fn db_version_parsing() {
        assert_eq!(
            db_version(
                "mysql",
                "mysqld  Ver 8.4.0 for Linux on x86_64 (MySQL Community Server - GPL)"
            ),
            "8.4.0"
        );
        assert_eq!(
            db_version(
                "mariadb",
                "mariadbd  Ver 10.11.6-MariaDB for Linux on x86_64"
            ),
            "10.11.6"
        );
        assert_eq!(
            db_version("postgresql", "postgres (PostgreSQL) 16.2"),
            "16.2"
        );
        assert_eq!(
            db_version("redis", "Redis server v=7.2.4 sha=00000000:0"),
            "7.2.4"
        );
        assert_eq!(db_version("mongodb", "db version v7.0.5"), "7.0.5");
    }
}

#[cfg(test)]
mod python_tests {
    use super::*;

    /// 版本号要拼进 shell 命令，必须严格限制（防注入）。
    /// 允许字母是为预发布后缀（3.15.0rc2），其余一律拒绝。
    #[test]
    fn version_token_must_be_numeric() {
        assert!(is_safe_version("3"));
        assert!(is_safe_version("3.12"));
        assert!(is_safe_version("3.12.4"));
        assert!(is_safe_version("3.15.0rc2"));
        assert!(!is_safe_version(""));
        assert!(!is_safe_version("3.12; rm -rf /"));
        assert!(!is_safe_version("3.12 ls"));
        assert!(!is_safe_version("$(id)"));
        assert!(!is_safe_version("3.12`id`"));
    }

    #[test]
    fn uv_list_parses_cpython_token() {
        assert_eq!(
            uv_list_version(
                "cpython-3.12.4-linux-x86_64-gnu   /root/.local/share/uv/python/x/bin/python3.12"
            )
            .unwrap(),
            "3.12.4"
        );
        // 表头 / 分隔行不能误判成版本
        assert!(uv_list_version("Installed versions").is_none());
        assert!(uv_list_version("Available for download").is_none());
    }

    #[test]
    fn python_detect_shape_is_stable() {
        let v = detect_python();
        // uv 可能没装，但字段必须齐全，否则面板渲染会缺字段
        assert!(v.get("uv").is_some());
        assert!(v.get("uv_path").is_some());
        assert!(v.get("uv_version").is_some());
        assert!(v.get("versions").and_then(|x| x.as_array()).is_some());
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::*;

    /// 国内镜像模式下必须带上 Node 发行版镜像，否则还是直连 nodejs.org。
    #[test]
    fn fnm_script_adds_dist_mirror_only_in_china_mode() {
        let cn = fnm_script("fnm install 20", "china");
        assert!(cn.contains("FNM_NODE_DIST_MIRROR"), "{cn}");
        assert!(cn.contains(NODE_DIST_MIRROR), "{cn}");

        let off = fnm_script("fnm install 20", "official");
        assert!(!off.contains("FNM_NODE_DIST_MIRROR"), "{off}");
    }

    /// 装 uv 要先试 pip（走已配的 PyPI 源，国内快），最后才回退官方脚本。
    #[test]
    fn uv_install_tries_pip_before_official_script() {
        assert!(
            INSTALL_UV_CMD.contains("pip3 install"),
            "{}",
            INSTALL_UV_CMD
        );
        assert!(INSTALL_UV_CMD.contains("astral.sh"), "{}", INSTALL_UV_CMD);
        // pip 失败才轮到 curl：必须是 || 兜底而不是无条件执行
        let pip_pos = INSTALL_UV_CMD.find("pip").unwrap();
        let curl_pos = INSTALL_UV_CMD.find("astral.sh").unwrap();
        assert!(pip_pos < curl_pos);
    }
}
