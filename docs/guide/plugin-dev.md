# 插件开发指南

ZAP 插件是**一个目录**里的三个文件，用 Lua 写逻辑，跑在 zapexec（root 守护进程）内嵌的
Lua 沙箱里。安装即用，不需要编译、不需要重启面板。

```
my-plugin/
├── manifest.yaml   描述 + 前端入口元数据（必需）
├── main.lua        逻辑入口 on_run(ctx) / on_<action>(ctx)（必需）
├── ui.html         可选，自带 HTML 界面（manifest 里 ui.html 指向它）
└── lib/*.lua       可选，插件自带的私有函数库（自动加载）
```

- **系统级**：`$ZAP_PATH/plugins/<name>/` —— 所有用户可见，**仅管理员**可安装（普通用户只能使用，不能自行安装）

以 `.` 或 `_` 开头的目录（`_lib`、`.git`）不会被当作插件。

---

## 1. 五分钟上手

`manifest.yaml`：

```yaml
name: hello
title: 打个招呼
version: 1.0.0
description: 最小可运行插件
author: you
scope: user            # user = 以调用方面板用户账号运行（不绑定站点）；site = 以站点账号；system = 以 root（仅系统级）

ui:
  placement: site.detail   # 出现在「站点详情 → 插件」标签页
  label: 打个招呼
  icon: material-symbols:waving-hand

options:
  - name: NAME
    label: 名字
    type: string
    default: world
    required: true
```

`main.lua`：

```lua
function on_run(ctx)
  local name = zap.opt('NAME', 'world')
  zap.log('hello, ' .. name)
  zap.log('运行在: ' .. zap.path.site(''))
end
```

把目录拷到 `$ZAP_PATH/plugins/hello/`（仅管理员可写），
打开「站点详情 → 插件」就能看到入口。

### 非站点插件：`scope: user`

有些插件根本不绑定某个站点（比如管理「当前面板用户」自己的 SSH 密钥、cron、家目录里的工具），
它们应该用 `scope: user`：

```yaml
name: my-keys
title: 我的密钥
scope: user            # 以触发这次请求的面板用户身份运行，不需要站点
ui:
  placement: site.detail
  label: 我的密钥
```

```lua
function on_run(ctx)
  local me = zap.home_dir()              -- 面板用户的家目录
  zap.log('当前用户家目录: ' .. me)
  zap.run('ls', { '-la', me })           -- 自动以该用户身份执行
end
```

要点：

- `scope: user` **不依赖站点上下文**，入口可挂在任意槽位，运行时也不会要求 `site_root`。
- 运行身份是**触发请求的那个面板用户**的 Linux 账号——即使是管理员安装的系统级插件，
  被普通用户触发时也只拥有该普通用户的权限，不会提权。
- `zap.site_root()` / `zap.site_linux_user()` 在 `scope: user` 下返回空串；
  取用户家目录请用 `zap.home_dir()`，不要用 `zap.path.site(...)`（那是站点根拼接）。
- `zap.exec`（root）在 `scope: user` 下被禁止；执行命令一律走 `zap.run` / `zap.exec_as_user`。

---

## 2. manifest.yaml 完整字段

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | ✓ | 插件标识，**只允许字母数字、下划线、连字符**，必须与目录名一致 |
| `title` | | 展示标题，缺省用 `name` |
| `version` / `description` / `author` / `homepage` | | 元信息，管理页展示 |
| `scope` | | `site`（以站点 Linux 账号运行，需站点上下文）/ `user`（以调用方面板用户账号运行，不绑定站点）/ `system`（以 root 运行，仅管理员安装的插件可用） |
| `async` | | `true` 时后台执行，日志走 SSE 实时回传，前端可取消 |
| `ui.placement` | ✓ | 入口挂载位置，**可以是单个槽位或槽位数组**，见下表（旧的单个字符串写法仍然生效） |
| `ui.label` | | 入口按钮文案 |
| `ui.icon` | | Iconify 图标名 |
| `ui.tab` | | 补充说明（显示在卡片副标题） |
| `ui.html` | | 自带 HTML 界面的文件名（插件目录内的单个 `.html`） |
| `options` | | 结构化表单，见 §4 |
| `actions` | | `action: 文案` 映射，缺省用第一个作按钮文案；也可以写成带开关的对象，见 §7 |
| `requires.commands` | | 依赖的外部命令，缺命令**直接拒装**，见 §8 |
| `signature` | | 插件文件摘要（HMAC-SHA256），被改动过会拒装，见 §8 |
| `i18n` | | （可选）按面板语言覆盖文案；不提供则始终显示基准语言，见 §2 的「多语言」小节 |

### placement（挂载位置）

| 值 | 位置 |
| --- | --- |
| `site.detail` | 站点详情抽屉 →「插件」标签页 |
| `file.editor` | 文件编辑器浮窗 / 文件管理器顶部工具栏（`mode="toolbar"`，父页面把当前目录作为 `options.cwd` 透传） |
| `file.context` | 文件管理器列表右键菜单（`mode="context"`，额外把选中的文件以 `options.files`（JSON 数组字符串）透传） |
| `dashboard.card` | 仪表盘（管理员首页）底部的一张卡片；没挂插件时整块不显示 |

过滤是「任一个槽位命中」匹配：插件不区分站点上装了什么应用，只按槽位出现。
一个插件可以挂多处：

```yaml
ui:
  placement:
    - file.editor
    - file.context
```

宿主只渲染它认识的槽位，老插件写了未知槽位也不会报错（只是不显示）。

### 多语言（i18n，可选）

上面的字段都是**基准语言**（写作者想写的任意语言，多数是中文）—— **插件完全可以不提供翻译**，
此时无论面板切到什么语言都显示这份基准文案，不会被当成错误、也不会出现空白。
想跟着面板语言走，才额外加一张 `i18n` 表：

```yaml
title: Git 管理
description: 在文件管理器里管理 Git 仓库
ui:
  label: Git
options:
  - name: cwd
    label: 目录
actions:
  status: 状态
  pull: 拉取

i18n:
  en-US:
    title: Git manager
    description: Manage Git repositories from the file manager
    label: Git
    actions:
      status: Status
      pull: Pull
    options:
      - name: cwd
        label: Directory
```

规则：

- **可选、可增量**：没有 `i18n` 表，或表里没有当前语言（比如只写了 `zh-CN`，用户切到英文），
  一律回落基准文案。既不强制提供 `en-US`，也不要求一次翻全 —— 先翻标题也是一个有效的中间状态。
- 语言键按 `zh-CN` → `en-US` 匹配，支持地区变体回退（`en-GB` 也会命中 `en-US`）。
- 能覆盖的字段：`title` / `description` / `ui.label` / `ui.tab` / `actions.<名字>` /
  `options[].label|desc|placeholder`。**结构**（`name`、`type`、`choices`、scope…）永远
  来自基准字段，翻译表只管文案 —— 免得中英文两份结构写歪还不自知。
- 翻译表里出现基准 `actions` 没有的动作名 = 写错了名字，面板上永远看不到它（CI 里会报错）。
  （注意这条只在你**提供了**某张翻译表时才检查；一张表都不写就不检查。）
- 面板语言与 **Element Plus 同一套**（右上角切换），`/plugin/list` 会把当前语言带过来；
  切换语言时宿主会重新拉列表并重载已打开的插件界面。

自带 HTML 界面也可以跟着走（同样是**可选**的：不处理 `zap.ui.lang` 的插件就一直显示它自己那份文案）：
宿主注入界面时会在最前面写 `window.__ZAP_LANG__`，UIKit 提供 `zap.ui.lang` 与 `zap.ui.t()`（见 §6）。

---

## 3. main.lua：全局表 `zap`

沙箱只启用 `table` / `string` / `math` / `utf8` / `coroutine` —— **没有 `io` / `os` / `package` / `debug`**。
理由很实在：插件跑在 zapexec 进程里，放开 `io` 就能以 root 读写任意文件，
直接绕过 `scope: site` 的降权（降权只对子进程生效）。要读文件一律走 `zap.fs.*`。

### 入口

```lua
function on_run(ctx) end        -- 默认入口
function on_scan(ctx) end       -- action=scan 时优先调用（HTML 界面用）
```

`ctx` 是 `{ action, scope, level }`。`action` 缺省 `run`；
`on_<action>` 不存在时回落到 `on_run`。

### 核心能力

| 调用 | 说明 |
| --- | --- |
| `zap.log(msg)` | 写日志，回传前端 |
| `zap.logf(fmt, ...)` | `string.format` 后写日志 |
| `zap.option(name)` | 读取选项 |
| `zap.run(prog, {args})` | **按 scope 自动分派**：`site` 走站点账号，`user` 走面板用户账号，`system` 走 root |
| `zap.try_run(prog, {args})` | 同上，失败不抛错，返回 `(ok, output)` |
| `zap.exec(prog, {args})` | 强制 root（仅 `scope: system`） |
| `zap.exec_as_user(prog, {args})` | 强制降权到运行账号：`site` 走站点账号，`user` 走面板用户账号 |
| `zap.site_root()` / `zap.site_linux_user()` | 站点文档根 / 站点运行账号（仅 `scope=site`，其余返回空） |
| `zap.home_dir()` | 当前执行身份的家目录（`site`=站点账号 home / `user`=面板用户 home / `system`=调用方 home） |
| `zap.plugin_dir()` | 插件自身目录（读自带资源） |
| `zap.scope()` / `zap.level()` | `"site"\|"user"\|"system"` / `"user"\|"system"` |
| `zap.canceled()` | 异步插件轮询它判断用户是否点了取消 |
| `zap.read_file(p)` / `zap.write_file(p,s)` / `zap.append_file(p,s)` | 按 scope 降权读写文件 |
| `zap.json_encode(v)` / `zap.json_decode(s)` | JSON |
| `zap.time()` / `zap.date(fmt[, ts])` | 时间 |
| `zap.env(name)` | 环境变量（只放开 `ZAP_*` 与 `PATH/HOME/USER/SHELL/LANG/TMPDIR`） |

命令失败会抛错终止插件；`zap.log` 之前的内容照样回传。

---

## 4. options：结构化表单

```yaml
options:
  - name: TARGET
    label: 目标目录
    type: dir              # 相对站点根，留空 = 根目录
    default: ''
    desc: 脚本会在这个目录下工作
  - name: MODE
    label: 模式
    type: select
    default: fast
    choices:
      - { label: 快速, value: fast }
      - { label: 完整, value: full }
  - name: EXTRA
    label: 附加参数
    type: string
    placeholder: '--no-dev'
  - name: FORCE
    label: 强制覆盖
    type: bool
    default: 'false'
  - name: TAGS
    label: 标签
    type: multiselect      # 多选，空格连接成一个字符串
    choices: [a, b, c]
  - name: FILES
    label: 文件
    type: files            # 多选文件，留空 = 不选
```

| `type` | 控件 |
| --- | --- |
| `string` / `number` | 输入框 |
| `bool` | 开关 |
| `select` | 单选下拉（需在 `choices` 里） |
| `multiselect` | 多选下拉，空格连接 |
| `dir` | 目录选择器（从站点根出发） |
| `file` / `files` | 文件选择器（单选 / 多选） |

`dir` 与 `file(s)` 回传的是**相对站点根的路径**（无站点时才是绝对路径），
插件侧用 `zap.path.site(rel)` 拼回绝对路径。

---

## 5. 公共函数库（自动加载）

`main.lua` 之前，zapexec 会按以下顺序自动加载目录下的 `*.lua`，**无需 `require`**：

1. `$ZAP_PATH/data/plugins/_lib/` —— 系统级公共库（随发行包提供 `zap.lua`）
2. `<plugin_dir>/lib/` —— 插件自带的私有库

后者可覆盖前者。

| 分类 | 函数 |
| --- | --- |
| 日志 / 断言 | `zap.logf` `zap.assert` `zap.fail` |
| 字符串 | `zap.str.trim` `.blank` `.split` `.join` `.starts` `.ends` `.contains` `.lines` |
| 路径 | `zap.path.join` `.dirname` `.basename` `.ext` `.is_abs` `.normalize` `.within` `.site` |
| 表 | `zap.tbl.keys` `.values` `.count` `.map` `.filter` `.contains` `.index_of` `.merge` |
| 文件系统 | `zap.fs.exists` `.is_dir` `.is_file` `.read` `.write` `.append` `.mkdir` `.remove` `.copy` `.move` `.chmod` `.list` `.read_lines` `.size` `.append_line` `.grep` |
| Shell | `zap.shell_quote`（别名 `zap.q`） |
| 选项 | `zap.opt` `.opt_required` `.opt_bool` `.opt_number` `.opt_list` |
| 持久化配置 | `zap.config.get` `.set` `.number` `.bool`（见下） |
| 网络 | `zap.http.request` `.get` `.post` `.json` `.download`（别名 `zap.download`，见下） |
| 其它 | `zap.to_json` `.from_json` `.now` `.fmt_time` `.is_site_scope` |
| 顶层别名 | `zap.split` `zap.trim` `zap.join` `zap.q` |

`zap.fs.*` 全部基于 `zap.run`，因此**自动继承 scope 降权** —— `scope: site` 时不会以 root 碰文件。

```lua
function on_run(ctx)
  local name = zap.opt_required('NAME')
  local dir  = zap.path.site(zap.opt('TARGET', ''))   -- 拼接站点根并校验不越界
  if not zap.fs.is_dir(dir) then zap.fs.mkdir(dir) end
  zap.fs.write(zap.path.join(dir, name .. '.txt'), 'hello\n')
  zap.logf('写了 %d 字节到 %s', #('hello\n'), dir)
end
```

### 持久化配置（KV）

插件目录是 root 所有，Lua 侧也没有 `io`，想「记住用户上次填的分支 / 保存一个 token」
就得有个代管的抽屉：`zap.config.*`。它按 **插件 + 面板用户** 分桶（`$ZAP_PATH/data/plugins/config/<插件>.yaml`），
别人看不到你的配置，同一个插件在不同用户手里各存一份。

```lua
function on_run(ctx)
  local last = zap.config.get('branch', 'main')   -- 没存过返回默认值
  zap.config.set('branch', ctx.options.BRANCH or 'main')
  zap.config.set('token', nil)                    -- 传 nil = 删除
  if zap.config.bool('auto_push', false) then … end
end
```

单个键上限 8KB，整套配置适合放偏好与小凭证；别当数据库用。

### 网络请求

沙箱里没有 socket，联网一律走宿主的 `curl`（自动继承 scope 降权，
`scope: site` 时不会以 root 身份发请求）：

```lua
-- GET / POST：返回 (status, body)
local code, body = zap.http.get('https://api.example.com/v1/ping')
local code, body = zap.http.post('https://api.example.com/v1/deploy', { env = 'prod' })  -- 表自动转 JSON

-- 完整选项：method / headers / body / form / json / timeout / follow / insecure
local code, body = zap.http.request{
  url = 'https://api.example.com/v1/hook',
  method = 'POST',
  headers = { Authorization = 'Bearer ' .. token, 'X-Trace: ' .. id },
  json = { ref = 'main' },
  timeout = 15,
}

-- 直接拿解析好的表：返回 (table, status, raw)
local data, code = zap.http.json('GET', 'https://api.example.com/v1/status')

-- 下载文件（自动建父目录）
local ok, err = zap.http.download('https://example.com/app.tar.gz', '/tmp/app.tar.gz')
```

网络不通返回 `(nil, 错误信息)`；**只接受 `http://` / `https://`**，其它 scheme 直接报错
（避免有人用 `file://` 之类的协议绕一圈碰文件系统）。

---

## 6. 自带 HTML 界面（交互 / 处理输入 / 拿返回值）

插件**可以**输出自己的 HTML 界面，与用户双向交互。声明 `ui.html` 即可，
前端不再渲染结构化表单，改为加载你的页面。

```yaml
ui:
  placement: site.detail
  label: 目录体检
  icon: material-symbols:monitor-heart
  html: ui.html        # 插件目录内的单个 .html 文件
```

### 运行方式

你的 HTML 被塞进一个 `sandbox="allow-scripts"` 的 iframe（**不含 `allow-same-origin`**）。
所以它是**不透明源**：

- 碰不到面板的 DOM / Cookie / localStorage
- 发不出带凭据的请求，拿不到用户 JWT
- 要调后端，只能走下面这套 `postMessage` 桥

面板会自动注入 `window.zap`：

```js
// 调用插件，返回 Promise<输出文本>
await zap.call(action, options)

// 便捷写法，等价于 zap.call('run', options)
await zap.run(options)

// 第三个参数传回调 = 开启流式日志（异步插件逐行回推）
await zap.call('du', { TARGET: 'public' }, (line) => {
  document.getElementById('out').textContent += line + '\n'
})

// 插件级持久化配置（与 Lua 侧的 zap.config 同一份数据）
const branch = await zap.configGet('branch')   // 没存过返回 null
await zap.configSet('branch', branch || 'main') // 传 null / '' 删除该键

// 打开面板的目录 / 文件选择器
await zap.pickDir({ startPath: '' })
```

调用链路：

```
ui.html ──postMessage──▶ 父页面 ──/plugin/run（带真实 JWT）──▶ zapexec ──▶ main.lua
       ◀──postMessage──         ◀──────── 输出 / 日志 ────────────────┘
```

父页面只认**自己那个 iframe** 发来的消息（`ev.source === iframe.contentWindow`），
并会按 `action` 把请求转发给 `on_<action>`。权限点（`plugin:run`）与 `scope` 降权
全在后端，绕过不了。

### UIKit：宿主注入的样式与 `zap.ui.*`

iframe 是隔离的：拿不到 Element Plus、也继承不到面板主题，于是每个插件都得把按钮 /
表格 / 页签重写一遍，观感还各不相同。为此宿主在返回你的 HTML 时**自动注入**一套
UIKit（`data/plugins/_lib/ui.css` + `ui.js`，部署于 `$ZAP_PATH/data/plugins/_lib/`）：

1. **组件样式**：一套 CSS 变量（`--zap-primary` 等）与 `.zui-*` 类。直接写 class 即可，
   不用自己排版：

   ```html
   <div class="zui-row">
     <button class="zui-btn">保存</button>
     <button class="zui-btn ghost">取消</button>
     <span class="zui-badge ok">干净</span>
   </div>
   <div class="zui-tabs"><button class="zui-tab is-active">状态</button></div>
   <div class="zui-panel is-active">…</div>
   <table class="zui-table">…</table>
   <pre class="zui-pre">输出</pre>
   ```

   常用类：按钮（`.zui-btn` + `.ghost / .danger / .sm / .link`）、表单（`.zui-input /
   .zui-select / .zui-textarea`）、布局（`.zui-row / .zui-grow / .zui-spacer /
   .zui-section / .zui-hr`）、页签（`.zui-tabs / .zui-tab / .zui-panel`）、表格
   （`.zui-table-wrap / .zui-table`）、列表（`.zui-list / .zui-item / .zui-path`）、
   反馈（`.zui-badge / .zui-tip / .zui-warn / .zui-empty / .zui-banner`）。

2. **运行时 `zap.ui.*`**：

   | 调用 | 说明 |
   | --- | --- |
   | `zap.ui.notify(msg, type)` | 弹面板通知条（`success / warning / error / info`），宿主不接管时退化为界面内吐司 |
   | `zap.ui.confirm(msg, { danger, title })` | 面板确认框，`Promise<boolean>` |
   | `zap.ui.prompt(msg, def, { title })` | 面板输入框，`Promise<string \| null>`（取消为 null） |
   | `zap.ui.toast(msg, type)` | 只在界面内弹（不打扰宿主） |
   | `zap.ui.table(el, spec)` | 按列渲染表格：`{ columns: [{key,label,width,render}], rows, onRowClick, empty }` |
   | `zap.ui.tabs(el, list, onChange)` | 渲染页签并联动 `[data-panel]` 面板，返回 `{ select, active }` |
   | `zap.ui.diff(el, text, isErr)` | 往 `pre` 里塞 diff，按 `+ / - / @@` 行着色 |
   | `zap.ui.busy(root, on, text)` | 锁 / 放按钮，避免连点触发并发任务 |
   | `zap.ui.el(tag, attrs, children)` / `zap.ui.escape(s)` | 建元素、转义 |
   | `zap.ui.lang` | 当前面板语言（`zh-CN` / `en-US`），与 Element Plus 一致 |
   | `zap.ui.t({ ['zh-CN'] = '状态', ['en-US'] = 'Status' })` | 取当前语言的文案；给定语言时 `zap.ui.t(文案表, 'en-US')` 强制取某一套 |
   | `zap.ui.localize(dict)` | 批量：给节点标 `data-i18n="run"`，再传 `{ run = { ['zh-CN']='运行', ['en-US']='Run' } }` 自动改写文本（`data-i18n-placeholder` 改占位符） |

   例子：用宿主的通知条替代 `alert`，用 `zap.ui.confirm` 替代 `window.confirm` ——
   观感与面板一致，用户也不用再看到原生弹窗。

UIKit 缺失（未部署 `_lib`）时 `plugin_ui` 原样返回你的 HTML，老插件不受影响。

### 返回结构化数据（JSON）

所有调用的返回值只有**文本**一种形态（同步插件取 `zap.log()` 的内容）。想让页面
拿到结构化状态做复杂 UI（表格 / 页签 / 勾选列表），约定做法是**日志里输出单行 JSON**：

```lua
function on_repo(ctx)
  local staged, unstaged = collect_entries(ctx)
  -- 只有这一行日志，前端 JSON.parse 即可拿到对象 / 数组
  zap.log(zap.json_encode({ ok = true, branch = 'main', staged = staged, unstaged = unstaged }))
end
```

```js
async function callJson(action, extra) {
  const text = await zap.call(action, extra)
  return JSON.parse(text) // 前端再做容错：拿不到对象就当调用失败
}
```

注意两点：

- **该 action 里不要再 `zap.log` 其它内容**（比如 `zap.try_run` 的输出），混进去一行就 parse 不了；
  需要调试信息时单独加 key，不要另起一行 log。
- `zap.try_run` 失败时返回值里会带 `命令退出码 N:` 前缀，读取类调用要先判第一个返回值
  （`local ok, out = zap.try_run(...)`），别把失败输出当结果。

写好在这之上，一个 880px 弹窗内做多页签、勾选列表、表单设置都可以： Git 插件
（`data/appstore/repos/appstore/plugins/git`）就是这套写法的参考实现。它同时演示了
从 `file.editor` 槽位拿当前目录（`options.cwd`，缺失时父页面自动补文件管理器当前目录）、
以及文件列表用 JSON 数组字符串传值（避免空格 / 逗号的文件名被拆坏）。

### 完整示例

`main.lua`：

```lua
function on_scan(ctx)
  local dir = zap.path.site(zap.opt('TARGET', ''))
  if not zap.fs.is_dir(dir) then zap.fail('不是目录: ' .. dir) end
  for _, n in ipairs(zap.fs.list(dir)) do
    local full = zap.path.join(dir, n)
    zap.logf('%s  %8d  %s', zap.fs.is_dir(full) and 'dir ' or 'file', zap.fs.size(full), n)
  end
end

function on_du(ctx)
  local ok, out = zap.try_run('du', { '-sh', zap.path.site(zap.opt('TARGET', '')) })
  if not ok then zap.fail('du 失败: ' .. tostring(out)) end
  zap.log(zap.str.trim(out))
end
```

`ui.html`：

```html
<input id="target" placeholder="相对站点根目录" />
<button onclick="run('scan')">扫描</button>
<button onclick="run('du')">统计占用</button>
<pre id="out"></pre>
<script>
  async function run(action) {
    const out = document.getElementById('out')
    out.textContent = '执行中…'
    try {
      out.textContent = await zap.call(action, { TARGET: target.value })
    } catch (e) {
      out.textContent = '失败: ' + e.message
    }
  }
</script>
```

可运行的完整版见 `data/plugins/examples/html-demo/`。

### 什么时候用 HTML，什么时候用 options

| | `options` 结构化表单 | `ui.html` |
| --- | --- | --- |
| 输入 | 声明式，前端渲染 | 自己写 |
| 输出 | 纯文本日志 | 自己渲染（表格、图表、进度…） |
| 交互 | 填一次 → 跑一次 | 多步、多按钮、边跑边看 |
| 限制 | 表单控件有限 | 必须走 `zap.call`，不能直连后端 |

简单的一次性任务用 `options`；要做向导、仪表盘、实时进度这类东西用 `ui.html`。

---

## 7. 异步插件

```yaml
async: true
```

后台执行，日志实时落盘，前端用 SSE 订阅，可以随时取消。
插件侧轮询 `zap.canceled()` 提前收尾：

```lua
function on_run(ctx)
  for i = 1, 100 do
    if zap.canceled() then zap.log('已取消'); return end
    zap.logf('进度 %d%%', i)
  end
end
```

### action 级 async / dangerous

`async` 是**插件级**开关：开了之后连「读状态」这种毫秒级动作也要走 SSE。
读写混合的插件（比如 Git）只想把少数几个长操作异步化，这时用 action 级开关：

```yaml
actions:
  status: 状态                                     # 简写：只有文案
  push: { label: 推送, async: true }               # 只有 push 走后台 + 可取消
  fetch: { label: 获取, async: true }
  reset: { label: 回退, dangerous: true }          # 破坏性动作
```

| 开关 | 作用 |
| --- | --- |
| `async: true` | 该动作后台执行，日志走 SSE；前端 observing 时会显示取消按钮 |
| `dangerous: true` | 宿主执行前弹一次**危险操作确认**；只读演示账号（demo 角色）在后端被直接拒绝 |

两种写法可以混用（Git 插件的 manifest 就是混用的）。`dangerous` 的拦截放在服务端，
插件作者忘了在 UI 里加 `confirm` 也拦得住。

---

## 8. 安装与卸载

两种方式，落地结果一致（都是一个插件目录）：

1. **面板上传**（应用商店 → 插件 → 上传安装）：`.zip` / `.tar.gz` / `.tgz` / `.tar`，
   包根或**唯一的顶层目录**里需含 `manifest.yaml` + `main.lua`
2. **手工放置**：直接拷目录

插件名可留空，此时取 `manifest.yaml` 的 `name`；两者都给了必须一致。
同名插件已存在时需勾选「覆盖安装」。卸载 = 删除整个插件目录。

安装时把来源 / 分支 / 级别 / 安装时间写回插件自身的 `manifest.yaml`（顶层 `zap_install:` 块），
管理页据此显示来源。手工放置的插件没有这段信息，来源显示为「手动放置」。

### 依赖声明：requires

插件要用的**外部命令**必须先声明，安装时 `command -v` 逐个校验，缺哪个就拒装并列出：

```yaml
requires:
  commands: [git, curl]
```

比跑起来才报 `command not found` 好排查。命令名只允许字母数字与 `._-`。

### 签名：signature

manifest 里带 `signature` 时，安装会重算插件文件摘要并比对，对不上直接拒装：

```yaml
signature: 3f2a…（64 位十六进制）
```

摘要 = 所有文件（排除隐藏文件、`manifest.yaml` 本身）的 `<相对路径>:<sha256>` 逐行拼接，
再算 HMAC-SHA256（密钥为面板主密钥）。**注意这是本机完整性校验**：能挡住「插件目录被人
换了几行代码」，但密钥就在同一台机器上，跨机器分发请配合可信来源（应用商店仓库本身）。

不带 `signature` 的插件照常安装 —— 这是个可选项，老插件不受影响。

---

## 9. 安全边界

| 边界 | 说明 |
| --- | --- |
| Lua 沙箱 | 无 `io` / `os` / `package` / `debug`；文件只能经 `zap.fs.*` 按 scope 降权访问 |
| `scope: site` | `user_cmd` + `drop_privileges`（清附加组 → setgid → setuid）降到站点账号 |
| `scope: user` | 降到触发请求的面板用户账号（同 `site` 的降权路径）；管理员装的系统级插件被普通用户触发时也仅该用户权限 |
| `scope: system` | 仅管理员安装的插件可用（声明 `system` 的插件只能由管理员安装到系统目录） |
| 路径校验 | 安装解包挡 `..` 与绝对路径；卸载与 `ui.html` 读取都做 canonicalize 越界检查 |
| HTML 界面 | iframe 无 `allow-same-origin`，只能经 postMessage 由父页面代跑 |
| 权限点 | `plugin:view` / `plugin:run` / `plugin:install` / `plugin:uninstall` |

---

## 10. 调试

- 插件列表与错误原因会写进 zapexec 日志
- `plugin_list` 会跳过 manifest 解析失败的目录并记 warn —— 入口不出现先查日志
- `scope: site` 插件要求有站点上下文；少了会报「需要 site_root / site_linux_user」
- `scope: user` 插件要求调用方有 Linux 账号；`user` 字段为空会报「user 作用域插件需要调用方 Linux 账号」
- 快速验证 Lua 语法：库文件有编译期校验（`cargo test -p zapexec`），
  自己的 `main.lua` 可以先用 `luac -p main.lua` 过一遍

---

## 11. 通过应用商店分发（系统级插件）

除了「上传 / Git / 手工放置」三种本地安装方式，系统级插件也可以作为 **应用商店（AppStore）包** 发布，
让用户从「应用商店 → 插件」分类里一键安装。两条入口落地结果完全一致（都是一个插件目录），
原有上传安装功能不受影响。

### 包结构

在应用商店仓库（如 `data/appstore/repos/appstore/`）下新建 `plugins/<name>/`：

```
plugins/hello-plugin/
  app.yaml        # 市场元数据：卡片标题、描述、版本、分类（category: plugins）
  manifest.yaml   # 插件运行时清单（scope / ui / options / actions），与原生插件一致
  main.lua        # 插件逻辑（on_run / on_<action>）
  ui.html         # 可选，自带 HTML 界面
  install.sh      # 应用商店安装钩子：把目录落到 $ZAP_PATH/plugins/<name>
  uninstall.sh    # 应用商店卸载钩子：删除该插件目录
```

- `app.yaml` 负责「市场卡片」与安装约束；`manifest.yaml` 负责「插件怎么跑」，两者互不干涉。
- `category` 必须写 `plugins`（应用商店按此分类展示，见 `zapd/src/zap/appstore.rs` 的分类列表）。
- `app.yaml` 不声明 `provision` / `run_as`：插件不走建站编排、也不降权，运行身份由 `manifest.yaml` 的 `scope` 决定。

### install.sh（安装钩子）

由 zapexec 以 **root** 执行（系统级插件）。以下环境变量由应用商店注入：
`ZAP_PATH`（面板根）、`APP_NAME`（插件名）、`PKG_SRC_PATH`（仓库里本包源码目录）。

```sh
#!/bin/sh
set -e
DEST="$ZAP_PATH/plugins/$APP_NAME"
rm -rf "$DEST"
mkdir -p "$DEST"
cp -r "$PKG_SRC_PATH/." "$DEST/"
# 去掉应用商店专属编排文件，保持插件目录干净
rm -f "$DEST/app.yaml" "$DEST/install.sh" "$DEST/uninstall.sh"
echo "插件 $APP_NAME 已安装到 $DEST"
```

`uninstall.sh` 只需 `rm -rf "$ZAP_PATH/plugins/$APP_NAME"`。应用商店在卸载成功后
还会自动清掉自己的安装元数据（`apps/plugins/<name>/default/meta.yaml`），不会留下脏数据。

### 权限与可见性

- 系统级插件 **仅管理员** 可从市场安装（与 `app.yaml` 的 `roles` 白名单规则一致；
  缺省即仅 admin，要在市场里开放给普通用户可加 `roles: [user]`，此时插件落到 `$ZAP_PATH/plugins`
  仍由后端权限点 `plugin:install` 把关）。
- 安装完成后，`/plugin/list` 会自动扫到 `$ZAP_PATH/plugins/<name>`，站点详情「插件」标签页
  照常渲染入口（含 `ui.html` 沙箱界面）—— 应用商店只是「分发渠道」，运行时仍是 Lua 引擎。

### 升级

应用商店的升级复用同一套 `install.sh`：把新版本仓库内容重新拷到 `$ZAP_PATH/plugins/<name>`
覆盖即可；也可在 `app.yaml` 里提供 `upgrade.sh` 做差异化升级。

> 完整可运行示例见 `data/appstore/repos/appstore/plugins/hello-plugin/`
> （HTTP 界面演示插件，安装后会以 `site.detail` 槽位出现在站点详情）。

---

## 12. 定时 / Webhook 触发

插件默认只在用户点按钮时跑。要周期性执行（每天半夜 `git fetch`）或让外部系统拉起来
（CI 推完自动更新），在**插件管理页 → 该插件的「触发器」**里加一条，不用改插件代码。

| 类型 | 怎么触发 | 适合 |
| --- | --- | --- |
| `cron` | 五段 cron（分 时 日 月 周），调度器每 30 秒轮询一轮，按服务器本地时区 | 备份、同步、清理 |
| `webhook` | 外部 POST `<面板地址>/api/plugin/hook/<随机令牌>` | CI 通知、Git 平台回调 |

几条约束：

- **身份归创建者**：定时器不代表 root 或匿名用户，它就是你 —— 用你的 Linux 账号、
  你的角色（也因此 你能手动点的按钮，定时才跑得动；dangerous 动作对只读演示账号依然被拒）。
- `site` 作用域的插件需要在触发器里填**站点 ID**，否则拿不到站点根。
- 别人看不到你的触发器；密钥轮换后立即失效。
- 执行结果（上次时间 / 成功失败）列在同一张表里，不用翻日志。

Webhook 那条路径在权限矩阵里是**公开**的（带 `zap_` 式的随机令牌鉴权，不走面板 JWT），
令牌泄露就立刻轮换。

---

## 13. 冒烟测试与 CI

在插件目录放一份 `tests.yaml`，就能把「每个 action 都没坏」变成一键可查：

```yaml
- action: status              # 缺省 run；会回落到 on_run 按 ctx.action 分发
  options: { cwd: '.' }
  user: www                   # scope: user / site 必须给出降权账号（不能是 root）
  site_root: /var/www/site    # scope: site 还需要站点根（userid 缺失会跳过而不是误判失败）
  expect:
    contains: [branch]        # 日志里必须出现
    not_contains: [fatal]     # 不许出现
```

触发方式：插件管理页里的测试（或直接在 `zapd` 侧调 `/plugin/test`）。
返回每条用例的 `ok`（`null` = 跳过）与实际日志，断言用的是**日志文本**，
所以「只输出 JSON 的 action」要挑一个一定会出现的字段来断言。
