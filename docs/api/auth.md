---
title: 认证 Auth
---

# 认证 Auth

> 登录 / 登出 / 修改密码 / 两步验证（TOTP）

## POST `/auth/login`

账号密码登录，成功后返回 JWT（含用户信息）

```bash
curl -X POST "https://<host>/api/auth/login" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "username": "<string>",
     "password": "<string>",
     "totp_code": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| username | string | 是 | 用户名 |
| password | string | 是 | 密码 |
| totp_code | string | 否 | 已开启 2FA 时必填的六位验证码 |

## GET `/auth/logout`

退出登录

```bash
curl -X GET "https://<host>/api/auth/logout" \
  -H "Authorization: Bearer <token>"
```

## POST `/auth/reflash_token`

用现有 JWT 换取新的 JWT

```bash
curl -X POST "https://<host>/api/auth/reflash_token" \
  -H "Authorization: Bearer <token>"
```

## POST `/auth/change_password`

修改当前账号密码

```bash
curl -X POST "https://<host>/api/auth/change_password" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "old_password": "<string>",
     "new_password": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| old_password | string | 是 | 原密码 |
| new_password | string | 是 | 新密码（长度至少 6 位） |

## GET `/auth/totp/setup`

获取 TOTP 密钥、二维码 URI 与人工输入密钥

```bash
curl -X GET "https://<host>/api/auth/totp/setup" \
  -H "Authorization: Bearer <token>"
```

## POST `/auth/totp/verify`

校验验证码并开启两步验证

```bash
curl -X POST "https://<host>/api/auth/totp/verify" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "code": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| code | string | 是 | 认证器 App 上显示的六位验证码 |

## POST `/auth/totp/disable`

关闭两步验证

```bash
curl -X POST "https://<host>/api/auth/totp/disable" \
  -H "Authorization: Bearer <token>"
```

## GET `/auth/totp/status`

查询两步验证是否已开启

```bash
curl -X GET "https://<host>/api/auth/totp/status" \
  -H "Authorization: Bearer <token>"
```

## GET `/user/info`

当前登录用户资料

```bash
curl -X GET "https://<host>/api/user/info" \
  -H "Authorization: Bearer <token>"
```

