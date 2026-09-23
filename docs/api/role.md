---
title: 角色管理
---

# 角色管理

> admin 专属

## GET `/system/role/list`

角色列表

```bash
curl -X GET "https://<host>/api/system/role/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/role/add`

新增角色

```bash
curl -X POST "https://<host>/api/system/role/add" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/role/update`

修改角色

```bash
curl -X POST "https://<host>/api/system/role/update" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/role/delete`

删除角色

```bash
curl -X POST "https://<host>/api/system/role/delete" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/role/permissions`

查询角色已分配权限（菜单）

```bash
curl -X GET "https://<host>/api/system/role/permissions" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/role/permissions/set`

给角色分配权限（菜单 ID 列表）

```bash
curl -X POST "https://<host>/api/system/role/permissions/set" \
  -H "Authorization: Bearer <token>"
```

