---
title: 
---

# 常见问题（FAQ）

> 文档源：`FAQ_zh-CN.md`（位于仓库根）。  
> 由 `build.sh` 拷贝到 `data/www/html/FAQ_zh-CN.md`，  
> 后端 `GET /api/docs/faq?lang=zh-CN` 渲染为 HTML 后展示。

## Q1：登录失败 / token 过期

- 检查浏览器是否禁用 cookie。
- 检查 `data/zapd.yaml` 的 `jwt_secret` 是否在重启后被重置。

## Q2：站点添加后访问 502

- 确认 PHP-FPM 已启动。
- 确认站点 `path` 路径存在且权限正确。

## Q3：升级后菜单丢了一些

- 检查 `data/zap.db` 是否被迁移脚本改过；新版本 seed 在 `init_db.rs`，开发期可直接重建数据库。

## Q4：API 跨域调用 401

- 在 `system/zap` → `JWT` 关闭 `cookie_only` 或者把 `Access-Control-Allow-Credentials` 配好。

## Q5：节点离线 / 接管时跳回登录页

- 节点 Token 被删或过期：在「集群 → 节点管理」编辑该节点，重新填一次节点账号密码即可自动换发；也可以手工填新 Token。
- 「测试」按钮只验证登录，不会签发 Token；要让新凭据生效请点「保存」。
- 只有 `system.monitor:view` 时接管里的写操作会被节点拒绝；写操作和终端还需要主控账号具备 `node:edit`。
- 节点侧 401 不会再把主控踢回登录页：节点问题在「集群 → 节点管理」更新凭据；主控自己的登录过期按主控账号正常登录即可。