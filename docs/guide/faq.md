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