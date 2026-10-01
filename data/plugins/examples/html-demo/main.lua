-- HTML 界面演示插件。
--
-- 界面在 ui.html 里，用户点按钮后由父页面代跑 /plugin/run：
--   zap.call('scan', {TARGET='public'})  ->  这里进 on_scan
--   zap.call('du',   {TARGET='public'})  ->  这里进 on_du
-- 返回值（zap.log 的内容）会原样回给界面。

--- 扫描目录：列出一级子项，标出类型与大小。
function on_scan(ctx)
  local target = zap.opt('TARGET', '')
  local dir = zap.path.site(target)
  if not zap.fs.is_dir(dir) then
    zap.log('不是目录: ' .. dir)
    return
  end
  local names = zap.fs.list(dir)
  if #names == 0 then
    zap.log('（空目录）')
    return
  end
  for _, n in ipairs(names) do
    local full = zap.path.join(dir, n)
    local kind = zap.fs.is_dir(full) and 'dir ' or 'file'
    zap.logf('%s  %8d  %s', kind, zap.fs.size(full), n)
  end
end

--- 统计目录占用（du -sh），演示异步流式日志。
function on_du(ctx)
  local dir = zap.path.site(zap.opt('TARGET', ''))
  zap.log('统计中: ' .. dir)
  local ok, out = zap.try_run('du', { '-sh', dir })
  if not ok then
    zap.fail('du 执行失败: ' .. tostring(out))
  end
  zap.log(zap.str.trim(out))
end

--- 没声明 action 时的默认入口。
function on_run(ctx)
  zap.log('请用界面上的按钮操作（action=' .. tostring(ctx.action) .. '）')
end
