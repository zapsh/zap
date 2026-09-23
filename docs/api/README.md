---
title: API 总览
---

# Zap 管理后台 API 文档

> 共 116 个接口，基址 `/api`。左侧目录按功能分组。

## 认证

- 所有业务接口需要在请求头携带认证凭据： Authorization: Bearer &lt;token&gt;
- 支持两种凭据：1) 登录 /api/auth/login 获取的 JWT（短期有效，过期后重新登录或调用 /api/auth/reflash_token 刷新）；2) 长效 API Token（在“开发 → 管理 API Tokens”中创建，形如 zap_ 开头，可设置有效期，随时可吊销）。
- 不同角色可访问的接口范围不同：admin 拥有全部权限；reseller 管理自己的客户；user 管理自己的资源；demo（只读演示）仅允许 GET 浏览，写操作返回 403。
- 统一响应结构：{"code": 0, "message": "OK", "data": ...}；code 非 0 表示失败，message 为错误说明。
- 路径变量用 {name} 表示，调用时替换为实际值。

## 统一响应

```json
{ "code": 0, "message": "OK", "data": ... }
```

`code` 非 0 表示失败，`message` 是可读的错误说明。

## 接口目录

| 分组 | 接口数 |
| --- | -: |
| [健康检查](/api/health.html) | 1 |
| [认证 Auth](/api/auth.html) | 9 |
| [用户管理](/api/user.html) | 5 |
| [角色管理](/api/role.html) | 6 |
| [菜单管理](/api/menu.html) | 6 |
| [服务器时间 / 网络](/api/network.html) | 7 |
| [IP 池管理](/api/ip-pool.html) | 5 |
| [系统服务与进程](/api/process.html) | 4 |
| [SSH 服务与密钥](/api/ssh.html) | 12 |
| [SSH 终端（连接管理）](/api/ssh-terminal.html) | 9 |
| [系统状态与审计](/api/audit.html) | 6 |
| [文件管理](/api/file.html) | 9 |
| [应用商店 AppStore](/api/appstore.html) | 16 |
| [站点管理](/api/site.html) | 5 |
| [开发（API Token）](/api/token.html) | 5 |
| [SSL/TLS（证书管理）](/api/ssl.html) | 8 |
| [Zap 设置（面板自身配置）](/api/zap-config.html) | 3 |
