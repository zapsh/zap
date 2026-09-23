---
title: 开发（API Token）
---

# 开发（API Token）

> 本组接口即本页面数据来源

## GET `/dev/api-token/list`

当前账号的 API Token 列表（只含前缀，不返回完整值）

```bash
curl -X GET "https://<host>/api/dev/api-token/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/dev/api-token/create`

新建 API Token；完整值仅创建响应中返回一次

```bash
curl -X POST "https://<host>/api/dev/api-token/create" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "expire_days": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 否 | 备注名称 |
| expire_days | int | 否 | 有效期天数，缺省 / 0 表示永不过期 |

## POST `/dev/api-token/update`

修改备注或启停用

```bash
curl -X POST "https://<host>/api/dev/api-token/update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "name": "<string>",
     "status": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | Token ID |
| name | string | 否 | 新备注 |
| status | int | 否 | 1 启用 / 0 停用 |

## POST `/dev/api-token/delete`

吊销（删除）API Token

```bash
curl -X POST "https://<host>/api/dev/api-token/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | Token ID |

## GET `/dev/api-docs`

API 文档数据（当前页面）

```bash
curl -X GET "https://<host>/api/dev/api-docs" \
  -H "Authorization: Bearer <token>"
```

