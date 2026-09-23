---
title: SSL/TLS（证书管理）
---

# SSL/TLS（证书管理）

> SSL 证书（crt / key / ca-bundle / csr）的导入、自签名生成与 Let's Encrypt 自动申请

## GET `/ssl/cert/list`

证书列表（不含私钥等内容，只含元信息）

```bash
curl -X GET "https://<host>/api/ssl/cert/list" \
  -H "Authorization: Bearer <token>"
```

## GET `/ssl/cert/detail`

证书详情（含 crt / key / ca-bundle / csr 明文）

```bash
curl -X GET "https://<host>/api/ssl/cert/detail" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 证书 ID |

## POST `/ssl/cert/parse`

解析证书 / CSR：自动读取域名（SAN + CN）、有效期、签发者、公钥、指纹；传 key_pem 时一并校验两者是否配对

```bash
curl -X POST "https://<host>/api/ssl/cert/parse" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "pem": "<string>",
     "key_pem": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| pem | string | 是 | PEM 文本（证书 crt 或 CSR；多张证书时只解析第一张叶子证书） |
| key_pem | string | 否 | 私钥 PEM；提供时返回 key_match（true/false）或 key_error（私钥无法解析） |

## POST `/ssl/cert/add`

手动添加证书（至少提供 cert / key / csr 之一；域名留空时自动从证书解析；cert 与 key 同时提供时会校验配对，不匹配则拒绝）

```bash
curl -X POST "https://<host>/api/ssl/cert/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "domains": "<string>",
     "cert_content": "<string>",
     "key_content": "<string>",
     "ca_bundle": "<string>",
     "csr": "<string>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 是 | 证书名称 |
| domains | string | 否 | 覆盖域名，逗号分隔 |
| cert_content | string | 否 | 证书 PEM（crt） |
| key_content | string | 否 | 私钥 PEM（key） |
| ca_bundle | string | 否 | CA 中间链 PEM |
| csr | string | 否 | CSR PEM |
| remark | string | 否 | 备注 |

## POST `/ssl/cert/update`

修改证书（名称 / 内容 / 备注 / 状态）；PEM 四段未传则保持原值，传空串才清空；key 与 cert 不匹配时拒绝

```bash
curl -X POST "https://<host>/api/ssl/cert/update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "name": "<string>",
     "status": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 证书 ID |
| name | string | 是 | 证书名称 |
| status | int | 否 | 1 启用 / 0 停用 |

## POST `/ssl/cert/delete`

删除证书

```bash
curl -X POST "https://<host>/api/ssl/cert/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 证书 ID |

## POST `/ssl/cert/self-sign`

生成自签名证书（rcgen，支持域名与 IP，保存 crt/key/csr）

```bash
curl -X POST "https://<host>/api/ssl/cert/self-sign" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "domains": "<string>",
     "days": "<int>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 是 | 证书名称 |
| domains | string | 是 | 域名 / IP，逗号分隔 |
| days | int | 否 | 有效天数，默认 365 |
| remark | string | 否 | 备注 |

## POST `/ssl/cert/letsencrypt`

向 Let's Encrypt 申请证书（ACME HTTP-01，需 80 端口可达）

```bash
curl -X POST "https://<host>/api/ssl/cert/letsencrypt" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "domains": "<string>",
     "email": "<string>",
     "name": "<string>",
     "staging": "<bool>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| domains | string | 是 | 域名（不可为 IP），逗号分隔，首个为主域名 |
| email | string | 是 | ACME 账户邮箱 |
| name | string | 否 | 证书名称，缺省用主域名 |
| staging | bool | 否 | true 使用 Let's Encrypt 测试环境 |
| remark | string | 否 | 备注 |

