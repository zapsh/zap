---
title: Cluster Management
---

# Cluster Management

> Register other ZAP panels under the master. Credentials are stored encrypted; metrics are sampled by the master. The node view proxies /nodes/{id}/** so reads, writes and terminal sessions happen on the managed node

## GET `/node/list`

List managed nodes with online status, latest sample and last error

```bash
curl -X GET "https://<host>/api/node/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/node/add`

Add a managed node: supply either an API token or account credentials. With credentials, the master logs in and issues a long-lived token (the password is never stored)

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

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| name | string | Yes | Display name |
| base_url | string | Yes | Panel URL including scheme and port, e.g. https://10.0.0.5:8443 |
| api_token | string | No | API token issued on the node (starts with zap_); alternative to credentials |
| username | string | No | Node login username; required when api_token is empty |
| password | string | No | Node login password; used only for token issuance, never stored |
| totp_code | string | No | 6-digit code when two-factor authentication is enabled on the node |
| remark | string | No | Remark |
| insecure_tls | bool | No | Allow self-signed certificates, defaults to true |

## POST `/node/update`

Update a node: rename, change URL, enable/disable or replace credentials; supplying credentials reissues the token (e.g. after deletion or expiry)

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

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| id | int | Yes | Node ID |
| name | string | No | New name |
| base_url | string | No | New panel URL |
| api_token | string | No | New token; empty means keep the stored one |
| username | string | No | Node login username; with password triggers token reissue |
| password | string | No | Node login password |
| totp_code | string | No | Two-factor code |
| status | int | No | 1 enabled / 0 disabled |
| remark | string | No | Remark |
| insecure_tls | bool | No | Allow self-signed certificates |

## POST `/node/delete`

Delete managed nodes and their metrics

```bash
curl -X POST "https://<host>/api/node/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "ids": "<int[]>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| ids | int[] | Yes | Array of node IDs |

## POST `/node/test`

Test connectivity: token mode reads system info; credentials mode only verifies login and issues no token

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

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| base_url | string | Yes | Node panel URL |
| api_token | string | No | API token; when editing, empty reuses the stored token |
| username | string | No | Node login username (credential mode) |
| password | string | No | Node login password (credential mode) |
| totp_code | string | No | Two-factor code |
| insecure_tls | bool | No | Allow self-signed certificates |
| id | int | No | Node ID being edited, used to reuse its stored token |

## POST `/node/refresh`

Immediately sample all enabled nodes once (a scheduled task also samples periodically)

```bash
curl -X POST "https://<host>/api/node/refresh" \
  -H "Authorization: Bearer <token>"
```

## GET `/node/overview`

Monitor dashboard data: latest samples, cluster summary and 30-minute trend; reads only the master database

```bash
curl -X GET "https://<host>/api/node/overview" \
  -H "Authorization: Bearer <token>"
```

## GET `/node/status`

Historical metrics for one node, read from the master sample database without contacting the node

```bash
curl -X GET "https://<host>/api/node/status" \
  -H "Authorization: Bearer <token>"
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| id | int | Yes | Node ID |
| range | string | No | Time window: live (default, last 30 minutes), 1h, 6h, 24h, 7d or 30d |

## ANY `/nodes/{id}/{*rest}`

Node-view reverse proxy: forwards any method and path to the managed node. GET/HEAD require system.monitor:view; writes and WebSocket also require node:edit. Master-owned paths (auth / node / dev) are denied

```bash
curl -X ANY "https://<host>/api/nodes/{id}/{*rest}" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "rest": "<string>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| id | int | Yes | Node ID (path parameter) |
| rest | string | Yes | Path on the node (path parameter), e.g. system/status |

