# Zap 更新日志（中文版）

> 文档源：`CHANGELOG_zh-CN.md`（位于仓库根）。  
> 由 `build.sh` 拷贝到 `data/www/html/CHANGELOG_zh-CN.md`，  
> 后端 `GET /api/docs/changelog?lang=zh-CN` 渲染为 HTML 后展示。

## [v1.0.10] - 2026-09-14

- 增加 i18n（vue-i18n 集成，新增 `zh-CN` 与 `en-US` 命名空间）
- 新增「文档」菜单：更新日志 / 用户手册 / FAQ / 升级指南

> 完整英文版参见 [`CHANGELOG.md`](./CHANGELOG.md)。

## [v1.0.18] - 2026-09-29

### 安全加固

- **#6 越权防护：`zapexec` 信任 `zapd` 授权（Medium）** —— 执行端不再盲信面板授权，新增两道独立校验（defense-in-depth）：
  - **文件操作路径沙箱**：`list` / `read` / `write` / `delete` / `mkdir` / `rename` / `download` / `upload` / `chmod` / `chown` / `copy` / `archive` / `info` 在以 root 执行前，对普通用户（非管理员）的解析路径独立校验其必须落在本人 **家目录（`/etc/passwd` 的 `pw_dir`）/ 私有临时目录 `/tmp/zap-<user>` / 面板数据目录 `data/users/<user>`** 之内；已存在路径额外 `canonicalize` 比对，防御符号链接逃逸（如 `/home/u/evil -> /etc`）。管理员与 root 行为不变。
  - **应用部署归属校验**：`AppDeploy` 新增请求方身份字段，`deploy` 校验普通用户部署的应用其运行账号 `owner_user` 必须等于请求方本人，杜绝以他人站点账号拉起应用（跨站点篡改 / 提权）；管理员不受此约束（`skip_owner_check`），站点归属仍由 `zapd` 校验。
  - 未绑定系统账号的普通用户直接拒绝，不再以 root 裸跑。

- **#7 async 处理器内同步阻塞 I/O（High）** —— tokio worker 中做阻塞 `std::fs` 会卡住整个异步线程池：
  - 将 `routers/**` 内全部阻塞文件操作（站点日志目录迁移 `rename`、应用商店 `provision`/`info.yaml` 读取与脚本树遍历、phpMyAdmin FPM socket 探测的 `read_dir`/`metadata`/`canonicalize`、四层转发证书落盘的 `create_dir_all`/`write`/`set_permissions`/`read_dir`/`remove_file`、Zap 设置证书读取/自签/删除、MySQL `my.cnf` 解析、上传临时目录创建与 chmod 当前权限读取）统一改为 `tokio::fs`（async 路径）或 `tokio::task::spawn_blocking`。
  - MySQL 配置解析（`parse_mycnf`）改为异步并通过 `tokio::sync::OnceCell` 缓存，递归读取走 `tokio::fs`。
  - 新增 clippy 守卫：根目录 `clippy.toml` 用 `disallowed-types` 禁止 `routers/**` 直接使用 `std::fs` 阻塞 I/O 类型（`File`/`OpenOptions`/`ReadDir`/`DirEntry`/`FileType`），并在 `routers/mod.rs` 顶部 `#![deny(clippy::disallowed_types)]`（测试构建豁免）防止回归；`Metadata`/`Permissions` 因是 `tokio::fs` 返回/所需类型而特意放行。

- **#9 写入路径 `unwrap_or(false)` 掩盖唯一性检查（Medium）** —— `system_stream.rs:968/1139` 端口重复检查失败被默认成"不重复"，可能让两条规则绑定同一 `listen_ip:port`，导致 `nginx -t` 失败、状态不一致；现已改为用 `?` 传播数据库错误，检查失败即返回错误而非默认放行。

- **#12 响应信封不一致（Medium）** —— 约 35 处 `json!({code})` 形态各异（部分缺失 `message`、`database.rs` 用私有 `ok()` 且无 `message`、部分 `data` 内嵌 `ok:true`）。新增统一信封 helper `zap::api_ok` / `api_ok_msg` / `api_ok_data` / `api_err`（成功固定 `{code:0,message,data}`、错误 `{code,message}`），并把 `database.rs` 的私有 `ok()` 改为 `api_ok`、无 `message` 的 `code:0,data` 成功响应统一改用 `api_ok`。注：`database` 响应 `data.ok` 字段因前端 `DbStatus.ok` 等类型契约仍保留，未删除。

- **#15 CI 噪声/缺口（Medium）** —— `ci.yml` 删除为商业模块 `zappro` 建占位 `mod.rs` 的死步骤（无任何 `Cargo.toml` 引用、CI 也不启用 `commercial` feature）；`web/tsconfig.node.json` 移除不存在的 `eslint.config.*` 引用（项目无 eslint 依赖）；`release.yml` 将 `cross` 由 git HEAD 固定到已发布版本 `0.2.5`，并把 `checkout` / `setup-node` / `cache` / `rust-toolchain` / `upload-artifact` / `download-artifact` / `action-gh-release` 等第三方 action 全部固定到 commit SHA（供应链加固）。

- **#19 README 与代码不符（Low）** —— README 将 Docker 容器管理与应用商店插件生态列为 Roadmap，但二者均已落地（`routers/docker.rs` 实现容器/镜像/网络/Compose 编排并感知 Docker/Podman 运行时，配套完整 `web/src/views/docker/` 前端；应用商店已在 `docs/api/appstore.md` 文档化）。核心特性与 Roadmap 现已更正为已交付状态。

- （同系列）JWT 默认密钥缺失时启动即失败（fail-closed）；高级反代 `raw` 体与自定义 `rewrite` 规则防止 `include` / 系统路径 / 云元数据注入。
### 插件平台

- **宿主 UIKit**：`data/plugins/_lib/ui.css` / `ui.js`（随发行包，已修掉 `.gitignore` 与打包脚本漏拷的问题）—— `plugin_ui` 返回插件 HTML 前自动注入，提供主题变量 + `.zui-*` 组件类与 `zap.ui.*` 运行时（`notify` / `confirm` / `prompt` / `toast` / `table` / `tabs` / `diff` / `busy` / `lang` / `t` / `localize`）。
- **槽位多值 + 两个新槽位**：`ui.placement` 支持数组（`file.editor` 与 `file.context` 可同时挂），新增 `file.context`（文件管理器右键菜单，透传选中文件）与 `dashboard.card`（管理员仪表盘卡片）。
- **action 级 `async` / `dangerous`**：破坏性动作在前端弹二次确认，**并在服务端拦截只读演示（demo）账号**；只有指定的长操作走 SSE 日志 + 可取消。
- **插件多语言**：manifest 支持 `i18n` 表（`zh-CN` / `en-US`），覆盖标题 / 描述 / 入口文案 / 动作名 / 选项说明，缺失的键回落基准语言；语言与 Element Plus 同一套，切换后立即生效（列表重拉、已打开的界面重载）。UIKit 提供 `zap.ui.lang` / `t()` / `localize()` 让自带 HTML 界面也能跟着走。
- **定时 / Webhook 触发**：插件管理页可为任意插件加 `cron` 或 `webhook` 触发器；任务归属创建者（同账号、同角色、同权限校验），公开路径 `/api/plugin/hook/<令牌>` 靠随机令牌鉴权，支持停用与令牌轮换。
- **插件级持久化配置**：新增 `zap.config.get/set/number/bool`（按插件 + 面板用户分桶，落地 `$ZAP_PATH/data/plugins/config/<插件>.yaml`），Lua 侧终于有地方存偏好与小凭证。
- **网络能力**：公共库新增 `zap.http.get/post/request/json/download`（底层走 `curl`，继承 scope 降权，只放行 http/https）。
- **依赖声明与签名**：manifest 支持 `requires.commands`（缺命令拒装）与 `signature`（HMAC-SHA256 文件摘要，被改动拒装）。
- **冒烟测试**：插件目录放 `tests.yaml` 即可逐个 action 跑并断言日志（管理页「测试」按钮查看通过 / 失败 / 跳过）。
