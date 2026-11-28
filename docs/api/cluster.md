---
title: 集群管理
---

# 集群管理

> 主控纳管其它 ZAP 面板：凭据加密存储、监控数据由主控采样落库；节点视图经 /nodes/{id}/** 反向代理，读写与终端都发生在受管节点上

## GET `/node/list`

受管节点列表（在线状态 / 最新版本采样 / 最近错误）

```bash
curl -X GET "https://<host>/api/node/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/node/add`

新增受管节点：API Token 与账号密码二选一；传账号密码时主控登录节点并自动签发长效 Token（不保存密码）

```bash
curl -X POST "https://<host>/api/node/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "base_url": "<string>",
     "api_token": "<string>",
     "username": "<string>",
     "password": "<string>",
     "totp_code": "<string>",
     "remark": "<string>",
     "insecure_tls": "<bool>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 是 | 节点显示名称 |
| base_url | string | 是 | 节点面板地址，含协议与端口，如 https://10.0.0.5:8443 |
| api_token | string | 否 | 节点上签发的 API Token（zap_ 开头）；与账号密码二选一 |
| username | string | 否 | 节点登录账号；api_token 留空时必填 |
| password | string | 否 | 节点登录密码；只用于本次签发，不落库 |
| totp_code | string | 否 | 节点账号开启两步验证时的 6 位验证码 |
| remark | string | 否 | 备注 |
| insecure_tls | bool | 否 | 是否容忍自签证书，默认 true |

## POST `/node/update`

修改节点：改名 / 地址 / 启停 / 换凭据；填账号密码会在节点上重新签发 Token（用于 Token 被删或过期后重连）

```bash
curl -X POST "https://<host>/api/node/update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "name": "<string>",
     "base_url": "<string>",
     "api_token": "<string>",
     "username": "<string>",
     "password": "<string>",
     "totp_code": "<string>",
     "status": "<int>",
     "remark": "<string>",
     "insecure_tls": "<bool>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 节点 ID |
| name | string | 否 | 新名称 |
| base_url | string | 否 | 新面板地址 |
| api_token | string | 否 | 新 Token，留空表示不修改 |
| username | string | 否 | 节点登录账号，与 password 一起传入时重新签发 Token |
| password | string | 否 | 节点登录密码 |
| totp_code | string | 否 | 两步验证码 |
| status | int | 否 | 1 启用 / 0 停用 |
| remark | string | 否 | 备注 |
| insecure_tls | bool | 否 | 是否容忍自签证书 |

## POST `/node/delete`

批量删除受管节点及其采样数据

```bash
curl -X POST "https://<host>/api/node/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "ids": "<int[]>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| ids | int[] | 是 | 节点 ID 数组 |

## POST `/node/test`

连通性测试：Token 模式读取节点系统信息；账号密码模式只验证能否登录，不签发 Token

```bash
curl -X POST "https://<host>/api/node/test" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "base_url": "<string>",
     "api_token": "<string>",
     "username": "<string>",
     "password": "<string>",
     "totp_code": "<string>",
     "insecure_tls": "<bool>",
     "id": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| base_url | string | 是 | 节点面板地址 |
| api_token | string | 否 | API Token；编辑态留空则复用库里已存的密文 |
| username | string | 否 | 节点登录账号（账号密码模式） |
| password | string | 否 | 节点登录密码（credential 模式） |
| totp_code | string | 否 | 两步验证码 |
| insecure_tls | bool | 否 | 是否容忍自签证书 |
| id | int | 否 | 编辑中的节点 ID（用于复用已存 Token） |

## POST `/node/refresh`

立即对所有启用节点采集一轮指标（定时任务也会周期性采集）

```bash
curl -X POST "https://<host>/api/node/refresh" \
  -H "Authorization: Bearer <token>"
```

## GET `/node/overview`

监控大屏数据：各节点最新采样 + 集群汇总 + 近 30 分钟趋势；只读主控自己的库

```bash
curl -X GET "https://<host>/api/node/overview" \
  -H "Authorization: Bearer <token>"
```

## GET `/node/status`

单节点历史曲线（从主控采样库读取，不访问节点）

```bash
curl -X GET "https://<host>/api/node/status" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 节点 ID |
| range | string | 否 | 时间范围：live（默认，近 30 分钟）/ 1h / 6h / 24h / 7d / 30d |

## ANY `/nodes/{id}/{*rest}`

节点视图反向代理：任意方法与路径转发到受管节点；GET/HEAD 只需 system.monitor:view，写操作与 WebSocket 另需 node:edit；主控自有接口（auth / node / dev）拒绝转发

```bash
curl -X ANY "https://<host>/api/nodes/{id}/{*rest}" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "rest": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 节点 ID（路径参数） |
| rest | string | 是 | 节点侧接口路径（路径参数），如 system/status |

