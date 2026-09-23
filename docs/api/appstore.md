---
title: 应用商店 AppStore
---

# 应用商店 AppStore

> Git 源应用 / 脚本安装、运行与日志

## GET `/appstore/repos`

Git 仓库源列表

```bash
curl -X GET "https://<host>/api/appstore/repos" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/repos/add`

新增仓库源

```bash
curl -X POST "https://<host>/api/appstore/repos/add" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/repos/remove`

移除仓库源

```bash
curl -X POST "https://<host>/api/appstore/repos/remove" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/repos/update`

更新仓库源配置

```bash
curl -X POST "https://<host>/api/appstore/repos/update" \
  -H "Authorization: Bearer <token>"
```

## GET `/appstore/packages`

可安装应用 / 包列表

```bash
curl -X GET "https://<host>/api/appstore/packages" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/install`

安装应用

```bash
curl -X POST "https://<host>/api/appstore/install" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/uninstall`

卸载应用

```bash
curl -X POST "https://<host>/api/appstore/uninstall" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/upgrade`

升级应用

```bash
curl -X POST "https://<host>/api/appstore/upgrade" \
  -H "Authorization: Bearer <token>"
```

## GET `/appstore/scripts/tree`

脚本文件树

```bash
curl -X GET "https://<host>/api/appstore/scripts/tree" \
  -H "Authorization: Bearer <token>"
```

## GET `/appstore/script/read`

读取脚本内容

```bash
curl -X GET "https://<host>/api/appstore/script/read" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/script/write`

保存脚本

```bash
curl -X POST "https://<host>/api/appstore/script/write" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/script/run`

运行脚本 / 安装任务

```bash
curl -X POST "https://<host>/api/appstore/script/run" \
  -H "Authorization: Bearer <token>"
```

## POST `/appstore/script/stop`

停止运行中的任务

```bash
curl -X POST "https://<host>/api/appstore/script/stop" \
  -H "Authorization: Bearer <token>"
```

## GET `/appstore/runs`

历史运行记录

```bash
curl -X GET "https://<host>/api/appstore/runs" \
  -H "Authorization: Bearer <token>"
```

## GET `/appstore/log/{run_id}`

运行日志

```bash
curl -X GET "https://<host>/api/appstore/log/{run_id}" \
  -H "Authorization: Bearer <token>"
```

## WS `/appstore/ws/{run_id}`

WebSocket 实时日志流

```bash
curl -X WS "https://<host>/api/appstore/ws/{run_id}" \
  -H "Authorization: Bearer <token>"
```

