---
title: 菜单管理
---

# 菜单管理

> admin 专属；前端导航 / 路由由该数据动态生成

## GET `/system/menus/tree`

菜单树（登录后用于渲染侧边栏）

```bash
curl -X GET "https://<host>/api/system/menus/tree" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/menus/list`

菜单列表

```bash
curl -X GET "https://<host>/api/system/menus/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/menus/add`

新增菜单

```bash
curl -X POST "https://<host>/api/system/menus/add" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/menus/update`

修改菜单

```bash
curl -X POST "https://<host>/api/system/menus/update" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/menus/delete`

删除菜单

```bash
curl -X POST "https://<host>/api/system/menus/delete" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/menus/status`

启用 / 停用菜单

```bash
curl -X POST "https://<host>/api/system/menus/status" \
  -H "Authorization: Bearer <token>"
```

