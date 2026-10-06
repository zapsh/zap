---
title: 域名站点应用管理
---

# 域名站点应用管理

> 域名站点下的应用（Application Manager）：能力 / 运行时查询、部署、重新部署、启停、移除与日志。均需 Bearer 鉴权且调用者对 site_id 有访问权

## GET `/site/app/caps`

站点可部署的应用能力（套餐允许的 app_type 列表与端口段等）

```bash
curl -X GET "https://<host>/api/site/app/caps" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |

## GET `/site/app/runtimes`

可用的运行时版本列表（python / nodejs / go 等及其可选项）

```bash
curl -X GET "https://<host>/api/site/app/runtimes" \
  -H "Authorization: Bearer <token>"
```

## GET `/site/app/list_all`

全部站点 + 应用清单（含 site_id / site_name / name / app_type / workdir），用于两级下拉

```bash
curl -X GET "https://<host>/api/site/app/list_all" \
  -H "Authorization: Bearer <token>"
```

## GET `/site/app/list`

某个站点的应用列表

```bash
curl -X GET "https://<host>/api/site/app/list" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |

## POST `/site/app/deploy`

部署应用（创建 / 更新并构建；参数较多，适合首次上线或复杂配置）

```bash
curl -X POST "https://<host>/api/site/app/deploy" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "site_id": "<int>",
     "name": "<string>",
     "app_type": "<string>",
     "runtime_version": "<string>",
     "build_cmd": "<string>",
     "create_venv": "<bool>",
     "workdir": "<string>",
     "entry": "<string>",
     "command": "<string>",
     "port": "<int>",
     "env": "<string>",
     "autostart": "<bool>",
     "install_deps": "<bool>",
     "auto_port": "<bool>",
     "domain": "<string>",
     "mount_path": "<string>",
     "match_mode": "<string>",
     "strip_prefix": "<bool>",
     "repo_url": "<string>",
     "branch": "<string>",
     "git_ref": "<string>",
     "git_subdir": "<string>",
     "git_depth": "<int>",
     "build_output": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID（与 domain 二选一） |
| name | string | 是 | 应用名（站点内唯一），仅字母数字 -_. 且 ≤48 |
| app_type | string | 是 | python / nodejs / go / static / generic 等（受套餐 caps.types 限制） |
| runtime_version | string | 否 | 运行时版本（如 3.11 / 20），空 = 系统默认 |
| build_cmd | string | 否 | 部署时先执行的构建命令，空 = 不构建 |
| create_venv | bool | 否 | python：是否生成 .venv（默认开启） |
| workdir | string | 否 | 工作目录，空 = 站点 web_root |
| entry | string | 否 | 入口 |
| command | string | 否 | 自定义启动命令（非空则覆盖类型默认模板；generic 必填） |
| port | int | 否 | 端口；配合 auto_port 可自动分配 |
| env | string | 否 | 环境变量，每行一条 KEY=VALUE |
| autostart | bool | 否 | 是否开机 / 部署后自启（默认 true） |
| install_deps | bool | 否 | 是否安装依赖（pip / npm） |
| auto_port | bool | 否 | 自动分配端口（true 时忽略 port） |
| domain | string | 否 | 公开仓库地址（空 = 使用现有 workdir）；也可填域名自动建反向代理站点 |
| mount_path | string | 否 | 站点上反代挂载前缀，默认 / |
| match_mode | string | 否 | 匹配方式：空=前缀 / exact=精确 / prefer=优先前缀 |
| strip_prefix | bool | 否 | 是否剥离挂载前缀再转发 |
| repo_url | string | 否 | 源代码公开仓库（空 = 使用现有 workdir） |
| branch | string | 否 | 分支（空 = 探测默认分支） |
| git_ref | string | 否 | 指定提交 / 标签（可选） |
| git_subdir | string | 否 | 仓库内子目录（应用根不在仓库根时用） |
| git_depth | int | 否 | 浅克隆深度（0 = 不浅克隆） |
| build_output | string | 否 | 静态型构建产物目录（相对 workdir；空 = 执行端自动探测） |

## POST `/site/app/git-update`

拉取最新代码 + 重建 + 重启（即「应用重新部署」），最适合自动化

```bash
curl -X POST "https://<host>/api/site/app/git-update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "site_id": "<int>",
     "name": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |
| name | string | 是 | 应用名（站点内唯一） |

## POST `/site/app/action`

对应用执行启停动作

```bash
curl -X POST "https://<host>/api/site/app/action" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "site_id": "<int>",
     "name": "<string>",
     "action": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |
| name | string | 是 | 应用名 |
| action | string | 是 | start / stop / restart / enable / disable |

## POST `/site/app/remove`

移除应用（停止并删除进程配置；可选删数据）

```bash
curl -X POST "https://<host>/api/site/app/remove" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "site_id": "<int>",
     "name": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |
| name | string | 是 | 应用名 |

## GET `/site/app/log`

应用运行日志尾部（stdout / stderr）

```bash
curl -X GET "https://<host>/api/site/app/log" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |
| name | string | 是 | 应用名 |
| lines | int | 否 | 返回行数，默认 200 |

