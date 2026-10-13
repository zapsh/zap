---
title: Users
---

# Users

> admin / reseller 创建与管理客户账号

## GET `/system/user/list`

用户分页列表

```bash
curl -X GET "https://<host>/api/system/user/list" \
  -H "Authorization: Bearer <token>"
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| page | int | No | 页码，默认 1 |
| page_size | int | No | 每页条数，默认 20 |
| keyword | string | No | 按用户名 / 昵称模糊搜索 |

## POST `/system/user/add`

新增用户

```bash
curl -X POST "https://<host>/api/system/user/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "username": "<string>",
     "password": "<string>",
     "role": "<string>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| username | string | Yes | 用户名 |
| password | string | Yes | 初始密码 |
| role | string | Yes | 角色：admin / reseller / user |

## POST `/system/user/update`

修改用户（昵称 / 状态 / 角色 / 重置密码等）

```bash
curl -X POST "https://<host>/api/system/user/update" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/user/delete`

删除用户

```bash
curl -X POST "https://<host>/api/system/user/delete" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/user/resellers`

经销商账号列表（用于选择上级）

```bash
curl -X GET "https://<host>/api/system/user/resellers" \
  -H "Authorization: Bearer <token>"
```

