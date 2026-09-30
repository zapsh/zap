# 插件（Plugin）机制

插件是一段以 **Lua（mlua 嵌入 zapexec）** 编写的扩展，复用应用商店的执行与鉴权模型，
可在前端指定位置（槽位）呈现入口，由用户/管理员触发运行。

## 目录约定（自动发现，无需编译注册）

- **系统级**：`$ZAP_PATH/plugins/<name>/`（对所有用户/站点可用；由 admin 放置或经应用商店 `plugin` 分类安装）
- **用户级**：`$HOME/plugins/<name>/`（仅该面板用户可见，作用于自己的站点与家目录；直接放置/上传即用，零安装）

每个插件是一个目录，含：

- `manifest.yaml` —— 描述与 UI 元数据（下表）
- `main.lua` —— 定义 `on_run(ctx)`，通过全局表 `zap` 调用受限能力

## manifest.yaml 字段

| 字段 | 说明 |
| --- | --- |
| `name` | 插件名（仅 `[A-Za-z0-9_-]`，与目录名一致） |
| `title` | 显示名 |
| `scope` | `site`（以站点 Linux 账号运行，需站点上下文）/ `system`（以 root 运行） |
| `ui.placement` | 前端入口槽位：`site.detail`（站点详情页「插件」Tab）等 |
| `ui.label` / `ui.icon` / `ui.tab` | 按钮文案 / 图标 / 分组 |
| `options` | 运行选项（结构同应用商店 `app.yaml` 的 `options`：`name`/`label`/`type`/`default`/`required`/`choices`…） |
| `actions` | 动作键 → 按钮文案，如 `run: 创建`；`on_run` 收到 `ctx.action` |

## main.lua 可用能力（全局表 `zap`）

- `zap.log(msg)` —— 日志（回传前端）
- `zap.option(name)` —— 读取运行选项
- `zap.exec(prog, {args})` —— 以 root 执行（仅 `scope=system`）
- `zap.exec_as_user(prog, {args})` —— 以站点 Linux 账号执行（仅 `scope=site`）
- `zap.site_root()` / `zap.site_linux_user()` —— 当前站点文档根 / 运行账号（`scope=site`）

## 安全边界

- 插件只能声明**结构化 UI 元数据**，不能注入任意前端代码/HTML/路由；前端由受信任的 `<PluginSlot>` 组件渲染。
- 执行身份由 `scope` 决定；`site` 走 `user_cmd` + `drop_privileges`（清附加组 → setgid → setuid）降到站点账号，`system` 才是 root。
- 列表/运行前 `zapd` 校验操作者身份与站点归属（复用 `site_in_scope`）。

## 最小闭环示例

把 `examples/composer-create/` 复制到某个用户的家目录即可在对应站点详情页出现入口：

```sh
cp -r data/plugins/examples/composer-create ~/<user>/plugins/composer-create
```
