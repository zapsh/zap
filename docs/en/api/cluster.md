---
title: Cluster Management
---

# Cluster Management

> Register other ZAP panels under the master. Credentials are stored encrypted; metrics are sampled by the master. The node view proxies /nodes/{id}/** so reads, writes and terminal sessions happen on the managed node

## GET `/node/list`

List managed nodes with online status, group, credential status and last error; scoped by ownership

```bash
curl -X GET "https://<host>/api/node/list" \
  -H "Authorization: Bearer <token>"
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| keyword | string | No | Search name, URL, public IP or remark |
| group_id | int | No | Group ID; 0 means no filter |
| status | int | No | 1 enabled / 0 disabled |
| online | int | No | 1 online / 0 offline |
| sort | string | No | Sort by id, name, cpu, memory, online or last_seen |

## POST `/node/add`

Add a managed node: supply either an API token or account credentials. With credentials, the master logs in and issues a long-lived token (the password is never stored). The token must belong to a node administrator because network/process collection needs Admin

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
     "insecure_tls": "<bool>",
     "group_id": "<int>",
     "owner_id": "<int>",
     "cred_expire_days": "<int>"
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
| group_id | int | No | Group ID; 0 means ungrouped |
| owner_id | int | No | Owner user ID; admins only, other callers always own new nodes |
| cred_expire_days | int | No | Lifetime in days for the bootstrapped token; 0 uses the node default |

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
     "insecure_tls": "<bool>",
     "group_id": "<int>",
     "owner_id": "<int>",
     "cred_expire_days": "<int>"
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
| group_id | int | No | Group ID |
| owner_id | int | No | Owner user ID (admins only) |
| cred_expire_days | int | No | Lifetime in days for a reissued token |

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

Test connectivity and verify administrator permission: token mode reads system info; credentials mode only verifies login and issues no token

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

Monitor dashboard data: latest samples, cluster/group summaries and 30-minute trend; reads only the master database

```bash
curl -X GET "https://<host>/api/node/overview" \
  -H "Authorization: Bearer <token>"
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| group_id | int | No | Restrict to one group |

## GET `/node/status`

Historical metrics for one node including network rates, read from the master database, plus the latest Top 10 process snapshot

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

## GET `/node/group/list`

List node groups with node counts

```bash
curl -X GET "https://<host>/api/node/group/list" \
  -H "Authorization: Bearer <token>"
```

## POST `/node/group/add`

Create a group (admins only)

```bash
curl -X POST "https://<host>/api/node/group/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "sort_order": "<int>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| name | string | Yes | Unique group name |
| sort_order | int | No | Sort order |

## POST `/node/group/update`

Rename or reorder a group (admins only)

```bash
curl -X POST "https://<host>/api/node/group/update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "name": "<string>",
     "sort_order": "<int>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| id | int | Yes | Group ID |
| name | string | Yes | New name |
| sort_order | int | No | Sort order |

## POST `/node/group/delete`

Delete a group (admins only); its nodes become ungrouped

```bash
curl -X POST "https://<host>/api/node/group/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| id | int | Yes | Group ID |

## POST `/node/batch/update`

Batch enable/disable or regroup; ownership is checked per node and any violation rejects the whole batch

```bash
curl -X POST "https://<host>/api/node/batch/update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "ids": "<int[]>",
     "status": "<int>",
     "group_id": "<int>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| ids | int[] | Yes | Array of node IDs |
| status | int | No | 1 enabled / 0 disabled |
| group_id | int | No | Target group; 0 means ungrouped |

## POST `/node/batch/script`

Dispatch an existing App Store script to several nodes; per-node results, no arbitrary commands

```bash
curl -X POST "https://<host>/api/node/batch/script" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "ids": "<int[]>",
     "path": "<string>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| ids | int[] | Yes | Array of node IDs |
| path | string | Yes | Path of a script already present on the nodes |

## GET `/node/owners`

List users eligible as node owners (all for admins; self and members for resellers)

```bash
curl -X GET "https://<host>/api/node/owners" \
  -H "Authorization: Bearer <token>"
```

## GET `/node/alert/config`

Read cluster alert thresholds (CPU / memory / disk / offline seconds / cooldown / email)

```bash
curl -X GET "https://<host>/api/node/alert/config" \
  -H "Authorization: Bearer <token>"
```

## POST `/node/alert/config`

Update alert thresholds (admins only); 0 disables a check

```bash
curl -X POST "https://<host>/api/node/alert/config" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "cpu": "<int>",
     "memory": "<int>",
     "disk": "<int>",
     "offline_seconds": "<int>",
     "cooldown_seconds": "<int>",
     "email": "<bool>"
   }'
```

| Parameter | Type | Required | Description |
| --- | --- | :-: | --- |
| cpu | int | Yes | CPU percentage threshold |
| memory | int | Yes | Memory percentage threshold |
| disk | int | Yes | Disk percentage threshold |
| offline_seconds | int | Yes | Offline alert delay in seconds |
| cooldown_seconds | int | Yes | Cooldown in seconds (minimum 60) |
| email | bool | Yes | Also email administrators |

## GET `/node/alert/events`

Latest 200 cluster alert events, scoped by node ownership

```bash
curl -X GET "https://<host>/api/node/alert/events" \
  -H "Authorization: Bearer <token>"
```

