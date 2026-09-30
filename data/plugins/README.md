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
| `options` | 运行选项（结构同应用商店 `app.yaml` 的 `options`：`name`/`label`/`type`/`default`/`required`/`choices`…）。`type` 支持 `string`/`number`/`bool`/`select`/`multiselect`，以及插件扩展的 **`dir`**（目录选择器）、**`file`**（单文件选择器）、**`files`**（多文件选择器）。`dir`/`file`/`files` 都回传**绝对路径**（多选以空格连接成单个字符串）；`dir` 留空表示站点根 |
| `actions` | 动作键 → 按钮文案，如 `run: 创建`；`on_run` 收到 `ctx.action` |
| `async` | 可选（`true`/`false`，默认 `false`）。`true` 时插件**异步执行**：`/plugin/run` 立即返回 `task_id` + `log_path`，前端用 SSE（`/plugin/watch`）实时收日志流；收尾时 zapexec 写入 `__ZAP_DONE__ <code>` 哨兵。适合联网下载、编译等耗时任务（避免前端请求超时）。轻量任务省略即可，走同步（一次性返回 `log`）。异步插件支持**运行中取消**：前端调 `POST /plugin/cancel`（`{token, task_id}`），zapexec 看门狗杀掉子进程（组），任务以退出码 `-2`（已取消）收尾并通过 SSE 推送「任务已取消」 |

### `dir` 类型选项示例

`dir` 渲染为一个「只读输入框 + 选择目录按钮」，点开目录选择器，回传**绝对路径**（与 `file`/`files` 一致）。留空表示站点根。插件里直接把它当目标路径用即可，**不要**再和 `site_root` 拼接（否则绝对路径会被重复拼一次）：

```lua
-- main.lua
local dest = zap.option("TARGET")
if dest == "" then dest = zap.site_root() end   -- 留空回落到站点根
-- dest 已是绝对路径，可直接传给 zap.exec_as_user
```

```yaml
options:
  - name: TARGET
    label: 目标目录
    type: dir          # ← 目录选择器，回传绝对路径
    default: ""        # 空 = 站点根目录本身
    desc: 目录选择器；回传绝对路径（留空 = 站点根）
```

行为约定：

- 选中 `/home/xxx/site/public` → 回传该绝对路径；不选择 → 回传空串（=`site_root`）。
- 目录选择器从站点根（无站点上下文时从用户家目录）出发浏览，但回传值始终是绝对路径。
- 隔离规则与文件管理一致：管理员可一路向上，普通用户出不了自己的 `home`。

> 注意：`dir` 回传的是字符串，在 `main.lua` 里用 `zap.option("TARGET")` 读取；因为回传的是绝对路径，插件无需再拼站点根。

### `file` / `files` 类型选项示例

`file` 渲染单文件选择器（点文件选中、双击确认），`files` 渲染多文件选择器（每个文件带勾选框）。两者都从站点根（无站点上下文时从家目录）出发浏览，回传**绝对路径**：

```yaml
options:
  - name: SRC            # 单文件
    label: 源文件
    type: file
    desc: 选择一个文件，回传其绝对路径
  - name: SRCS           # 多文件
    label: 源文件清单
    type: files
    desc: 可多选，回传以空格连接的绝对路径串（如 /a.txt /b.txt）
```

行为约定：

- 两者都回传字符串；`files` 多选时把路径用**空格**拼成一个字符串（与 `multiselect` 一致），在 `main.lua` 里用 `zap.option("SRCS")` 拿到后用 `split(" ")` 拆开即可。
- 路径是绝对路径（与 `dir` 一致），插件可直接 `zap.exec_as_user("cat", { path })` 之类使用，无需再拼站点根。
- 隔离规则与文件管理一致：管理员可一路向上，普通用户出不了自己的 `home`。

## main.lua 可用能力（全局表 `zap`）

- `zap.log(msg)` —— 日志（回传前端）
- `zap.option(name)` —— 读取运行选项
- `zap.exec(prog, {args})` —— 以 root 执行（仅 `scope=system`）
- `zap.exec_as_user(prog, {args})` —— 以站点 Linux 账号执行（仅 `scope=site`）
- `zap.site_root()` / `zap.site_linux_user()` —— 当前站点文档根 / 运行账号（`scope=site`）
- `zap.home_dir()` —— 当前执行身份的家目录：`scope=system` 时为调用方 home，`scope=site` 时为站点 Linux 账号的 home（如 `/home/admin`）

## 安全边界

- 插件只能声明**结构化 UI 元数据**，不能注入任意前端代码/HTML/路由；前端由受信任的 `<PluginSlot>` 组件渲染。
- 执行身份由 `scope` 决定；`site` 走 `user_cmd` + `drop_privileges`（清附加组 → setgid → setuid）降到站点账号，`system` 才是 root。
- 列表/运行前 `zapd` 校验操作者身份与站点归属（复用 `site_in_scope`）。

## 最小闭环示例

把 `examples/` 下的插件目录复制到某个用户的 `.zap` 目录下即可在对应站点详情页出现入口：

```sh
cp -r data/plugins/examples/composer-create ~/<user>/.zap/plugins/composer-create
cp -r data/plugins/examples/widgets-demo   ~/<user>/.zap/plugins/widgets-demo
```

- `composer-create`：真实可用的「在站点根下 composer create-project」示例（`dir` 控件）。
- `widgets-demo`：**控件全家桶示例**，把 `string` / `number` / `bool` / `select` / `multiselect` / `dir` / `file` / `files` 全部演示一遍，`main.lua` 会把每个控件回传的值打印出来，方便对照前端与 `zap.option` 的取值。
