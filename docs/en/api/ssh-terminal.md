---
title: SSH Terminal (Connections)
---

# SSH Terminal (Connections)

> 保存的主机连接；页面内嵌 Web 终端

## GET `/terminal/connections`

已保存的连接列表

```bash
curl -X GET "https://<host>/api/terminal/connections" \
  -H "Authorization: Bearer <token>"
```

## GET `/terminal/connections/{id}`

连接详情

```bash
curl -X GET "https://<host>/api/terminal/connections/{id}" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/connections/create`

新增连接（主机 / 端口 / 认证方式等）

```bash
curl -X POST "https://<host>/api/terminal/connections/create" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/connections/{id}/update`

修改连接

```bash
curl -X POST "https://<host>/api/terminal/connections/{id}/update" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/connections/{id}/delete`

删除连接

```bash
curl -X POST "https://<host>/api/terminal/connections/{id}/delete" \
  -H "Authorization: Bearer <token>"
```

## GET `/terminal/connections/test`

测试连接连通性

```bash
curl -X GET "https://<host>/api/terminal/connections/test" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/connections/{id}/push-key`

推送本机公钥到目标主机

```bash
curl -X POST "https://<host>/api/terminal/connections/{id}/push-key" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/push-key`

按表单参数直推公钥（连接无需先保存）

```bash
curl -X POST "https://<host>/api/terminal/push-key" \
  -H "Authorization: Bearer <token>"
```

## WS `/terminal/ws/{id}`

WebSocket 终端通道（使用 query 携带 JWT；record=0 可关闭会话录制）

```bash
curl -X WS "https://<host>/api/terminal/ws/{id}" \
  -H "Authorization: Bearer <token>"
```

## GET `/terminal/sftp/list`

SFTP 列目录（复用终端连接）

```bash
curl -X GET "https://<host>/api/terminal/sftp/list" \
  -H "Authorization: Bearer <token>"
```

## GET `/terminal/sftp/download`

SFTP 下载文件（base64）

```bash
curl -X GET "https://<host>/api/terminal/sftp/download" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/sftp/upload`

SFTP 上传文件（base64）

```bash
curl -X POST "https://<host>/api/terminal/sftp/upload" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/sftp/mkdir`

SFTP 新建目录

```bash
curl -X POST "https://<host>/api/terminal/sftp/mkdir" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/sftp/remove`

SFTP 删除文件/目录

```bash
curl -X POST "https://<host>/api/terminal/sftp/remove" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/sftp/rename`

SFTP 重命名/移动

```bash
curl -X POST "https://<host>/api/terminal/sftp/rename" \
  -H "Authorization: Bearer <token>"
```

## GET `/terminal/recordings`

会话录制列表（按用户隔离）

```bash
curl -X GET "https://<host>/api/terminal/recordings" \
  -H "Authorization: Bearer <token>"
```

## GET `/terminal/recordings/{id}`

取回一条录制（asciinema cast 文本，供回放）

```bash
curl -X GET "https://<host>/api/terminal/recordings/{id}" \
  -H "Authorization: Bearer <token>"
```

## POST `/terminal/recordings/{id}/delete`

删除一条会话录制

```bash
curl -X POST "https://<host>/api/terminal/recordings/{id}/delete" \
  -H "Authorization: Bearer <token>"
```

