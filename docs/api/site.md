---
title: 站点管理
---

# 站点管理

> 一个站点可绑定多个域名与多个 IP

## GET `/site/list`

站点列表（可按关键词搜索域名 / IP）

```bash
curl -X GET "https://<host>/api/site/list" \
  -H "Authorization: Bearer <token>"
```

## GET `/site/users`

可选归属用户列表（admin / reseller 用）

```bash
curl -X GET "https://<host>/api/site/users" \
  -H "Authorization: Bearer <token>"
```

## POST `/site/add`

新增站点

```bash
curl -X POST "https://<host>/api/site/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "user_id": "<int>",
     "name": "<string>",
     "domains": "<string[]>",
     "ips": "<string[]>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| user_id | int | 是 | 归属用户 ID |
| name | string | 否 | 站点名称，留空默认取第一个域名 |
| domains | string[] | 是 | 绑定的域名列表（可多个） |
| ips | string[] | 否 | 绑定的 IP 列表（可多个） |
| remark | string | 否 | 备注 |

## POST `/site/update`

修改站点（domains / ips 不传则保持原样，传入则整体覆盖）

```bash
curl -X POST "https://<host>/api/site/update" \
  -H "Authorization: Bearer <token>"
```

## POST `/site/delete`

删除站点（级联清理域名 / IP 绑定；可选删除网站数据）

```bash
curl -X POST "https://<host>/api/site/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "ids": "<int[]>",
     "remove_data": "<bool>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| ids | int[] | 是 | 站点 ID 列表（支持批量） |
| remove_data | bool | 否 | true = 同时删除站点文档根（网站文件）与日志目录；false / 不传 = 只删配置，保留数据 |

