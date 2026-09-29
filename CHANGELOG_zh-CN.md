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

- （同系列）JWT 默认密钥缺失时启动即失败（fail-closed）；高级反代 `raw` 体与自定义 `rewrite` 规则防止 `include` / 系统路径 / 云元数据注入。