-- zap 插件公共函数库（自动加载）
--
-- 本文件在 `main.lua` 之前由 zapexec 自动加载，插件里可以直接使用下面这些 helpers，
-- 无需 require。加载顺序：系统级 `data/plugins/_lib/` → 用户级 `~/.zap/plugins/_lib/`
-- → 插件自带 `<plugin>/lib/`，后者可覆盖前者。
--
-- 沙箱里没有 `io` / `os` / `package`（插件跑在 zapexec 进程里，放开它们就能以 root
-- 读写任意文件，绕过 scope=site 的降权）。需要与外部交互时用下面的 fs / run 系列，
-- 它们都按 scope 自动降权到站点 Linux 账号。

local M = zap

-- ── 日志 ───────────────────────────────────────────────────

--- 格式化日志：`zap.logf("共 %d 个文件", n)`
function M.logf(fmt, ...)
  zap.log(string.format(fmt, ...))
end

--- 断言，失败时抛出带消息的错误（会被 zapd 记为插件执行失败）。
function M.assert(cond, msg)
  if not cond then error(msg or '断言失败') end
  return cond
end

--- 直接抛错收尾。
function M.fail(msg)
  error(msg or '插件执行失败')
end

-- ── 字符串 ─────────────────────────────────────────────────
local str = {}
M.str = str

function str.trim(s)
  return (tostring(s or ''):gsub('^%s+', ''):gsub('%s+$', ''))
end

--- 空串 / 全空白。
function str.blank(s)
  return str.trim(s) == ''
end

--- 拆分字符串。sep 为空或 nil 时按空白拆分（正好用来拆 files / multiselect 的回传值）。
function str.split(s, sep, plain)
  s = tostring(s or '')
  local out = {}
  if sep == nil or sep == '' then
    for w in s:gmatch('%S+') do out[#out + 1] = w end
    return out
  end
  local pos = 1
  while true do
    local a, b = s:find(sep, pos, plain == true)
    if not a then
      out[#out + 1] = s:sub(pos)
      break
    end
    out[#out + 1] = s:sub(pos, a - 1)
    pos = b + 1
  end
  return out
end

--- 用 sep 连接列表（列表元素会被 tostring）。
function str.join(list, sep)
  local parts = {}
  for i, v in ipairs(list or {}) do parts[i] = tostring(v) end
  return table.concat(parts, sep or '')
end

function str.starts(s, prefix)
  return tostring(s or ''):sub(1, #prefix) == prefix
end

function str.ends(s, suffix)
  local t = tostring(s or '')
  return suffix == '' or t:sub(-#suffix) == suffix
end

function str.contains(s, sub, plain)
  return tostring(s or ''):find(sub, 1, plain == true) ~= nil
end

--- 按行拆分（去掉 \r）。
function str.lines(s)
  local out = {}
  for line in tostring(s or ''):gmatch('[^\r\n]+') do
    out[#out + 1] = (line:gsub('\r$', ''))
  end
  return out
end

-- ── 路径 ───────────────────────────────────────────────────
local path = {}
M.path = path

--- 拼接路径，多余的分隔符会被规范掉；后出现的绝对路径会重置结果（同 os.path.join）。
function path.join(...)
  local segs = {}
  for i = 1, select('#', ...) do
    local p = tostring((select(i, ...)) or '')
    if p ~= '' then segs[#segs + 1] = p end
  end
  if #segs == 0 then return '' end
  local out = segs[1]
  for i = 2, #segs do
    local p = segs[i]
    if p:sub(1, 1) == '/' then
      out = p
    else
      if out:sub(-1) ~= '/' then out = out .. '/' end
      out = out .. p
    end
  end
  return out
end

function path.dirname(p)
  p = tostring(p or '')
  local i = p:match('^.*()/')
  if not i then return '.' end
  if i == 1 then return '/' end
  return p:sub(1, i - 1)
end

function path.basename(p)
  p = tostring(p or ''):gsub('/+$', '')
  local name = p:match('([^/]*)$')
  return name or ''
end

function path.ext(p)
  return path.basename(p):match('%.([^.]*)$') or ''
end

function path.is_abs(p)
  return tostring(p or ''):sub(1, 1) == '/'
end

--- 纯字符串归一化：合并重复斜杠、消掉 `.` 与 `..` 段（不碰真实文件系统）。
function path.normalize(p)
  local p0 = tostring(p or '')
  local abs = path.is_abs(p0)
  local out = {}
  for _, seg in ipairs(str.split(p0, '/')) do
    if seg == '' or seg == '.' then
      -- 跳过
    elseif seg == '..' then
      if #out > 0 and out[#out] ~= '..' then
        out[#out] = nil
      elseif not abs then
        out[#out + 1] = '..'
      end
    else
      out[#out + 1] = seg
    end
  end
  local r = table.concat(out, '/')
  if abs then r = '/' .. r end
  if r == '' then return abs and '/' or '.' end
  return r
end

--- `p` 是否在 `base` 之内（用于挡住用户传进来的 `../..`）。
function path.within(base, p)
  base = path.normalize(base)
  local q = path.normalize(p)
  if q == base then return true end
  return q:sub(1, #base + 1) == base .. '/'
end

--- 站点内的路径：把相对路径拼到站点根上，并确保没有越出站点根。
--- scope≠site 时（没有站点根）直接返回原路径。
function path.site(rel)
  local root = zap.site_root()
  if root == nil or root == '' then return tostring(rel or '') end
  local full = path.normalize(path.join(root, rel or ''))
  if not path.within(root, full) then
    error(string.format('路径越出站点根目录: %s', tostring(rel)))
  end
  return full
end

-- ── 表 ─────────────────────────────────────────────────────
local tbl = {}
M.tbl = tbl

function tbl.keys(t)
  local out = {}
  for k in pairs(t or {}) do out[#out + 1] = k end
  return out
end

function tbl.values(t)
  local out = {}
  for _, v in pairs(t or {}) do out[#out + 1] = v end
  return out
end

function tbl.count(t)
  local n = 0
  for _ in pairs(t or {}) do n = n + 1 end
  return n
end

function tbl.map(t, fn)
  local out = {}
  for k, v in pairs(t or {}) do out[k] = fn(v, k) end
  return out
end

function tbl.filter(t, fn)
  local out = {}
  for _, v in ipairs(t or {}) do if fn(v) then out[#out + 1] = v end end
  return out
end

function tbl.contains(t, v)
  for _, x in ipairs(t or {}) do if x == v then return true end end
  return false
end

function tbl.index_of(t, v)
  for i, x in ipairs(t or {}) do if x == v then return i end end
  return nil
end

--- 浅合并，后面的覆盖前面的。
function tbl.merge(...)
  local out = {}
  for i = 1, select('#', ...) do
    local t = select(i, ...)
    for k, v in pairs(t or {}) do out[k] = v end
  end
  return out
end

-- ── Shell 转义 ─────────────────────────────────────────────

--- 把字符串包成 shell 里的单个参数（拼进 `sh -c` 脚本时用）。
function M.shell_quote(s)
  s = tostring(s or '')
  if s == '' then return "''" end
  if s:match('^[%w@%_%-%.%,/:%=%+]+$') then return s end
  return "'" .. s:gsub("'", "'\\''") .. "'"
end
M.quote = M.shell_quote

-- ── 文件系统（全部按 scope 降权到子进程执行）────────────────
local fs = {}
M.fs = fs

--- 路径存在？
function fs.exists(p)
  return zap.try_run('test', { '-e', p })
end

function fs.is_dir(p)
  return zap.try_run('test', { '-d', p })
end

function fs.is_file(p)
  return zap.try_run('test', { '-f', p })
end

--- 读整个文件（文本）。文件不存在会抛错，先用 fs.exists 判断。
function fs.read(p)
  return zap.read_file(p)
end

--- 覆盖写文件（不存在时自动建父目录）。
function fs.write(p, content)
  fs.mkdir(path.dirname(p))
  return zap.write_file(p, content)
end

--- 追加写文件。
function fs.append(p, content)
  fs.mkdir(path.dirname(p))
  return zap.append_file(p, content)
end

function fs.mkdir(p)
  return zap.run('mkdir', { '-p', p })
end

--- 递归删除。拒绝删根目录和空路径，避免手滑。
function fs.remove(p)
  local t = str.trim(tostring(p or ''))
  if t == '' or t == '/' then
    error('拒绝删除该路径: ' .. t)
  end
  return zap.run('rm', { '-rf', '--', t })
end

function fs.copy(src, dst)
  return zap.run('cp', { '-a', '--', src, dst })
end

function fs.move(src, dst)
  return zap.run('mv', { '--', src, dst })
end

function fs.chmod(mode, p)
  return zap.run('chmod', { mode, '--', p })
end

--- 列出目录下的条目名（不含 . 和 ..）。目录不存在返回空表。
function fs.list(p)
  local ok, out = zap.try_run('ls', { '-1', '-A', '--', p })
  if not ok then return {} end
  return str.lines(out)
end

--- 读文件并按行返回；不存在返回空表。
function fs.read_lines(p)
  if not fs.exists(p) then return {} end
  return str.lines(fs.read(p))
end

--- 文件大小（字节），读不到返回 nil。
function fs.size(p)
  local ok, out = zap.try_run('stat', { '-c', '%s', '--', p })
  if not ok then return nil end
  return tonumber(str.trim(out))
end

--- 在文件里追加一行（不存在则创建），常用于写 .gitignore / 配置片段。
function fs.append_line(p, line)
  return fs.append(p, line .. '\n')
end

--- 文件是否包含某段文本。
function fs.grep(p, needle)
  if not fs.exists(p) then return false end
  return str.contains(fs.read(p), needle, true)
end

-- ── 选项读取 ───────────────────────────────────────────────

--- 读选项，空值时回落到 default。
function M.opt(name, default)
  local v = zap.option(name)
  if v == nil or v == '' then return default end
  return v
end

--- 读必填选项，缺失直接抛错。
function M.opt_required(name)
  local v = zap.option(name)
  if v == nil or str.trim(v) == '' then
    error(string.format('缺少必填选项: %s', name))
  end
  return v
end

--- 读布尔选项（manifest 里 type=bool 的回传值是字符串 "true" / "false"）。
function M.opt_bool(name, default)
  local v = str.trim(zap.option(name) or ''):lower()
  if v == '' then return default == true end
  return v == 'true' or v == '1' or v == 'yes' or v == 'on'
end

--- 读数字选项。
function M.opt_number(name, default)
  local v = tonumber(str.trim(zap.option(name) or ''))
  if v == nil then return default end
  return v
end

--- 把 `files` / `multiselect` 回传的空格连接串拆成路径列表。
function M.opt_list(name)
  return str.split(zap.option(name) or '')
end

-- ── 杂项 ───────────────────────────────────────────────────

--- 当前作用域是否为 site（有站点上下文）。
--- 注意：`zap.canceled()` 本身就是 Rust 侧实现，这里不要同名覆盖。
function M.is_site_scope()
  return zap.scope() == 'site'
end

--- JSON 便捷读写（底层是 zap.json_encode / zap.json_decode）。
function M.to_json(v)
  return zap.json_encode(v)
end

function M.from_json(s)
  return zap.json_decode(s)
end

--- 当前时间戳（秒）。
function M.now()
  return zap.time()
end

--- 格式化时间，`fmt` 用 chrono 的格式串（默认 `%Y-%m-%d %H:%M:%S`）。
function M.fmt_time(fmt, ts)
  return zap.date(fmt or '%Y-%m-%d %H:%M:%S', ts)
end

-- 顶层短别名：写插件时 `zap.q(...)`、`zap.split(...)` 更顺手
M.split = str.split
M.trim = str.trim
M.join = str.join
M.q = M.shell_quote
