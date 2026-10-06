---
title: 站点管理
---

# 站点管理

> 一个站点可绑定多个域名与多个 IP；含站点同步、启停、安全（WAF / 限速）、日志与流量分析

## GET `/site/list`

站点列表（可按关键词搜索域名 / IP）

```bash
curl -X GET "https://<host>/api/site/list" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| search | string | 否 | 关键词，匹配域名或 IP |
| status | int | 否 | 按启用状态过滤（0 / 1） |
| user_id | int | 否 | 按归属用户过滤（admin / reseller 用） |

## GET `/site/users`

可选归属用户列表（admin / reseller 建站时选归属用户用）

```bash
curl -X GET "https://<host>/api/site/users" \
  -H "Authorization: Bearer <token>"
```

## GET `/site/feature`

当前账号的站点能力：是否可用反代（proxy）、WAF、自定义目录

```bash
curl -X GET "https://<host>/api/site/feature" \
  -H "Authorization: Bearer <token>"
```

## GET `/site/loc-directives`

location 附加指令白名单（面板下拉项，与执行端校验同源）

```bash
curl -X GET "https://<host>/api/site/loc-directives" \
  -H "Authorization: Bearer <token>"
```

## POST `/site/add`

新增站点

```bash
curl -X POST "https://<host>/api/site/add" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "user_id": "<int>",
     "name": "<string>",
     "domains": "<string[]>",
     "ips": "<string[]>",
     "site_type": "<string>",
     "php_instance": "<string>",
     "web_root_custom": "<bool>",
     "web_root": "<string>",
     "web_root_sub": "<string>",
     "ssl_cert_id": "<int>",
     "force_https": "<bool>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| user_id | int | 否 | 归属用户 ID；admin / reseller 必填，普通用户忽略（恒为自己） |
| name | string | 否 | 站点名称，留空默认取第一个域名 |
| domains | string[] | 是 | 绑定的域名列表（可多个） |
| ips | string[] | 否 | 绑定的 IP 列表（可多个） |
| site_type | string | 否 | 站点类型：php（默认）/ static（纯静态）/ proxy（反向代理） |
| php_instance | string | 否 | PHP 实例标识（appstore 已安装 PHP 应用的 instance，如 php74）；空 = 不绑定 |
| web_root_custom | bool | 否 | true = 使用归属用户家目录下已有自定义目录（需同时给 web_root） |
| web_root | string | 否 | 自定义站点目录绝对路径（web_root_custom=true 时必填） |
| web_root_sub | string | 否 | 自动目录自定义子路径（相对归属用户家目录，如 www/blog；空 = 面板默认规划） |
| ssl_cert_id | int | 否 | 绑定证书库证书 id；0 = 不启用 HTTPS |
| force_https | bool | 否 | 允许 HTTP 跳转 HTTPS（仅绑定证书后生效） |
| remark | string | 否 | 备注 |

## POST `/site/update`

修改站点（字段不传则保持原样，传入则整体覆盖；domains / ips / ssl_cert_id 传空或 0 表示清空 / 解绑）

```bash
curl -X POST "https://<host>/api/site/update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "name": "<string>",
     "domains": "<string[]>",
     "ips": "<string[]>",
     "run_state": "<string>",
     "status": "<int>",
     "php_instance": "<string>",
     "site_type": "<string>",
     "web_root_custom": "<bool>",
     "web_root": "<string>",
     "web_root_sub": "<string>",
     "ssl_cert_id": "<int>",
     "force_https": "<bool>",
     "sec": "<object>",
     "remark": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| name | string | 否 | 站点名称 |
| domains | string[] | 否 | 整体覆盖域名；不传 = 不变；传空数组 = 清空 |
| ips | string[] | 否 | 整体覆盖 IP |
| run_state | string | 否 | 运行状态：running / stopped / maintenance（与 status 同时传时以此为准） |
| status | int | 否 | 启用状态（0 / 1） |
| php_instance | string | 否 | None = 不变；空串 = 清除 PHP 实例绑定 |
| site_type | string | 否 | 站点类型（None = 不变） |
| web_root_custom | bool | 否 | Some(true)=切到「选择已有目录」；Some(false)=切回面板自动目录 |
| web_root | string | 否 | 自定义站点目录绝对路径（web_root_custom=true 且首次指定时必填） |
| web_root_sub | string | 否 | 自动目录自定义子路径（Some(非空) 时刷新为 {home}/{sub}） |
| ssl_cert_id | int | 否 | None = 不变；Some(0) = 解绑证书；Some(id) = 绑定证书 |
| force_https | bool | 否 | 允许 HTTP 跳转 HTTPS（None = 不变） |
| sec | object | 否 | 安全配置（WAF / 限速 / 限并发）；None = 不改动。结构见 /site/security/save |
| remark | string | 否 | 备注 |

## POST `/site/delete`

删除站点（级联清理域名 / IP 绑定；可选删除网站数据）

```bash
curl -X POST "https://<host>/api/site/delete" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "ids": "<int[]>",
     "remove_data": "<bool>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| ids | int[] | 是 | 站点 ID 列表（支持批量） |
| remove_data | bool | 否 | true = 同时删除站点文档根（网站文件）与日志目录；false / 不传 = 只删配置，保留数据 |

## POST `/site/sync`

同步单个站点 vhost 配置（改完配置后让执行端生效）

```bash
curl -X POST "https://<host>/api/site/sync" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |

## POST `/site/sync_all`

同步全部站点 vhost 配置（部署 / 迁移后批量生效用）

```bash
curl -X POST "https://<host>/api/site/sync_all" \
  -H "Authorization: Bearer <token>"
```

## POST `/site/state`

切换站点运行三态（同步 vhost：停止撤软链、维护发维护页）

```bash
curl -X POST "https://<host>/api/site/state" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "state": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| state | string | 是 | running（启动）/ stopped（停止）/ maintenance（维护页） |

## POST `/site/dirs`

浏览归属用户家目录下的子目录（供「选择已有目录」用）

```bash
curl -X POST "https://<host>/api/site/dirs" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "user_id": "<int>",
     "path": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| user_id | int | 否 | 归属用户 id；admin / reseller 可指定他人，普通用户忽略（恒为自己） |
| path | string | 否 | 当前浏览目录（不传 = 家目录根），返回其下子目录 |

## GET `/site/security`

读取站点安全配置（WAF / 限速 / 限并发）+ 两项能力可用性

```bash
curl -X GET "https://<host>/api/site/security" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |

## POST `/site/security/save`

保存站点安全配置并立即同步 vhost（安全片段随配置生效）

```bash
curl -X POST "https://<host>/api/site/security/save" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "sec": "<object>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| sec | object | 是 | 安全配置对象：waf_enable(bool), waf_mode(0跟随全局/1拦截/2仅检测), waf_rules(string), waf_audit(bool), whitelist(string,IP/CIDR), limit_dry_run(bool), limit_req_enable(bool), limit_req_rate(u32每秒请求上限), limit_req_burst(u32突发放行), limit_conn_enable(bool), limit_conn_num(u32单IP并发上限) |

## GET `/site/security/caps`

站点级 WAF 能力：全局是否就绪（waf_ready）、套餐是否允许（waf_allowed）与阻塞原因

```bash
curl -X GET "https://<host>/api/site/security/caps" \
  -H "Authorization: Bearer <token>"
```

## GET `/site/logs`

站点日志尾部行（当前日志或归档，支持关键词 / 状态码过滤）

```bash
curl -X GET "https://<host>/api/site/logs" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| kind | string | 否 | 日志类型：access / error；空 = 默认 |
| archive | string | 否 | 归档文件名（来自 /site/logs/archives）；空 = 当前日志 |
| lines | int | 否 | 返回行数，默认 200 |
| keyword | string | 否 | 关键词过滤 |
| status | string | 否 | 按 HTTP 状态码过滤（如 404 / 500） |

## GET `/site/logs/archives`

当前日志与历史归档列表（供查看 / 下载历史）

```bash
curl -X GET "https://<host>/api/site/logs/archives" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |

## POST `/site/logs/clear`

清空站点当前日志

```bash
curl -X POST "https://<host>/api/site/logs/clear" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>",
     "kind": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| kind | string | 否 | access / error；空 = 两者都清空 |

## POST `/site/logs/rotate`

手动轮转该站点日志（按天切割 + 归档）

```bash
curl -X POST "https://<host>/api/site/logs/rotate" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "id": "<int>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |

## GET `/site/logs/audit`

按 unique_id 反查该站点 WAF 审计明细

```bash
curl -X GET "https://<host>/api/site/logs/audit" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| unique_id | string | 是 | WAF 审计日志里的 unique_id（高熵随机串） |

## GET `/site/traffic`

站点流量分析（按天曲线 + Top URL + 汇总）

```bash
curl -X GET "https://<host>/api/site/traffic" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| id | int | 是 | 站点 ID |
| days | int | 否 | 统计天数，默认 30（0 视为 30，最大 365） |

