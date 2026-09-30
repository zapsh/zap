# 插件（Plugin）机制

插件是一段以 **Lua（mlua 嵌入 zapexec）** 编写的扩展，复用应用商店的执行与鉴权模型，
可在前端指定位置（槽位）呈现入口，由用户/管理员触发运行。

## 目录约定（自动发现，无需编译注册）

- **系统级**：`$ZAP_PATH/plugins/<name>/`（对所有用户/站点可用；由 admin 放置或经应用商店 `plugin` 分类安装）
- **用户级**：`$HOME/.zap/plugins/<name>/`（仅该面板用户可见，作用于自己的站点与家目录；直接放置/上传即用，零安装）

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
| `options` | 运行选项（结构同应用商店 `app.yaml` 的 `options`：`name`/`label`/`type`/`default`/`required`/`choices`…）。`type` 支持 `string`/`number`/`bool`/`select`/`multiselect`，以及插件扩展的 **`dir`**（目录选择器：在站点详情场景会从站点根出发选目录，并把选中结果按站点根裁切成**相对路径**回传，契合 `main.lua` 里 `site_root .. "/" .. target` 的拼法；无站点上下文时返回绝对路径） |
| `actions` | 动作键 → 按钮文案，如 `run: 创建`；`on_run` 收到 `ctx.action` |

### `dir` 类型选项示例

`dir` 渲染为一个「只读输入框 + 选择目录按钮」，点开目录选择器。它最适合**站点作用域**插件里"站点根下的某个子目录"这类输入——回传的是**相对站点根的路径**，与 `main.lua` 里 `site_root .. "/" .. target` 的拼法天然契合：

```yaml
options:
  - name: TARGET
    label: 目标子目录
    type: dir          # ← 目录选择器
    default: ""        # 空 = 站点根目录本身
    desc: 留空则直接建在站点根目录；用「选择目录」从站点根下挑一个子目录
```

行为约定：

- 选中站点根下的 `public/` → 回传 `public`；直接选站点根本身 → 回传空串（=`site_root`）。
- 站点详情页会把它从 `web_root` 出发选目录，并自动裁掉 `web_root` 前缀；`PluginSlot` 通过 `:web-root` 拿到该值。
- 非站点上下文（拿不到 `web_root`）时，选择器从用户家目录出发，回传**绝对路径**。

> 注意：`dir` 回传的是字符串，在 `main.lua` 里仍用 `zap.option("TARGET")` 读取，插件无需为它做特殊处理。

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

把 `examples/composer-create/` 复制到某个用户的 `.zap` 目录下即可在对应站点详情页出现入口：

```sh
cp -r data/plugins/examples/composer-create ~/<user>/.zap/plugins/composer-create
```
