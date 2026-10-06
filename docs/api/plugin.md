---
title: 插件 Plugin
---

# 插件 Plugin

> Lua 插件的安装 / 卸载、列表与运行；权限点 plugin:view / run / install / uninstall

## GET `/plugin/list`

插件列表（统一在系统级 $ZAP_PATH/plugins）

```bash
curl -X GET "https://<host>/api/plugin/list" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| slot | string | 否 | 按 ui.placement 槽位过滤，如 site.detail；留空返回全部 |
| scope | string | 否 | 按运行作用域过滤：site / system |

## GET `/plugin/ui`

读取插件自带的 HTML 界面内容（前端塞进沙箱 iframe 渲染）

```bash
curl -X GET "https://<host>/api/plugin/ui" \
  -H "Authorization: Bearer <token>"
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 是 | 插件名 |
| level | string | 否 | user（默认）/ system |

## POST `/plugin/run`

运行插件（异步插件返回 task_id + log_path）

```bash
curl -X POST "https://<host>/api/plugin/run" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "action": "<string>",
     "site_id": "<number>",
     "options": "<object>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 是 | 插件名（目录名） |
| action | string | 否 | manifest.actions 里的动作键，缺省 run |
| site_id | number | 否 | scope=site 插件必填，用于解析站点 root 与 Linux 账号 |
| options | object | 否 | 运行选项，键为 manifest.options 里的 name |

## POST `/plugin/install`

上传插件包安装（multipart：level / force / name / file）

```bash
curl -X POST "https://<host>/api/plugin/install" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "level": "<string>",
     "force": "<string>",
     "name": "<string>",
     "file": "<file>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| level | string | 否 | user（默认，登录用户可装）/ system（仅管理员） |
| force | string | 否 | true 时覆盖同名插件 |
| name | string | 否 | 插件名，留空取 manifest 里的 name |
| file | file | 是 | 插件包：.zip / .tar.gz / .tgz / .tar |

## POST `/plugin/install-git`

从 Git 仓库安装插件

```bash
curl -X POST "https://<host>/api/plugin/install-git" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "url": "<string>",
     "level": "<string>",
     "git_ref": "<string>",
     "name": "<string>",
     "force": "<bool>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| url | string | 是 | 仓库地址：http(s):// / ssh:// / git@ |
| level | string | 否 | user（默认）/ system（仅管理员） |
| git_ref | string | 否 | 分支 / 标签，留空用默认分支 |
| name | string | 否 | 插件名，留空取 manifest 里的 name |
| force | bool | 否 | 覆盖同名插件 |

## POST `/plugin/uninstall`

卸载插件（删除整个插件目录）

```bash
curl -X POST "https://<host>/api/plugin/uninstall" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "name": "<string>",
     "level": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| name | string | 是 | 插件名 |
| level | string | 否 | user（默认）/ system（仅管理员） |

## GET `/plugin/watch`

异步插件日志流（SSE，?token=&log_path=）

```bash
curl -X GET "https://<host>/api/plugin/watch" \
  -H "Authorization: Bearer <token>"
```

## POST `/plugin/cancel`

取消运行中的异步插件（写取消哨兵文件）

```bash
curl -X POST "https://<host>/api/plugin/cancel" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "token": "<string>",
     "task_id": "<string>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| token | string | 是 | JWT（EventSource 不能带自定义头，故走参数） |
| task_id | string | 是 | plugin/run 返回的 task_id |

