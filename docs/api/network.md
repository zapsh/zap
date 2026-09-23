---
title: 服务器时间 / 网络
---

# 服务器时间 / 网络

> admin 专属

## GET `/system/config/time`

查询系统当前时间与时区

```bash
curl -X GET "https://<host>/api/system/config/time" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/time/sync`

NTP 时间同步

```bash
curl -X POST "https://<host>/api/system/config/time/sync" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/time/timezone`

设置时区

```bash
curl -X POST "https://<host>/api/system/config/time/timezone" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/time/timezones`

可用时区列表

```bash
curl -X GET "https://<host>/api/system/config/time/timezones" \
  -H "Authorization: Bearer <token>"
```

## GET `/system/config/network`

查询主机名 / DNS 配置

```bash
curl -X GET "https://<host>/api/system/config/network" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/network/hostname`

修改主机名

```bash
curl -X POST "https://<host>/api/system/config/network/hostname" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/network/resolver`

修改 DNS 解析器

```bash
curl -X POST "https://<host>/api/system/config/network/resolver" \
  -H "Authorization: Bearer <token>"
```

