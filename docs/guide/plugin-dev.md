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

两种方式，落地结果一致（都是一个插件目录）：

1. **面板上传**（应用商店 → 插件 → 上传安装）：`.zip` / `.tar.gz` / `.tgz` / `.tar`，
   包根或**唯一的顶层目录**里需含 `manifest.yaml` + `main.lua`
2. **手工放置**：直接拷目录

插件名可留空，此时取 `manifest.yaml` 的 `name`；两者都给了必须一致。
同名插件已存在时需勾选「覆盖安装」。卸载 = 删除整个插件目录。

安装时把来源 / 分支 / 级别 / 安装时间写回插件自身的 `manifest.yaml`（顶层 `zap_install:` 块），
管理页据此显示来源。手工放置的插件没有这段信息，来源显示为「手动放置」。

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
