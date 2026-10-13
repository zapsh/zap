---
title: API Overview
---

# Zap 管理后台 API 文档

::: warning Translation in progress
Some endpoint descriptions have no English text yet and fall back to the Chinese source.
Add `summary_en` / `desc_en` fields in `zapd/src/routers/api_docs.json` and re-run `npm run gen:api`.
:::

> 158 endpoints in total, base path `/api`. The sidebar groups them by feature.

## Authentication

- 所有业务接口需要在请求头携带认证凭据： Authorization: Bearer &lt;token&gt;
- 支持两种凭据：1) 登录 /api/auth/login 获取的 JWT（短期有效，过期后重新登录或调用 /api/auth/reflash_token 刷新）；2) 长效 API Token（在“开发 → 管理 API Tokens”中创建，形如 zap_ 开头，可设置有效期，随时可吊销）。
- 不同角色可访问的接口范围不同：admin 拥有全部权限；reseller 管理自己的客户；user 管理自己的资源；demo（只读演示）仅允许 GET 浏览，写操作返回 403。
- 统一响应结构：{"code": 0, "message": "OK", "data": ...}；code 非 0 表示失败，message 为错误说明。
- 路径变量用 {name} 表示，调用时替换为实际值。

## Unified Response

```json
{ "code": 0, "message": "OK", "data": ... }
```

Any `code` other than 0 means failure; `message` carries a human-readable error description.

## Endpoint Index

| Group | Endpoints |
| --- | -: |
| [Health Check](/en/api/health) | 1 |
| [Authentication](/en/api/auth) | 9 |
| [Users](/en/api/user) | 5 |
| [Roles](/en/api/role) | 6 |
| [Menus](/en/api/menu) | 6 |
| [Server Time & Network](/en/api/network) | 7 |
| [IP Pool](/en/api/ip-pool) | 5 |
| [Services & Processes](/en/api/process) | 4 |
| [SSH Service & Keys](/en/api/ssh) | 12 |
| [SSH Terminal (Connections)](/en/api/ssh-terminal) | 18 |
| [System Status & Audit](/en/api/audit) | 6 |
| [File Management](/en/api/file) | 10 |
| [App Store](/en/api/appstore) | 16 |
| [Plugins](/en/api/plugin) | 8 |
| [Sites](/en/api/site) | 20 |
| [Site Applications](/en/api/site-app) | 9 |
| [Development (API Tokens)](/en/api/token) | 5 |
| [SSL/TLS (Certificates)](/en/api/ssl) | 8 |
| [ZAP Settings](/en/api/zap-config) | 3 |
