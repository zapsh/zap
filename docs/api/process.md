---
title: 系统服务与进程
---

# 系统服务与进程

> admin 专属

## GET `/system/config/services`

systemd 服务列表与状态

```bash
curl -X GET "https://<host>/api/system/config/services" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/services/action`

服务操作：start / stop / restart / enable / disable

```bash
curl -X POST "https://<host>/api/system/config/services/action" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/processes`

进程列表（含 CPU / 内存）

```bash
curl -X GET "https://<host>/api/system/config/processes" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/processes/kill`

结束进程

```bash
curl -X POST "https://<host>/api/system/config/processes/kill" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "pid": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| pid | int | 是 | 进程 PID |

