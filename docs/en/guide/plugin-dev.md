---
title: Plugin Development
---

# Plugin Development Guide

A ZAP plugin is **three files in one directory**: logic is written in Lua and runs inside the Lua sandbox embedded
in `zapexec` (the root daemon). Install it and it works — no compilation, no panel restart.

```
my-plugin/
├── manifest.yaml   description + frontend entry metadata (required)
├── main.lua        logic entry point on_run(ctx) / on_<action>(ctx) (required)
├── ui.html         optional, ships its own HTML interface (pointed at by `ui.html` in the manifest)
└── lib/*.lua       optional, plugin-private function library (auto-loaded)
```

- **System level**: `$ZAP_PATH/plugins/<name>/` — visible to all users, **installable by admins only**
  (regular users can use plugins but cannot install them)

Directories starting with `.` or `_` (`_lib`, `.git`) are not treated as plugins.

---

## 1. Five-minute quick start

`manifest.yaml`:

```yaml
name: hello
title: Say hello
version: 1.0.0
description: The smallest runnable plugin
author: you
scope: user            # user = run as the calling panel user's account (not bound to a site); site = as the site account; system = as root (system level only)

ui:
  placement: site.detail   # appears on the "Site detail → Plugins" tab
  label: Say hello
  icon: material-symbols:waving-hand

options:
  - name: NAME
    label: Name
    type: string
    default: world
    required: true
```

`main.lua`:

```lua
function on_run(ctx)
  local name = zap.opt('NAME', 'world')
  zap.log('hello, ' .. name)
  zap.log('running in: ' .. zap.path.site(''))
end
```

Copy the directory to `$ZAP_PATH/plugins/hello/` (writable by admins only) and the entry appears under
"Site detail → Plugins".

### Non-site plugins: `scope: user`

Some plugins are not tied to a site at all (for example managing "the current panel user's" own SSH keys, cron jobs,
or tools in their home directory). Those should use `scope: user`:

```yaml
name: my-keys
title: My Keys
scope: user            # runs as the panel user who triggered the request; no site needed
ui:
  placement: site.detail
  label: My Keys
```

```lua
function on_run(ctx)
  local me = zap.home_dir()              -- the panel user's home directory
  zap.log('current user home: ' .. me)
  zap.run('ls', { '-la', me })           -- automatically run as that user
end
```

Key points:

- `scope: user` **does not depend on site context**: entries can be mounted on any slot, and `site_root` is not
  required at runtime.
- The run identity is the Linux account of **the panel user who triggered the request** — even for a system-level
  plugin installed by an admin, when a regular user triggers it that user's permissions apply; there is no privilege
  escalation.
- `zap.site_root()` / `zap.site_linux_user()` return empty strings under `scope: user`;
  use `zap.home_dir()` for the user's home directory, not `zap.path.site(...)` (which joins onto the site root).
- `zap.exec` (root) is forbidden under `scope: user`; always run commands via `zap.run` / `zap.exec_as_user`.

---

## 2. Complete `manifest.yaml` fields

| Field | Required | Description |
| --- | --- | --- |
| `name` | ✓ | Plugin identifier — **letters, digits, underscores and hyphens only**; must match the directory name |
| `title` | | Display title; falls back to `name` |
| `version` / `description` / `author` / `homepage` | | Metadata shown on the management page |
| `scope` | | `site` (run as the site's Linux account, requires site context) / `user` (run as the calling panel user's account, not bound to a site) / `system` (run as root; only for plugins installed by an admin) |
| `async` | | when `true`, execute in the background, stream logs back over SSE, and allow the frontend to cancel |
| `ui.placement` | ✓ | Where the entry is mounted — **a single slot or an array of slots** (the old single-string form still works) |
| `ui.label` | | Entry button text |
| `ui.icon` | | Iconify icon name |
| `ui.tab` | | Supplementary note (shown as the card subtitle) |
| `ui.html` | | File name of the bundled HTML interface (a single `.html` inside the plugin directory) |
| `options` | | Structured form, see §4 |
| `actions` | | `action: text` mapping; defaults to the first one as the button text. Can also be an object with switches, see §7 |
| `requires.commands` | | External commands the plugin depends on — installation is **refused** if any is missing, see §8 |
| `signature` | | Digest of the plugin files (HMAC-SHA256); installation is refused if files were modified, see §8 |
| `i18n` | | (optional) override texts per panel language; without it the base language is always shown — see "Localization" below |

### placement (mount location)

| Value | Location |
| --- | --- |
| `site.detail` | Site detail drawer → "Plugins" tab |
| `file.editor` | File editor overlay / file manager top toolbar (`mode="toolbar"`; the host passes the current directory as `options.cwd`) |
| `file.context` | File manager list right-click menu (`mode="context"`; additionally passes the selected files as `options.files`, a JSON array string) |
| `dashboard.card` | A card at the bottom of the dashboard (admin home); the whole block is hidden when no plugin is mounted |

Filtering matches "any slot hits": plugins are not filtered by which app a site has installed, only by slot.
One plugin can mount in several places:

```yaml
ui:
  placement:
    - file.editor
    - file.context
```

The host only renders slots it knows; an unknown slot written by an older plugin is not an error (it just does not show).

### Localization (i18n, optional)

All fields above are in the **base language** (whatever the author wrote, usually Chinese) — **a plugin may ship
without any translation at all**, in which case that base text is shown regardless of the panel language; this is
not treated as an error and nothing renders blank. Add an `i18n` table only if you want it to follow the panel language:

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

Rules:

- **Optional and incremental**: with no `i18n` table, or no entry for the current language (e.g. only `zh-CN` is
  provided and the user switches to English), it falls back to the base text. `en-US` is not mandatory and you do not
  have to translate everything at once — translating just the title is a valid intermediate state.
- Language keys match `zh-CN` → `en-US`, with regional variants falling back (`en-GB` also matches `en-US`).
- Overridable fields: `title` / `description` / `ui.label` / `ui.tab` / `actions.<name>` /
  `options[].label|desc|placeholder`. **Structure** (`name`, `type`, `choices`, scope, …) always comes from the base
  fields — translation tables only carry text, so the two languages cannot silently drift apart structurally.
- An action name in a translation table that does not exist in the base `actions` is a typo and will never be visible
  in the panel (CI reports it as an error). (Note this check only runs when you **do** provide a translation table;
  provide none and nothing is checked.)
- The panel language is **the same one used by Element Plus** (switch it in the top-right corner); `/plugin/list`
  receives the current language, and switching languages makes the host re-fetch the list and reload any open plugin
  interface.

A bundled HTML interface can follow along too (also **optional**: a plugin that ignores `zap.ui.lang` simply keeps
showing its own text): the host writes `window.__ZAP_LANG__` before injecting the interface, and the UIKit provides
`zap.ui.lang` and `zap.ui.t()` (see §6).

---

## 3. `main.lua`: the global `zap` table

The sandbox enables only `table` / `string` / `math` / `utf8` / `coroutine` — **no `io` / `os` / `package` / `debug`**.
The reason is concrete: plugins run inside the `zapexec` process, so exposing `io` would let them read and write any
file as root, bypassing the `scope: site` privilege drop (which only applies to child processes). To read files,
always go through `zap.fs.*`.

### Entry points

```lua
function on_run(ctx) end        -- default entry
function on_scan(ctx) end       -- called first when action=scan (used by HTML interfaces)
```

`ctx` is `{ action, scope, level }`. `action` defaults to `run`;
when `on_<action>` does not exist it falls back to `on_run`.

### Core capabilities

| Call | Description |
| --- | --- |
| `zap.log(msg)` | Write a log line, returned to the frontend |
| `zap.logf(fmt, ...)` | `string.format` then log |
| `zap.option(name)` | Read an option |
| `zap.run(prog, {args})` | **Dispatched automatically by scope**: `site` → site account, `user` → panel user account, `system` → root |
| `zap.try_run(prog, {args})` | Same, but does not raise on failure; returns `(ok, output)` |
| `zap.exec(prog, {args})` | Force root (only under `scope: system`) |
| `zap.exec_as_user(prog, {args})` | Force a drop to the run account: `site` → site account, `user` → panel user account |
| `zap.site_root()` / `zap.site_linux_user()` | Site document root / site run account (only when `scope=site`; empty otherwise) |
| `zap.home_dir()` | Home directory of the current identity (`site` = site account home / `user` = panel user home / `system` = caller home) |
| `zap.plugin_dir()` | The plugin's own directory (read bundled resources) |
| `zap.scope()` / `zap.level()` | `"site"\|"user"\|"system"` / `"user"\|"system"` |
| `zap.canceled()` | Async plugins poll this to detect that the user clicked cancel |
| `zap.read_file(p)` / `zap.write_file(p,s)` / `zap.append_file(p,s)` | Read/write files with the scope's privilege drop |
| `zap.json_encode(v)` / `zap.json_decode(s)` | JSON |
| `zap.time()` / `zap.date(fmt[, ts])` | Time |
| `zap.env(name)` | Environment variables (only `ZAP_*` plus `PATH/HOME/USER/SHELL/LANG/TMPDIR` are exposed) |

A failing command raises and terminates the plugin; anything already passed to `zap.log` is still returned.

---

## 4. `options`: structured forms

```yaml
options:
  - name: TARGET
    label: Target directory
    type: dir              # relative to the site root; empty = the root itself
    default: ''
    desc: The script works inside this directory
  - name: MODE
    label: Mode
    type: select
    default: fast
    choices:
      - { label: Fast, value: fast }
      - { label: Full, value: full }
  - name: EXTRA
    label: Extra arguments
    type: string
    placeholder: '--no-dev'
  - name: FORCE
    label: Overwrite
    type: bool
    default: 'false'
  - name: TAGS
    label: Tags
    type: multiselect      # multi-select, joined into one string with spaces
    choices: [a, b, c]
  - name: FILES
    label: Files
    type: files            # multi-select files; empty = none selected
```

| `type` | Widget |
| --- | --- |
| `string` / `number` | Input box |
| `bool` | Switch |
| `select` | Single-select dropdown (values must appear in `choices`) |
| `multiselect` | Multi-select dropdown, joined with spaces |
| `dir` | Directory picker (starting from the site root) |
| `file` / `files` | File picker (single / multiple) |

`dir` and `file(s)` return a path **relative to the site root** (absolute only when there is no site);
on the plugin side use `zap.path.site(rel)` to join it back into an absolute path.

---

## 5. Shared function library (auto-loaded)

Before `main.lua`, zapexec auto-loads `*.lua` from these directories in order — **no `require` needed**:

1. `$ZAP_PATH/data/plugins/_lib/` — system-level shared libraries (`zap.lua` ships with the release package)
2. `<plugin_dir>/lib/` — the plugin's own private libraries

The latter can override the former.

| Category | Functions |
| --- | --- |
| Logging / assertions | `zap.logf` `zap.assert` `zap.fail` |
| Strings | `zap.str.trim` `.blank` `.split` `.join` `.starts` `.ends` `.contains` `.lines` |
| Paths | `zap.path.join` `.dirname` `.basename` `.ext` `.is_abs` `.normalize` `.within` `.site` |
| Tables | `zap.tbl.keys` `.values` `.count` `.map` `.filter` `.contains` `.index_of` `.merge` |
| File system | `zap.fs.exists` `.is_dir` `.is_file` `.read` `.write` `.append` `.mkdir` `.remove` `.copy` `.move` `.chmod` `.list` `.read_lines` `.size` `.append_line` `.grep` |
| Shell | `zap.shell_quote` (alias `zap.q`) |
| Options | `zap.opt` `.opt_required` `.opt_bool` `.opt_number` `.opt_list` |
| Persistent config | `zap.config.get` `.set` `.number` `.bool` (see below) |
| Network | `zap.http.request` `.get` `.post` `.json` `.download` (alias `zap.download`, see below) |
| Misc | `zap.to_json` `.from_json` `.now` `.fmt_time` `.is_site_scope` |
| Top-level aliases | `zap.split` `zap.trim` `zap.join` `zap.q` |

`zap.fs.*` is entirely built on `zap.run`, so it **inherits the scope's privilege drop** — under `scope: site` it never
touches files as root.

```lua
function on_run(ctx)
  local name = zap.opt_required('NAME')
  local dir  = zap.path.site(zap.opt('TARGET', ''))   -- join the site root and verify it stays inside
  if not zap.fs.is_dir(dir) then zap.fs.mkdir(dir) end
  zap.fs.write(zap.path.join(dir, name .. '.txt'), 'hello\n')
  zap.logf('wrote %d bytes to %s', #('hello\n'), dir)
end
```

### Persistent configuration (KV)

The plugin directory is owned by root and the Lua side has no `io`, so "remember the branch the user picked last time"
or "store a token" needs a managed drawer: `zap.config.*`. It buckets by **plugin + panel user**
(`$ZAP_PATH/data/plugins/config/<plugin>.yaml`), so nobody else sees your config and the same plugin keeps a separate
copy per user.

```lua
function on_run(ctx)
  local last = zap.config.get('branch', 'main')   -- returns the default when never stored
  zap.config.set('branch', ctx.options.BRANCH or 'main')
  zap.config.set('token', nil)                    -- passing nil deletes the key
  if zap.config.bool('auto_push', false) then … end
end
```

Each key is capped at 8 KB; the mechanism suits preferences and small credentials — do not use it as a database.

### Network requests

There is no socket in the sandbox; all networking goes through the host's `curl` (which **inherits the scope's
privilege drop**, so under `scope: site` requests are not sent as root):

```lua
-- GET / POST: returns (status, body)
local code, body = zap.http.get('https://api.example.com/v1/ping')
local code, body = zap.http.post('https://api.example.com/v1/deploy', { env = 'prod' })  -- tables become JSON

-- Full options: method / headers / body / form / json / timeout / follow / insecure
local code, body = zap.http.request{
  url = 'https://api.example.com/v1/hook',
  method = 'POST',
  headers = { Authorization = 'Bearer ' .. token, 'X-Trace: ' .. id },
  json = { ref = 'main' },
  timeout = 15,
}

-- Get a parsed table directly: returns (table, status, raw)
local data, code = zap.http.json('GET', 'https://api.example.com/v1/status')

-- Download a file (parent directories are created automatically)
local ok, err = zap.http.download('https://example.com/app.tar.gz', '/tmp/app.tar.gz')
```

Network failures return `(nil, error message)`; **only `http://` and `https://` are accepted** — any other scheme is
rejected outright (so nobody can loop back into the filesystem via something like `file://`).

---

## 6. Bundled HTML interface (interaction / input handling / return values)

A plugin **can** render its own HTML interface and interact with the user bidirectionally. Declare `ui.html` and the
frontend loads your page instead of rendering the structured form.

```yaml
ui:
  placement: site.detail
  label: Directory checkup
  icon: material-symbols:monitor-heart
  html: ui.html        # a single .html file inside the plugin directory
```

### How it runs

Your HTML is placed in an iframe with `sandbox="allow-scripts"` (**without `allow-same-origin`**), so it has an
**opaque origin**:

- It cannot touch the panel's DOM / cookies / localStorage
- It cannot send credentialed requests or obtain the user's JWT
- To call the backend it must use the `postMessage` bridge below

The panel injects `window.zap` automatically:

```js
// Call the plugin; returns Promise<output text>
await zap.call(action, options)

// Shorthand, equivalent to zap.call('run', options)
await zap.run(options)

// A third argument (callback) enables streaming logs (async plugins push line by line)
await zap.call('du', { TARGET: 'public' }, (line) => {
  document.getElementById('out').textContent += line + '\n'
})

// Plugin-level persistent config (same data as zap.config on the Lua side)
const branch = await zap.configGet('branch')   // null when never stored
await zap.configSet('branch', branch || 'main') // null / '' deletes the key

// Open the panel's directory / file picker
await zap.pickDir({ startPath: '' })
```

Call chain:

```
ui.html ──postMessage──▶ parent page ──/plugin/run (with the real JWT)──▶ zapexec ──▶ main.lua
       ◀──postMessage──         ◀──────── output / logs ────────────────┘
```

The parent page only accepts messages from **its own iframe** (`ev.source === iframe.contentWindow`) and forwards
requests to `on_<action>` based on `action`. The permission point (`plugin:run`) and the `scope` privilege drop all
live in the backend and cannot be bypassed.

### UIKit: host-injected styles and `zap.ui.*`

The iframe is isolated: it cannot reach Element Plus and does not inherit the panel theme, so every plugin would have
to re-implement buttons / tables / tabs and end up looking different. To fix this, the host **automatically injects**
a UIKit when returning your HTML (`data/plugins/_lib/ui.css` + `ui.js`, deployed under
`$ZAP_PATH/data/plugins/_lib/`):

1. **Component styles**: a set of CSS variables (`--zap-primary`, …) and `.zui-*` classes. Just write the classes,
   no layout work needed:

   ```html
   <div class="zui-row">
     <button class="zui-btn">Save</button>
     <button class="zui-btn ghost">Cancel</button>
     <span class="zui-badge ok">Clean</span>
   </div>
   <div class="zui-tabs"><button class="zui-tab is-active">Status</button></div>
   <div class="zui-panel is-active">…</div>
   <table class="zui-table">…</table>
   <pre class="zui-pre">Output</pre>
   ```

   Common classes: buttons (`.zui-btn` + `.ghost / .danger / .sm / .link`), forms
   (`.zui-input / .zui-select / .zui-textarea`), layout (`.zui-row / .zui-grow / .zui-spacer / .zui-section /
   .zui-hr`), tabs (`.zui-tabs / .zui-tab / .zui-panel`), tables (`.zui-table-wrap / .zui-table`), lists
   (`.zui-list / .zui-item / .zui-path`), feedback (`.zui-badge / .zui-tip / .zui-warn / .zui-empty / .zui-banner`).

2. **Runtime `zap.ui.*`**:

   | Call | Description |
   | --- | --- |
   | `zap.ui.notify(msg, type)` | Panel notification bar (`success / warning / error / info`); degrades to an in-page toast when the host does not take over |
   | `zap.ui.confirm(msg, { danger, title })` | Panel confirm dialog, `Promise<boolean>` |
   | `zap.ui.prompt(msg, def, { title })` | Panel input prompt, `Promise<string \| null>` (null when cancelled) |
   | `zap.ui.toast(msg, type)` | In-page toast only (does not disturb the host) |
   | `zap.ui.table(el, spec)` | Render a table from columns: `{ columns: [{key,label,width,render}], rows, onRowClick, empty }` |
   | `zap.ui.tabs(el, list, onChange)` | Render tabs wired to `[data-panel]` panels; returns `{ select, active }` |
   | `zap.ui.diff(el, text, isErr)` | Fill a `pre` with a diff, colouring `+ / - / @@` lines |
   | `zap.ui.busy(root, on, text)` | Lock / unlock buttons to prevent double-clicks launching concurrent tasks |
   | `zap.ui.el(tag, attrs, children)` / `zap.ui.escape(s)` | Create elements, escape text |
   | `zap.ui.lang` | Current panel language (`zh-CN` / `en-US`), same as Element Plus |
   | `zap.ui.t({ ['zh-CN'] = 'Status', ['en-US'] = 'Status' })` | Pick a string for the current language; with a second argument `zap.ui.t(textTable, 'en-US')` forces one language |
   | `zap.ui.localize(dict)` | Batch mode: mark nodes with `data-i18n="run"` and pass `{ run = { ['zh-CN']='Run', ['en-US']='Run' } }` to rewrite text automatically (`data-i18n-placeholder` rewrites placeholders) |

   For example, replace `alert` with the host's notification bar and `window.confirm` with `zap.ui.confirm`: the look
   matches the panel and users no longer see native dialogs.

When the UIKit is missing (`_lib` not deployed), `plugin_ui` returns your HTML untouched — older plugins are unaffected.

### Returning structured data (JSON)

Every call returns **text only** (a synchronous plugin returns the content passed to `zap.log()`). To let the page
receive structured state for a richer UI (tables / tabs / checkbox lists), the convention is to **log a single line
of JSON**:

```lua
function on_repo(ctx)
  local staged, unstaged = collect_entries(ctx)
  -- this one log line only; the frontend JSON.parse's it into an object / array
  zap.log(zap.json_encode({ ok = true, branch = 'main', staged = staged, unstaged = unstaged }))
end
```

```js
async function callJson(action, extra) {
  const text = await zap.call(action, extra)
  return JSON.parse(text) // add fault tolerance: treat a non-object as a failed call
}
```

Two caveats:

- **Do not `zap.log` anything else in that action** (such as the output of `zap.try_run`) — one extra line and parsing
  fails. When you need debug info, add another key instead of another log line.
- When `zap.try_run` fails its return value is prefixed with `command exit code N:` — for read-only calls check the
  first return value first (`local ok, out = zap.try_run(...)`) and do not treat failure output as a result.

Built on top of this you can do multi-tab layouts, checkbox lists and form settings inside an 880px dialog: the Git
plugin (`data/appstore/repos/appstore/plugins/git`) is the reference implementation. It also demonstrates reading the
current directory from the `file.editor` slot (`options.cwd`, filled in by the parent page from the file manager when
missing) and passing file lists as a JSON array string (so filenames with spaces or commas are not mangled).

### Complete example

`main.lua`:

```lua
function on_scan(ctx)
  local dir = zap.path.site(zap.opt('TARGET', ''))
  if not zap.fs.is_dir(dir) then zap.fail('not a directory: ' .. dir) end
  for _, n in ipairs(zap.fs.list(dir)) do
    local full = zap.path.join(dir, n)
    zap.logf('%s  %8d  %s', zap.fs.is_dir(full) and 'dir ' or 'file', zap.fs.size(full), n)
  end
end

function on_du(ctx)
  local ok, out = zap.try_run('du', { '-sh', zap.path.site(zap.opt('TARGET', '')) })
  if not ok then zap.fail('du failed: ' .. tostring(out)) end
  zap.log(zap.str.trim(out))
end
```

`ui.html`:

```html
<input id="target" placeholder="Relative to the site root" />
<button onclick="run('scan')">Scan</button>
<button onclick="run('du')">Disk usage</button>
<pre id="out"></pre>
<script>
  async function run(action) {
    const out = document.getElementById('out')
    out.textContent = 'Running…'
    try {
      out.textContent = await zap.call(action, { TARGET: target.value })
    } catch (e) {
      out.textContent = 'Failed: ' + e.message
    }
  }
</script>
```

A complete runnable version lives in `data/plugins/examples/html-demo/`.

### When to use HTML, when to use options

| | `options` structured form | `ui.html` |
| --- | --- | --- |
| Input | declarative, rendered by the frontend | you write it |
| Output | plain-text log | you render it (tables, charts, progress, …) |
| Interaction | fill once → run once | multi-step, multiple buttons, watch while it runs |
| Constraint | limited set of form widgets | must go through `zap.call`; cannot reach the backend directly |

Use `options` for simple one-shot tasks; use `ui.html` for wizards, dashboards and live progress.

---

## 7. Async plugins

```yaml
async: true
```

Execution moves to the background, logs are written to disk in real time, the frontend subscribes via SSE and can
cancel at any time. On the plugin side, poll `zap.canceled()` to finish early:

```lua
function on_run(ctx)
  for i = 1, 100 do
    if zap.canceled() then zap.log('cancelled'); return end
    zap.logf('progress %d%%', i)
  end
end
```

### Action-level `async` / `dangerous`

`async` is a **plugin-level** switch: once enabled, even millisecond-scale "read status" actions go through SSE.
Read/write-mixed plugins (Git, for example) want only a few long operations to be async — use the action-level switch:

```yaml
actions:
  status: Status                                     # shorthand: text only
  push: { label: Push, async: true }                 # only push runs in the background and can be cancelled
  fetch: { label: Fetch, async: true }
  reset: { label: Reset, dangerous: true }           # destructive action
```

| Switch | Effect |
| --- | --- |
| `async: true` | That action runs in the background with logs over SSE; the frontend shows a cancel button while observing |
| `dangerous: true` | The host asks for a **dangerous operation confirmation** before running; read-only demo accounts (the demo role) are rejected outright in the backend |

The two forms can be mixed (the Git plugin's manifest mixes them). `dangerous` is enforced server-side, so it still
holds even if the plugin author forgets a `confirm` in the UI.

---

## 8. Installation and uninstallation

Two approaches with the same result (a plugin directory):

1. **Upload from the panel** (App Store → Plugins → Upload & install): `.zip` / `.tar.gz` / `.tgz` / `.tar`; the
   package root or its **single top-level directory** must contain `manifest.yaml` + `main.lua`
2. **Manual placement**: copy the directory directly

The plugin name may be left empty, in which case `name` from `manifest.yaml` is used; if both are given they must match.
Installing over an existing plugin of the same name requires ticking "Overwrite". Uninstalling = deleting the whole
plugin directory.

On installation, source / branch / level / install time are written back into the plugin's own `manifest.yaml`
(a top-level `zap_install:` block), and the management page shows the source from it. Manually placed plugins have no
such block and show "manually placed" as their source.

### Dependency declaration: `requires`

**External commands** the plugin uses must be declared; installation validates each with `command -v` and refuses
with a list of what is missing:

```yaml
requires:
  commands: [git, curl]
```

Much easier to diagnose than a `command not found` at run time. Command names allow letters, digits and `._-` only.

### Signing: `signature`

When the manifest carries `signature`, installation recomputes the digest of the plugin files and compares it;
a mismatch refuses the installation:

```yaml
signature: 3f2a… (64 hex characters)
```

The digest is `<relative path>:<sha256>` for every file (excluding hidden files and `manifest.yaml` itself), joined
line by line, then HMAC-SHA256 with the panel master key as the secret. **Note this is a local integrity check**: it
stops "someone swapped a few lines in the plugin directory", but the key lives on the same machine, so for
cross-machine distribution combine it with a trustworthy source (the App Store repository itself).

Plugins without a `signature` install normally — it is optional and older plugins are unaffected.

---

## 9. Security boundaries

| Boundary | Description |
| --- | --- |
| Lua sandbox | No `io` / `os` / `package` / `debug`; files are reachable only via `zap.fs.*` with the scope's privilege drop |
| `scope: site` | `user_cmd` + `drop_privileges` (clear supplementary groups → setgid → setuid) drops to the site account |
| `scope: user` | Drops to the panel user account that triggered the request (same drop path as `site`); a system-level plugin installed by an admin still only has that user's rights when a regular user triggers it |
| `scope: system` | Only available to plugins installed by an admin (plugins declaring `system` can only be installed into the system directory by an admin) |
| Path validation | Installing/extracting blocks `..` and absolute paths; uninstallation and `ui.html` reads both canonicalize and check for escapes |
| HTML interface | The iframe has no `allow-same-origin`, so it can only be proxied by the parent page via postMessage |
| Permission points | `plugin:view` / `plugin:run` / `plugin:install` / `plugin:uninstall` |

---

## 10. Debugging

- The plugin list and the reason for any error are written to the zapexec log
- `plugin_list` skips directories whose manifest fails to parse and logs a warning — if an entry does not appear,
  check the log first
- `scope: site` plugins require site context; without it they report "site_root / site_linux_user required"
- `scope: user` plugins require the caller to have a Linux account; an empty `user` field reports
  "user-scoped plugins require a caller Linux account"
- To validate Lua syntax quickly: library files are checked at compile time (`cargo test -p zapexec`), and you can
  run your own `main.lua` through `luac -p main.lua` first

---

## 11. Distribution via the App Store (system-level plugins)

Besides the three local installation methods (upload / Git / manual placement), system-level plugins can also be
published as **App Store packages**, letting users install them in one click from the "App Store → Plugins" category.
Both entry points land in exactly the same place (a plugin directory), and the existing upload-install feature is
unaffected.

### Package structure

Create `plugins/<name>/` under the App Store repository (e.g. `data/appstore/repos/appstore/`):

```
plugins/hello-plugin/
  app.yaml        # marketplace metadata: card title, description, version, category (category: plugins)
  manifest.yaml   # plugin runtime manifest (scope / ui / options / actions), same as a native plugin
  main.lua        # plugin logic (on_run / on_<action>)
  ui.html         # optional bundled HTML interface
  install.sh      # App Store install hook: place the directory into $ZAP_PATH/plugins/<name>
  uninstall.sh    # App Store uninstall hook: delete that plugin directory
```

- `app.yaml` handles the "marketplace card" and installation constraints; `manifest.yaml` handles "how the plugin
  runs". The two do not interfere with each other.
- `category` must be `plugins` (the App Store displays by this category; see the category list in
  `zapd/src/zap/appstore.rs`).
- `app.yaml` declares neither `provision` nor `run_as`: plugins do not go through site provisioning and are not
  privilege-dropped — the run identity is decided by `scope` in `manifest.yaml`.

### install.sh (install hook)

Executed by zapexec as **root** (system-level plugin). The App Store injects these environment variables:
`ZAP_PATH` (panel root), `APP_NAME` (plugin name), `PKG_SRC_PATH` (source directory of this package in the repository).

```sh
#!/bin/sh
set -e
DEST="$ZAP_PATH/plugins/$APP_NAME"
rm -rf "$DEST"
mkdir -p "$DEST"
cp -r "$PKG_SRC_PATH/." "$DEST/"
# drop the App Store-specific orchestration files to keep the plugin directory clean
rm -f "$DEST/app.yaml" "$DEST/install.sh" "$DEST/uninstall.sh"
echo "plugin $APP_NAME installed to $DEST"
```

`uninstall.sh` only needs `rm -rf "$ZAP_PATH/plugins/$APP_NAME"`. After a successful uninstall the App Store also
cleans up its own installation metadata (`apps/plugins/<name>/default/meta.yaml`), leaving nothing behind.

### Permissions and visibility

- **Admins only** can install system-level plugins from the marketplace (consistent with the `roles` allowlist rule in
  `app.yaml`; the default is admin-only — to open it to regular users in the marketplace add `roles: [user]`, in which
  case the plugin still lands in `$ZAP_PATH/plugins` guarded by the backend permission point `plugin:install`).
- After installation, `/plugin/list` picks up `$ZAP_PATH/plugins/<name>` automatically and the "Plugins" tab of site
  detail renders the entry as usual (including the `ui.html` sandboxed interface) — the App Store is only a
  "distribution channel"; the runtime is still the Lua engine.

### Upgrading

App Store upgrades reuse the same `install.sh`: copy the new version's repository content over
`$ZAP_PATH/plugins/<name>` again; you can also provide an `upgrade.sh` in `app.yaml` for a differential upgrade.

> A complete runnable example lives in `data/appstore/repos/appstore/plugins/hello-plugin/`
> (a plugin demonstrating an HTTP interface; after installation it appears on the `site.detail` slot in site detail).

---

## 12. Scheduled / webhook triggers

By default a plugin only runs when a user clicks its button. To run it periodically (`git fetch` every night) or let
an external system invoke it (auto-update after a CI push), add an entry under **Plugin management → that plugin's
"Triggers"** — no plugin code changes needed.

| Type | How it triggers | Good for |
| --- | --- | --- |
| `cron` | Five-field cron (minute hour day month weekday); the scheduler polls every 30 seconds, using the server's local timezone | Backups, sync, cleanup |
| `webhook` | External `POST <panel address>/api/plugin/hook/<random token>` | CI notifications, Git platform callbacks |

A few constraints:

- **Identity belongs to the creator**: the timer is not root and not anonymous — it is you, using your Linux account
  and your role (so a timer can only run buttons you could click yourself; `dangerous` actions remain blocked for
  read-only demo accounts).
- `site`-scoped plugins need the **site ID** filled in on the trigger, otherwise the site root is unavailable.
- Nobody else sees your triggers; they stop working immediately after a key rotation.
- Execution results (last run time / success or failure) are listed in the same table — no log digging required.

The webhook path is **public** in the permission matrix (authenticated by a `zap_`-style random token rather than the
panel JWT), so rotate the token immediately if it leaks.

---

## 13. Smoke tests and CI

Drop a `tests.yaml` into the plugin directory and "no action is broken" becomes a one-click check:

```yaml
- action: status              # defaults to run; falls back to on_run dispatched by ctx.action
  options: { cwd: '.' }
  user: www                   # scope: user / site must give a drop-privilege account (not root)
  site_root: /var/www/site    # scope: site also needs the site root (a missing userid skips rather than mis-reports failure)
  expect:
    contains: [branch]        # must appear in the log
    not_contains: [fatal]     # must not appear
```

Trigger it from the test button on the plugin management page (or call `/plugin/test` on the `zapd` side directly).
It returns each case's `ok` (`null` = skipped) and the actual log. Assertions run against the **log text**, so for an
action that only outputs JSON, pick a field that is always present to assert on.
