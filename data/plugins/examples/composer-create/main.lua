-- composer-create 插件：在站点根（或指定子目录）下执行 composer create-project。
-- 以站点 Linux 账号身份运行（scope=site → zap.exec_as_user 走 drop_privileges）。

function on_run(ctx)
  local pkg = zap.option("PACKAGE")
  if pkg == "" then
    zap.log("错误：未提供包名（PACKAGE）")
    error("缺少 PACKAGE")
  end

  local target = zap.option("TARGET")
  local root = zap.site_root()
  local dest = root
  if target ~= "" then
    dest = root .. "/" .. target
  end

  zap.log("在 " .. dest .. " 创建 Composer 项目：" .. pkg)
  zap.log("运行身份：" .. zap.site_linux_user())

  -- 以站点账号执行；composer 需在站点账号的 PATH 中（面板已探测 composer 可用性）
  local out = zap.exec_as_user("composer", {
    "create-project",
    "--prefer-dist",
    pkg,
    dest,
  })
  zap.log(out)

  zap.log("完成。可把站点根切换到 " .. dest .. "/public 并加 Laravel 伪静态（try_files）。")
end
