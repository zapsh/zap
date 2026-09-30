-- widgets-demo 插件：把 manifest 里声明的所有控件类型都读出来并打印，方便对照前端回传值。
-- 以站点 Linux 账号身份运行（scope=site → zap.exec_as_user 走 drop_privileges）。

function on_run(ctx)
  zap.log("=== 控件示例插件 ===")
  zap.log("scope       = " .. (ctx.scope or ""))
  zap.log("action      = " .. (ctx.action or ""))
  zap.log("site_root   = " .. zap.site_root())
  zap.log("site_linux  = " .. zap.site_linux_user())
  zap.log("--------------------------------------------------")
  zap.log("TEXT   (string)      = [" .. zap.option("TEXT") .. "]")
  zap.log("NUM    (number)      = [" .. zap.option("NUM") .. "]")
  zap.log("FLAG   (bool)        = [" .. zap.option("FLAG") .. "]")
  zap.log("SINGLE (select)      = [" .. zap.option("SINGLE") .. "]")
  zap.log("MULTI  (multiselect) = [" .. zap.option("MULTI") .. "]")
  zap.log("TARGET (dir)         = [" .. zap.option("TARGET") .. "]")
  zap.log("SRC    (file)        = [" .. zap.option("SRC") .. "]")
  zap.log("SRCS   (files)       = [" .. zap.option("SRCS") .. "]")
  zap.log("--------------------------------------------------")
  zap.log("TARGET 最终路径        = " .. zap.site_root() .. "/" .. zap.option("TARGET"))

  -- 多文件：把空格连接的串拆开，逐个打印（selected 的路径必然存在）
  local raw = zap.option("SRCS")
  if raw ~= "" then
    local i = 1
    for path in raw:gmatch("%S+") do
      zap.log("SRCS[" .. i .. "] = " .. path)
      i = i + 1
    end
  end

  zap.log("完成：以上即各控件回传到 main.lua 的值。")
end
