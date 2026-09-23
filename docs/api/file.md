---
title: 文件管理
---

# 文件管理

> 浏览服务器文件系统

## GET `/system/files/list`

目录列表

```bash
curl -X GET "https://<host>/api/system/files/list" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| path | string | 是 | 目录绝对路径 |

## GET `/system/files/read`

读取文本文件内容

```bash
curl -X GET "https://<host>/api/system/files/read" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/files/write`

写入文本文件

```bash
curl -X POST "https://<host>/api/system/files/write" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/files/delete`

删除文件 / 目录

```bash
curl -X POST "https://<host>/api/system/files/delete" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/files/mkdir`

新建目录

```bash
curl -X POST "https://<host>/api/system/files/mkdir" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/files/rename`

重命名 / 移动

```bash
curl -X POST "https://<host>/api/system/files/rename" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/files/download`

下载文件（二进制流）

```bash
curl -X GET "https://<host>/api/system/files/download" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/files/upload`

上传文件（multipart）

```bash
curl -X POST "https://<host>/api/system/files/upload" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/files/info`

单个文件详情

```bash
curl -X GET "https://<host>/api/system/files/info" \
  -H "Authorization: Bearer <token>"
```

