---
title: SSH 服务与密钥
---

# SSH 服务与密钥

> admin 专属

## GET `/system/config/ssh/status`

SSH 服务状态与端口

```bash
curl -X GET "https://<host>/api/system/config/ssh/status" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/restart`

重启 SSH 服务

```bash
curl -X POST "https://<host>/api/system/config/ssh/restart" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/install`

安装 / 配置 SSH（密钥登录等）

```bash
curl -X POST "https://<host>/api/system/config/ssh/install" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/ssh/install/log/{run_id}`

安装任务日志

```bash
curl -X GET "https://<host>/api/system/config/ssh/install/log/{run_id}" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/ssh/keys`

本机 SSH 密钥对列表

```bash
curl -X GET "https://<host>/api/system/config/ssh/keys" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/ssh/keys/content`

查看密钥对公钥 / 私钥内容

```bash
curl -X GET "https://<host>/api/system/config/ssh/keys/content" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/keys/generate`

生成新的密钥对

```bash
curl -X POST "https://<host>/api/system/config/ssh/keys/generate" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/keys/import`

导入密钥对

```bash
curl -X POST "https://<host>/api/system/config/ssh/keys/import" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/keys/delete`

删除密钥对

```bash
curl -X POST "https://<host>/api/system/config/ssh/keys/delete" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/ssh/authorized_keys`

authorized_keys 授权列表

```bash
curl -X GET "https://<host>/api/system/config/ssh/authorized_keys" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/authorize`

将公钥加入 authorized_keys

```bash
curl -X POST "https://<host>/api/system/config/ssh/authorize" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/ssh/deauthorize`

从 authorized_keys 移除公钥

```bash
curl -X POST "https://<host>/api/system/config/ssh/deauthorize" \
  -H "Authorization: Bearer <token>"
```

