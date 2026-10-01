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

- **系统级**：`$ZAP_PATH/plugins/<name>/` —— 所有用户可见，**仅管理员**可安装
- **用户级**：`$HOME/.zap/plugins/<name>/` —— 仅本人可见，登录用户自己就能装

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
scope: site            # site = 以站点账号运行；system = 以 root 运行（仅系统级插件）

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

把目录拷到 `$HOME/.zap/plugins/hello/`（或 `$ZAP_PATH/plugins/hello/`），
打开「站点详情 → 插件」就能看到入口。

---

## 2. manifest.yaml 完整字段

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | ✓ | 插件标识，**只允许字母数字、下划线、连字符**，必须与目录名一致 |
| `title` | | 展示标题，缺省用 `name` |
| `version` / `description` / `author` / `homepage` | | 元信息，管理页展示 |
| `scope` | | `site`（默认可用，以站点 Linux 账号运行）/ `system`（以 root 运行）。**用户级插件不允许 `system`** |
| `async` | | `true` 时后台执行，日志走 SSE 实时回传，前端可取消 |
| `ui.placement` | ✓ | 入口挂载位置，见下表 |
| `ui.label` | | 入口按钮文案 |
| `ui.icon` | | Iconify 图标名 |
| `ui.tab` | | 补充说明（显示在卡片副标题） |
| `ui.html` | | 自带 HTML 界面的文件名（插件目录内的单个 `.html`） |
| `options` | | 结构化表单，见 §4 |
| `actions` | | `action: 文案` 映射，缺省用第一个作按钮文案 |

### placement（挂载位置）

| 值 | 位置 |
| --- | --- |
| `site.detail` | 站点详情抽屉 →「插件」标签页（当前唯一内置槽位） |

过滤是**字符串相等**匹配：插件不区分站点上装了什么应用，只按槽位出现。

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
| `zap.run(prog, {args})` | **按 scope 自动分派**：`site` 走站点账号，`system` 走 root |
| `zap.try_run(prog, {args})` | 同上，失败不抛错，返回 `(ok, output)` |
| `zap.exec(prog, {args})` | 强制 root（仅 `scope: system`） |
| `zap.exec_as_user(prog, {args})` | 强制站点账号（仅 `scope: site`） |
| `zap.site_root()` / `zap.site_linux_user()` | 站点文档根 / 运行账号（`scope: site`） |
| `zap.home_dir()` | 当前执行身份的家目录 |
| `zap.plugin_dir()` | 插件自身目录（读自带资源） |
| `zap.scope()` / `zap.level()` | `"site"\|"system"` / `"user"\|"system"` |
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
2. `$HOME/.zap/plugins/_lib/` —— 用户级公共库
3. `<plugin_dir>/lib/` —— 插件自带的私有库

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
```

调用链路：

```
ui.html ──postMessage──▶ 父页面 ──/plugin/run（带真实 JWT）──▶ zapexec ──▶ main.lua
       ◀──postMessage──         ◀──────── 输出 / 日志 ────────────────┘
```

父页面只认**自己那个 iframe** 发来的消息（`ev.source === iframe.contentWindow`），
并会按 `action` 把请求转发给 `on_<action>`。权限点（`plugin:run`）与 `scope` 降权
全在后端，绕过不了。

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

---

## 8. 安装与卸载

三种方式，落地结果一致（都是一个插件目录）：

1. **面板上传**（开发 → 插件 → 上传安装）：`.zip` / `.tar.gz` / `.tgz` / `.tar`，
   包根或**唯一的顶层目录**里需含 `manifest.yaml` + `main.lua`
2. **面板 Git 安装**（开发 → 插件 → Git 安装）：`http(s)://` / `ssh://` / `git@`，
   可选分支或标签（内部走 `git clone --depth 1`）
3. **手工放置**：直接拷目录

插件名可留空，此时取 `manifest.yaml` 的 `name`；两者都给了必须一致。
同名插件已存在时需勾选「覆盖安装」。卸载 = 删除整个插件目录。

安装后目录里会写入 `.zap-install.json`（来源 / 分支 / 级别 / 时间），
管理页据此显示来源。手工放置的插件没有它，显示为「手动放置」。

---

## 9. 安全边界

| 边界 | 说明 |
| --- | --- |
| Lua 沙箱 | 无 `io` / `os` / `package` / `debug`；文件只能经 `zap.fs.*` 按 scope 降权访问 |
| `scope: site` | `user_cmd` + `drop_privileges`（清附加组 → setgid → setuid）降到站点账号 |
| `scope: system` | 仅系统级插件可用；**用户级插件声明 `system` 会在安装和运行两处被拒** |
| 路径校验 | 安装解包挡 `..` 与绝对路径；卸载与 `ui.html` 读取都做 canonicalize 越界检查 |
| HTML 界面 | iframe 无 `allow-same-origin`，只能经 postMessage 由父页面代跑 |
| 权限点 | `plugin:view` / `plugin:run` / `plugin:install` / `plugin:uninstall` |

---

## 10. 调试

- 插件列表与错误原因会写进 zapexec 日志
- `plugin_list` 会跳过 manifest 解析失败的目录并记 warn —— 入口不出现先查日志
- `scope: site` 插件要求有站点上下文；少了会报「需要 site_root / site_linux_user」
- 快速验证 Lua 语法：库文件有编译期校验（`cargo test -p zapexec`），
  自己的 `main.lua` 可以先用 `luac -p main.lua` 过一遍
