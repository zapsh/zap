# 插件（Plugin）机制

插件是一段以 **Lua（mlua 嵌入 zapexec）** 编写的扩展，复用应用商店的执行与鉴权模型，
可在前端指定位置（槽位）呈现入口，由用户/管理员触发运行。

## 目录约定（自动发现，无需编译注册）

- **系统级**：`$ZAP_PATH/plugins/<name>/`（对所有用户/站点可用；**仅管理员**可安装）
- **用户级**：`$HOME/.zap/plugins/<name>/`（仅该面板用户可见，作用于自己的站点与家目录；登录用户自己就能装）

每个插件是一个目录，含：

- `manifest.yaml` —— 描述与 UI 元数据（下表）
- `main.lua` —— 定义 `on_run(ctx)` / `on_<action>(ctx)`，通过全局表 `zap` 调用受限能力
- `ui.html` —— 可选，自带 HTML 界面（`ui.html` 字段指向它，见「自带 HTML 界面」）
- `lib/*.lua` —— 可选，插件自带的私有函数库（自动加载，见「公共函数库」）

以 `.` 或 `_` 开头的目录（`_lib`、`.git`）不会被当成插件。

## 安装与卸载

两种方式，落地结果完全一致（都是一个插件目录）：

1. **面板上传**（应用商店 → 插件 → 上传安装）：选 `.zip` / `.tar.gz` / `.tgz` / `.tar` 包。
   包根或**唯一的顶层目录**里需含 `manifest.yaml` + `main.lua`。
2. **手工放置**：直接把目录拷到 `$ZAP_PATH/plugins/<name>/` 或 `$HOME/.zap/plugins/<name>/`。

插件名可留空，此时取 `manifest.yaml` 里的 `name`；若两者都给了则必须一致。
同名插件已存在时需勾选「覆盖安装」。

卸载 = 删除整个插件目录（面板按 `level` 定位，路径越界会被拒绝）。

### 安装信息

安装成功后，来源 / 分支 / 级别 / 安装时间会写回插件自身的 `manifest.yaml`（顶层 `zap_install:` 块）：

```yaml
zap_install:
  source: archive
  src: /usr/local/zap/data/plugins/tmp/uploads/1-1767225600000.zip
  level: user
  installed_at: 1767225600
```

列表接口据此回传 `source` / `src` / `installed_at`，不再单独维护 Meta 文件。手工放置的插件没有这段信息，界面显示来源为「手动放置」。

## manifest.yaml 字段

| 字段 | 说明 |
| --- | --- |
| `name` | 插件名（仅 `[A-Za-z0-9_-]`，与目录名一致） |
| `title` | 显示名 |
| `scope` | `site`（以站点 Linux 账号运行，需站点上下文）/ `user`（以面板用户账号运行，不绑定站点）/ `system`（以 root 运行，仅系统级） |
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

### 基础

- `zap.log(msg)` —— 日志（回传前端）
- `zap.logf(fmt, ...)` —— `string.format` 后写日志（公共库提供）
- `zap.option(name)` —— 读取运行选项
- `zap.run(prog, {args})` —— **按 scope 自动分派**：`site` 走站点账号，`user` 走面板用户账号，`system` 走 root
- `zap.try_run(prog, {args})` —— 同上，但失败不抛错，返回 `(ok, output)`
- `zap.exec(prog, {args})` —— 强制以 root 执行（仅 `scope=system`；`site`/`user` 下被禁止）
- `zap.exec_as_user(prog, {args})` —— 强制以运行账号执行：`site` 走站点账号，`user` 走面板用户账号
- `zap.site_root()` / `zap.site_linux_user()` —— 当前站点文档根 / 运行账号（仅 `scope=site`，其余返回空）
- `zap.home_dir()` —— 当前执行身份的家目录：`scope=system` 为调用方 home，`scope=site` 为站点账号 home，`scope=user` 为面板用户 home
- `zap.plugin_dir()` —— 插件自身目录（读自带资源用）
- `zap.scope()` / `zap.level()` —— `"site"|"user"|"system"` / `"user"|"system"`
- `zap.canceled()` —— 异步插件轮询它判断用户是否点了取消
- `zap.read_file(p)` / `zap.write_file(p, s)` / `zap.append_file(p, s)` —— 按 scope 降权读写文件
- `zap.json_encode(v)` / `zap.json_decode(s)`
- `zap.time()` / `zap.date(fmt[, ts])` —— 时间戳 / 格式化（`%Y-%m-%d %H:%M:%S`）
- `zap.env(name)` —— 环境变量（只放开 `ZAP_*` 与 `PATH/HOME/USER/SHELL/LANG/TMPDIR`）

## 自带 HTML 界面（可选）

声明 `ui.html: <文件名>` 后，前端不再渲染结构化表单，改为把该文件塞进
`sandbox="allow-scripts"` 的 iframe（**不含 `allow-same-origin`**）。
面板自动注入 `window.zap`，界面靠 `postMessage` 回调后端：

```js
await zap.call('scan', { TARGET: 'public' })             // -> Promise<输出文本>
await zap.call('du', { TARGET: '' }, (line) => { ... })  // 流式日志（异步插件）
```

请求按 `action` 分发到 `main.lua` 的 `on_<action>`（未定义则回落到 `on_run`）。
完整说明与示例见 `docs/guide/plugin-dev.md` 与 `examples/html-demo/`。

## 公共函数库（自动加载）

`main.lua` 之前，zapexec 会按以下顺序自动加载 `*.lua`，无需 `require`：

1. `$ZAP_PATH/data/plugins/_lib/` —— 系统级公共库（随发行包提供 `zap.lua`）
2. `$HOME/.zap/plugins/_lib/` —— 用户级公共库
3. `<plugin_dir>/lib/` —— 插件自带的私有库

后者可覆盖前者。库里已经定义好的 helpers：

| 分类 | 函数 |
| --- | --- |
| 日志 / 断言 | `zap.logf` `zap.assert` `zap.fail` |
| 字符串 | `zap.str.trim` `.blank` `.split` `.join` `.starts` `.ends` `.contains` `.lines` |
| 路径 | `zap.path.join` `.dirname` `.basename` `.ext` `.is_abs` `.normalize` `.within` `.site` |
| 表 | `zap.tbl.keys` `.values` `.count` `.map` `.filter` `.contains` `.index_of` `.merge` |
| 文件系统 | `zap.fs.exists` `.is_dir` `.is_file` `.read` `.write` `.append` `.mkdir` `.remove` `.copy` `.move` `.chmod` `.list` `.read_lines` `.size` `.append_line` `.grep` |
| Shell | `zap.shell_quote`（别名 `zap.q`） |
| 选项 | `zap.opt` `.opt_required` `.opt_bool` `.opt_number` `.opt_list` |
| 其它 | `zap.to_json` `.from_json` `.now` `.fmt_time` `.is_site_scope` |
| 顶层别名 | `zap.split` `zap.trim` `zap.join` `zap.q` |

`zap.fs.*` 全部基于 `zap.run`，因此**自动继承 scope 降权** —— `scope=site` 时不会以 root 碰到文件。

示例：

```lua
function on_run(ctx)
  local name = zap.opt_required("NAME")
  local dir  = zap.path.site(zap.opt("TARGET", ""))   -- 拼接站点根并校验不越界
  if not zap.fs.is_dir(dir) then zap.fs.mkdir(dir) end
  zap.fs.write(zap.path.join(dir, name .. ".txt"), "hello\n")
  zap.logf("写了 %d 字节到 %s", #("hello\n"), dir)
  for _, line in ipairs(zap.fs.read_lines(zap.path.join(dir, ".env"))) do
    zap.log(line)
  end
end
```

## 安全边界

- **沙箱只启用 `table` / `string` / `math` / `utf8` / `coroutine`**，没有 `io` / `os` / `package` / `debug`。
  插件跑在 zapexec 进程里，放开 `io` 就能以 root 身份读写任意文件，直接绕过 `scope=site` 的降权
  （`drop_privileges` 只对子进程生效）。要读文件用 `zap.fs.*` / `zap.read_file`。
- 默认只声明**结构化 UI 元数据**，前端由受信任的 `<PluginSlot>` 组件渲染。
- 声明 `ui.html` 的插件可以自带 HTML 界面，但运行在 `sandbox="allow-scripts"` 的 iframe 里
  （**不含 `allow-same-origin`**）：碰不到面板 DOM / Cookie / localStorage，也发不出带凭据的请求，
  要调后端只能 `postMessage` 给父页面，由父页面带真实 JWT 代跑 `/plugin/run`。权限点与 scope 仍在后端把关。
- 执行身份由 `scope` 决定；`site` 走 `user_cmd` + `drop_privileges`（清附加组 → setgid → setuid）降到站点账号，`system` 才是 root。
- **用户级插件不允许 `scope: system`**（安装和运行两处都会拒绝）—— 否则普通用户往 `~/.zap/plugins`
  放一个插件就能以 root 执行任意命令。系统级插件只有管理员能安装 / 卸载。
- 列表/运行前 `zapd` 校验操作者身份与站点归属（复用 `site_in_scope`）。
- 接口权限点：`plugin:view` / `plugin:run` / `plugin:install` / `plugin:uninstall`，
  登记在 `zapd/src/routers/access.rs` 的 `RULES` 里（角色权限页可勾选「插件」模块）。

## 最小闭环示例

把 `examples/` 下的插件目录复制到某个用户的 `.zap` 目录下即可在对应站点详情页出现入口：

```sh
cp -r data/plugins/examples/composer-create ~/<user>/.zap/plugins/composer-create
cp -r data/plugins/examples/widgets-demo   ~/<user>/.zap/plugins/widgets-demo
```

- `composer-create`：真实可用的「在站点根下 composer create-project」示例（`dir` 控件）。
- `widgets-demo`：**控件全家桶示例**，把 `string` / `number` / `bool` / `select` / `multiselect` / `dir` / `file` / `files` 全部演示一遍，`main.lua` 会把每个控件回传的值打印出来，方便对照前端与 `zap.option` 的取值。
