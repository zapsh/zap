---
title: IP 池管理
---

# IP 池管理

> admin 专属；维护可分配 / 保留的 IP 资源

## GET `/system/ip/list`

IP 池分页列表

```bash
curl -X GET "https://<host>/api/system/ip/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/ip/add`

批量新增 IP（支持网段/范围展开）

```bash
curl -X POST "https://<host>/api/system/ip/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "addresses": "<string[]>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| addresses | string[] | 是 | IP 或 网段（如 1.1.1.0/24） |
| remark | string | 否 | 备注 |

## POST `/system/ip/delete`

删除 IP

```bash
curl -X POST "https://<host>/api/system/ip/delete" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/ip/update`

修改 IP（备注等）

```bash
curl -X POST "https://<host>/api/system/ip/update" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/ip/batch-reserved`

批量设置保留 / 取消保留

```bash
curl -X POST "https://<host>/api/system/ip/batch-reserved" \
  -H "Authorization: Bearer <token>"
```

