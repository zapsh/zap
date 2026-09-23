---
title: 系统状态与审计
---

# 系统状态与审计

> 监控大盘 / 操作审计

## GET `/system/info`

主机基础信息（CPU / 内存 / 磁盘 / 网络等）

```bash
curl -X GET "https://<host>/api/system/info" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/status`

实时运行状态快照

```bash
curl -X GET "https://<host>/api/system/status" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/overview`

大盘概览（图表数据源）

```bash
curl -X GET "https://<host>/api/system/overview" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/job/stop`

停止监控采集任务

```bash
curl -X GET "https://<host>/api/system/job/stop" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/job/start`

启动监控采集任务

```bash
curl -X GET "https://<host>/api/system/job/start" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/audit/list`

操作审计日志（分页 / 按动作与用户过滤）

```bash
curl -X GET "https://<host>/api/system/audit/list" \
  -H "Authorization: Bearer <token>"
```

