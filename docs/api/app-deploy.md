---
title: 应用部署（App Deploy）
---

# 应用部署（App Deploy）

> 本组接口用于站点的「应用」部署 / 重新部署 / 启停，均要求 `Authorization: Bearer <token>`，
> token 可以是**用户 JWT**，也可以是**开发页创建的 API Token（PAT）**；且调用者必须对 `site_id` 有访问权（后端 `site_in_scope` 校验）。

以下接口特别适合被「外部 Webhook 触发器」驱动自动化（见插件触发器里的「应用重新部署」「站点代码同步」等能力）。

## POST `/site/app/git-update`

拉取应用 git 仓库最新代码 + 重建 + 重启（即「应用重新部署」）。只需站点与应用名，**最适合自动化**。

```bash
curl -X POST "https://<host>/api/site/app/git-update" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{ "site_id": 12, "name": "myapp" }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |
| name | string | 是 | 应用名（站点内唯一） |

返回 `{ code: 0, task_id }`，`task_id` 可用于查后台部署日志。

## POST `/site/app/action`

对应用执行启停等动作（`start` / `stop` / `restart` / `enable` / `disable`）。

```bash
curl -X POST "https://<host>/api/site/app/action" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{ "site_id": 12, "name": "myapp", "action": "restart" }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID |
| name | string | 是 | 应用名 |
| action | string | 是 | `start` / `stop` / `restart` / `enable` / `disable` |

## POST `/site/app/deploy`

完整部署（创建 / 更新应用并构建）。参数较多，**一般不适合纯自动化**，多用于首次上线或复杂配置。

```bash
curl -X POST "https://<host>/api/site/app/deploy" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "site_id": 12,
    "name": "myapp",
    "app_type": "nodejs",
    "runtime_version": "20",
    "build_cmd": "npm run build",
    "command": "npm start",
    "port": 3000,
    "env": "NODE_ENV=production\nDEBUG=1",
    "autostart": true,
    "install_deps": true
  }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| site_id | int | 是 | 站点 ID（与 `domain` 二选一） |
| name | string | 是 | 应用名（站点内唯一），仅字母数字 `-_.` 且 ≤48 |
| app_type | string | 是 | `python` / `nodejs` / `go` / `static` / `generic` 等（受套餐 `caps.types` 限制） |
| runtime_version | string | 否 | 运行时版本（如 `3.11` / `20`），空 = 系统默认 |
| build_cmd | string | 否 | 部署时先执行的构建命令，空 = 不构建 |
| create_venv | bool | 否 | python：是否生成 `.venv`（默认开） |
| workdir | string | 否 | 工作目录，空 = 站点 `web_root` |
| entry | string | 否 | 入口 |
| command | string | 否 | 自定义启动命令（非空则覆盖类型默认模板；`generic` 必填） |
| port | int | 否 | 端口；配合 `auto_port` 可自动分配 |
| env | string | 否 | 环境变量，每行一条 `KEY=VALUE` |
| autostart | bool | 否 | 是否开机/部署后自启（默认 true） |
| install_deps | bool | 否 | 是否安装依赖（pip / npm） |
| auto_port | bool | 否 | 自动分配端口（true 时忽略 `port`） |
| domain | string | 否 | 填了自动创建反代站点（`site_type=proxy`）；不填 `site_id` 时用它建站 |
| mount_path | string | 否 | 站点上反代挂载前缀，默认 `/` |
| match_mode | string | 否 | 匹配方式：空=前缀 / `exact`=精确 / `prefer`=优先前缀 |
| strip_prefix | bool | 否 | 是否剥离挂载前缀再转发 |
| repo_url | string | 否 | 公开仓库地址（空 = 使用现有 workdir） |
| branch | string | 否 | 分支（空 = 探测默认分支） |

> 提示：以上接口经 Bearer 鉴权后，可直接被 git 插件的 `redeploy` 动作（内部 `curl` 调用 `/site/app/git-update`）
> 在 Webhook 触发器里复用，无需在端点上混用鉴权方案。
