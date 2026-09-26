# 设计：四层转发（stream）/ PHP 能力 / 用户容器

状态：**已实现**（决策见 §0；改动清单见 §11）

## 0. 评审结论（已确认，实现以此为准）

| # | 结论 |
|---|---|
| 1 | 四层转发：**admin 组可用且可管理**，不进套餐，普通用户不可见/不可用 |
| 2 | `allow_php` **默认 1（PHP 默认拥有）**；存量不兼容 —— 套餐关掉 PHP 后，已有 PHP 站点同样改不动 |
| 3 | `allow_docker` **默认 0（不给用户分配）**；前端在开关旁明确提示风险（容器 ≈ 宿主机 root） |
| 4 | 容器能力**只有运行时是 Podman 才对非管理员生效**；Docker 运行时下即使开了 `allow_docker` 也不放行 |
| 5 | 面板**可以改 nginx.conf 主配置**（自动补 / 撤 `include zap-stream.conf;`，写前备份、失败回滚） |
| 6 | stream 后端地址**不限制**（管理员自愿承担风险） |
| 7 | **reseller 同样受 `allow_docker` 约束**（只有 admin 恒可） |
| 8 | docker 安装脚本**增加 Podman 选项**；「运行环境」里可显式设置当前运行时 `auto` / `docker` / `podman` |

## 1. 目标与范围

三件事，都挂在「谁能用什么」这条线上：

| # | 能力 | 控制方式 | 本期是否做隔离/迁移 |
|---|---|---|---|
| 1 | Nginx stream（四层转发） | **仅 admin 角色**（不进套餐） | 不做 |
| 2 | 新建站点时能否用 PHP | 套餐开关 `allow_php` | 不做 |
| 3 | 用户运行自己的容器 | 套餐开关 `allow_docker`，且**仅当运行时是 Podman** 才对普通用户开放 | 本期只加开关，**不做用户隔离**（容器仍共享 root） |

明确不做：

- **不为旧数据做兼容**：新增列按新语义直接生效，不写回填迁移、不给存量站点/存量用户留判断分支（例如"老站点就不校验 PHP"这类分支一律不写）。
- **不做容器隔离**：rootless、subuid/subgid、每用户 daemon、socket 代理鉴权都不在本期；本期只是"开关 + 运行时识别"，为后期 rootless Podman 留出接口。
- stream 基础模式只做 TCP/UDP 转发；TLS 终止、`ssl_preread`（SNI 分流）、
  `resolver` / `map` / 多后端 upstream 走**高级模式**（自定义片段），见 §6.1。

## 2. 现状（改动落点）

- 套餐：`packages` 表已有 `allow_ssh` / `allow_proxy` 两个能力开关，加列走 `ensure_column` 幂等补列：

```67:87:zapd/src/db/init_db.rs
/// 幂等补列：列已存在则跳过，否则 `ALTER TABLE ... ADD COLUMN`。
```

- 站点能力门禁的现成写法（`allow_proxy` 的门禁，admin/reseller 恒放行）：

```779:789:zapd/src/routers/site.rs
async fn gates_for(claims: &jwt::Claims) -> bool {
    if is_operator(claims) {
        return true;
    }
    match crate::routers::package::effective_package_of(claims.id as i64).await {
        Some(pkg) => pkg.allow_proxy == 1,
        None => false,
    }
}
```

- 站点类型：`php` / `static` / `proxy`，`site_add` 里归一化后落库：

```1836:1839:zapd/src/routers/site.rs
    let site_type = {
        let t = payload.site_type.trim().to_lowercase();
        norm_site_type(if t.is_empty() { "php" } else { &t })?
    };
```

- 容器命令全部走一个构造函数（硬编码 `docker`，共 9 处 `"docker"` 字面量），这是运行时抽象的改造中心：

```1586:1598:zapexec/src/verbs/docker.rs
fn docker_cmd<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new("docker");
```

- **stream 目前完全没有**：`zapexec` 只做 conf 目录树解析与站点 vhost 渲染（`<prefix>/conf/sites-enabled/zap-site-{id}.conf`），没有生成 `stream { }` 的代码。

## 3. 数据模型

### 3.1 packages 新增两列

```sql
ALTER TABLE packages ADD COLUMN allow_php    INTEGER NOT NULL DEFAULT 1;  -- 待确认：0 还是 1
ALTER TABLE packages ADD COLUMN allow_docker INTEGER NOT NULL DEFAULT 0;
```

（实际用 `ensure_column("packages", "allow_php", "INTEGER NOT NULL DEFAULT 1")` 幂等补列，不写迁移回填脚本。）

| 列 | 含义 | 建议默认 | admin/reseller |
|---|---|---|---|
| `allow_php` | 该套餐的用户新建站点时能否选 PHP 类型 | 1（与现状一致）**待确认** | 恒可，不看套餐 |
| `allow_docker` | 该套餐的用户能否使用容器功能 | 0 | 恒可，不看套餐 |

`PackageRow` 加 `allow_php: i32` / `allow_docker: i32`，`COLS`、`row_json`、add/update 的 `Option<bool>` 入参同步（照 `allow_proxy` 的写法）。

### 3.2 四层转发规则（新表）

```sql
CREATE TABLE nginx_stream (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         VARCHAR(64) NOT NULL UNIQUE,      -- 规则名，仅作标识
    listen_ip    TEXT NOT NULL DEFAULT '0.0.0.0',  -- 监听地址
    listen_port  INTEGER NOT NULL,                 -- 1..65535
    protocol     TEXT NOT NULL DEFAULT 'tcp',      -- tcp | udp
    target_host  TEXT NOT NULL,                    -- 后端地址
    target_port  INTEGER NOT NULL,
    remark       TEXT NOT NULL DEFAULT '',
    status       INTEGER NOT NULL DEFAULT 1,       -- 1 启用 / 0 停用（停用=不渲染）
    created_at   INTEGER,
    updated_at   INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_nginx_stream_port ON nginx_stream(listen_ip, listen_port, protocol);
```

规则名与后端地址都做字符校验（`target_host` 禁止空格/换行/引号等注入字符，`listen_ip` 只收 IP 字面量或 `0.0.0.0` / `::`）。

## 4. 权限矩阵

| 能力 | admin | reseller | 普通用户 |
|---|---|---|---|
| stream 管理 | ✅ | ❌（按你的要求只给 admin） | ❌ |
| 站点选 PHP | ✅ 恒可 | ✅ 恒可 | 套餐 `allow_php=1` |
| 容器功能 | ✅ 恒可 | 待确认（建议恒可，或同样受 `allow_docker` 约束） | `allow_docker=1` **且** 运行时是 Podman |

stream 走路由级角色门禁（`Required::Admin` + `is_admin` 二次校验），**不进套餐、不进权限点目录**。

## 5. 接口

### 5.1 套餐

- `POST /system/package/add`、`POST /system/package/update`：请求体新增 `allow_php?: boolean`、`allow_docker?: boolean`，缺省 `false`（update 未传不改）。
- `GET /system/package/list`：`row_json` 增加 `allow_php`、`allow_docker`。

### 5.2 站点（PHP 门禁）

`POST /site/add` 与 `POST /site/update`：归一化 `site_type` 之后、写库之前插入一道校验，失败返回明确文案：

```rust
// operator（admin/reseller）恒可；普通用户看套餐
async fn php_allowed_for(claims: &jwt::Claims) -> bool
```

错误文案：`当前账号未开放 PHP 站点（请联系管理员在「系统 → 套餐」中开启「PHP 站点」）`。

### 5.3 容器能力（运行时识别）

`docker.info`（zapexec 现有 capabilities 响应）增加字段：

```jsonc
{ "installed": true, "daemon": true, "runtime": "podman", "rootless": false, "user_usable": false, ... }
```

- `runtime`：`"docker"` | `"podman"` | `""`（都没装）
- `rootless`：`podman info` / 运行身份判断（本期只上报，不用于门禁）
- `user_usable`：zapd 算好的"普通用户是否可用"，前端直接用

zapd 侧新增门禁函数（admin 直通；否则看套餐 + 运行时）：

```rust
async fn docker_usable_by(claims: &jwt::Claims) -> bool {
    is_admin(claims)
        || (package_of(claims).allow_docker == 1 && container_runtime().await == Podman)
}
```

### 5.4 四层转发（新增，admin-only）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/nginx/stream/list` | 规则列表 |
| POST | `/nginx/stream/add` | 新建；写库 + 渲染配置 + `nginx -t` + reload，任一步失败回滚 |
| POST | `/nginx/stream/update` | 同上 |
| POST | `/nginx/stream/delete` | 删库 + 重新渲染 |
| POST | `/nginx/stream/apply` | 仅重新渲染并 reload（修坏了能一键重建） |

`add/update` 的 `listen_port` 冲突检测：与表内其它规则 + 已启用的站点监听端口（`listen` 80/443 及其它）+ 实际监听（`ss -lntu`）比对，冲突直接拒绝。

## 6. Nginx stream 配置

### 6.1 落盘位置

`stream { }` 必须位于 **main context**，不能塞进 `sites-enabled`（那个目录被 include 在 `http { }` 里）。方案：

- 面板渲染单文件：`<nginx prefix>/conf/zap-stream.conf`（幂等全量渲染，改一条就整体重写，便于回滚）
- nginx.conf 顶层需要一行：`include zap-stream.conf;`

启用 stream 时若主配置缺这行：备份原文件 → 在**顶层**插入 → `nginx -t` 校验 → 失败则还原备份并报错。**待确认**：是否允许面板改主配置（见 §9）。

### 6.2 模板

```nginx
# 由面板生成，勿手工修改 —— 修改请到「系统 → 四层转发」
stream {
    log_format zap_stream '$remote_addr [$time_local] $protocol $status '
                          '$bytes_sent $bytes_received $session_time $upstream_addr';
    access_log /var/log/nginx/zap-stream.log zap_stream;

    # rule: mysql-forward (id=3)
    upstream zap_stream_3 {
        server 10.0.0.5:3306 max_fails=3 fail_timeout=10s;
    }
    server {
        listen 0.0.0.0:13306;
        proxy_pass zap_stream_3;
        proxy_connect_timeout 5s;
        proxy_timeout 1h;
    }

    # rule: dns-udp (id=4)
    upstream zap_stream_4 { server 10.0.0.6:53; }
    server {
        listen 0.0.0.0:53 udp;
        proxy_pass zap_stream_4;
        proxy_responses 1;
        proxy_timeout 10s;
    }
}
```

说明：

- UDP 规则加 `udp` 参数，并按需设 `proxy_responses`。
- 禁用状态的规则不渲染；表里没规则时渲染空 `stream { }`（或直接不写文件并移除 include）。
- 每次渲染前先写临时文件 → `nginx -t` → 通过才替换 → reload；失败保留旧文件。

### 6.3 前置检查

- `nginx -V 2>&1` 必须含 `--with-stream`，否则 stream 功能整个禁用并提示（`nginx -V` 输出解析，缺模块时不渲染、不写 include）。
- 端口 <1024 可监听（nginx master 是 root），不做限制。

## 7. 容器运行时抽象（为后期 Podman 铺路）

### 7.1 改造点

把 `docker_cmd()` 换成运行时感知的构造函数：

```rust
enum Runtime { Docker, Podman }

fn container_cmd<I, S>(args: I) -> Command { /* Command::new(runtime_bin()) + args */ }
```

探测顺序（结果进程内缓存，可用 `ZAP_CONTAINER_RUNTIME=docker|podman` 覆盖）：

1. `podman version --format json` 成功 → Podman
2. `docker version` 成功 → Docker
3. 都没有 → 空（面板显示"未检测到容器运行时"）

### 7.2 命令差异（实现时逐条核对）

| 场景 | Docker | Podman | 处理 |
|---|---|---|---|
| 引擎 API | `bollard` 直连 `/var/run/docker.sock` | 走 `podman system service` 的兼容 socket（或 CLI） | 本期：Podman 时 CLI 优先，API 调用标注差异 |
| compose | `docker compose`（插件） | `podman compose`（podman-compose / 5.x 内置） | 能力探测 `compose_available()` 按运行时判断 |
| 列举容器 | `ps -a` 一致 | `ps -a` 一致 | 无差异 |
| 日志 | `logs --tail N` 一致 | 一致 | 无差异 |
| 网络/卷 | 一致 | 基本一致（rootless 下网络为 slirp4netns） | 展示层标注 |
| 运行身份 | root daemon | rootless 时 `${XDG_RUNTIME_DIR}/podman/podman.sock` | 本期只上报 `rootless` |

### 7.3 本期的门禁语义（按你的要求）

```
runtime = Docker  → 非 admin 一律不可用（容器仍是 root 跑，没有隔离，不放开）
runtime = Podman  → 被授予 allow_docker 的普通用户可用
```

即：Docker 环境下 `allow_docker` 对普通用户**不生效**（前端置灰并提示"当前为 Docker 运行时，用户容器仅在 Podman（rootless）环境开放"）。

## 8. 前端改动点

- `web/src/views/system/packages/index.vue`：两个开关（`allow_php` / `allow_docker`）+ 列表列 + 表单默认值，照 `allow_ssh` / `allow_proxy` 的写法。
- `web/src/views/site/…`：站点类型选择项在不可用时禁用 + 提示（后端兜底报错）。
- Docker 页：`user_usable=false` 时非 admin 看到置灰提示，说明当前运行时。
- 新增 stream 页面（仅 admin 菜单可见）：表格 + 新增/编辑弹窗（监听地址、端口、协议、目标地址、目标端口、备注）+ 「重新应用配置」。
- i18n：`zh-CN.ts` / `en-US.ts` 增加套餐开关文案、stream 页面文案、错误提示。

## 9. 风险与待确认

1. ~~`allow_php` 默认值取 1 还是 0~~ → **已定：1**（PHP 默认拥有）。
2. ~~存量 PHP 站点~~ → **已定：一刀切**，套餐关 PHP 后已有 PHP 站点也改不动（不写"老站点例外"分支）。
3. ~~面板能否改 nginx.conf 主配置~~ → **已定：允许**，自动补 / 撤 include，写前备份、`nginx -t` 失败即回滚。
4. ~~stream 后端地址是否限制~~ → **已定：不限制**。
5. ~~reseller 是否受 `allow_docker` 约束~~ → **已定：受约束**（只有 admin 恒可）。
6. **Podman 由 root 运行时**（非 rootless）要不要算"可开放"？建议不算（隔离没到位），本期 `user_usable` 只按 `runtime == podman` 判断，后续再加 `rootless` 条件。

## 10. 验收

- 单测：`php_allowed_for` 的 admin/套餐矩阵；`container_cmd` 运行时选择（可注入）；stream 配置渲染（含 UDP、停用规则、空规则集）；端口冲突检测。
- 手工：Docker 机器上普通用户看不到/用不了容器；Podman 机器上开了 `allow_php` / `allow_docker` 的普通用户可建 PHP 站点、可用容器；stream 规则增删改后 `nginx -t` 通过且 `ss -lntp` 能看到监听；渲染失败时旧配置不被破坏。

## 11. 改动清单（已落地）

**数据模型**
- `packages`：`allow_php`（默认 1）/ `allow_docker`（默认 0），走 `ensure_column` 增量补列 + 新建表 DDL。
- `nginx_stream` 新表：规则（监听 IP/端口/协议 → 后端地址/端口），`UNIQUE(listen_ip, listen_port, protocol)`。

**后端**
- `zapd/src/routers/package.rs`：两个开关进 `COLS` / `row_json` / add / update。
- `zapd/src/routers/site.rs`：`php_allowed_for` + `require_php_allowed`，在 `site_add` / `site_update` 归一化类型后拦一道（operator 恒放行）。
- `zapd/src/routers/docker.rs`：`require_user_docker` 加在 `require_container_view` / `require_container_manage` / `require_builder` 上 —— 非 admin 需同时满足「套餐 allow_docker」+「运行时 podman」+ 原权限点。
- `zapd/src/routers/system_stream.rs`（新）：stream 规则 CRUD + 渲染 + 下发，`/system/stream/*`  全部 `Required::Admin`。
  支持两种模式：**basic**（后端模式 = 单后端直接 `proxy_pass` / 负载组 upstream；超时、listen 参数、
  响应包数、监听侧 TLS 终止、`ssl_preread`+`proxy_pass` 变量、server 内附加指令都在基础表单上）
  与 **advanced**（`raw`：stream 块内任意配置，`map` + SNI 分流这类写法直接贴）；
  监听侧 TLS 的证书可**从证书库选**（`ssl_certificate_id`，应用时把 PEM 落盘到
  `{data}/ssl/stream/zap-stream-{id}.crt|.key`，私钥 0600，规则停用/删除后清理），也可手工填路径；
  另有**全局自定义片段** `nginx_stream_global`（单行表，`resolver` / `map` / 公共 `upstream`），
  渲染时排在规则之前供其引用 —— UI 放在**服务配置 → Nginx** 页（不是四层转发页）。
  自定义片段统一挡掉嵌套 `stream { }`，落盘前仍由 `nginx -t` 兜底回滚。
- `zapd/src/routers/system_env.rs` + `zapexec/src/verbs/docker.rs`：`container_runtime` 设置项（`auto` / `docker` / `podman`），exec 侧 `runtime()` 决定 CLI、socket、installed 判定与 compose 探测。
- `zapexec/src/verbs/nginx.rs`：`NginxStreamStatus` / `NginxStreamApply` —— 写盘 → `nginx -t` → 失败回滚 → reload；主配置缺 include 时自动补、撤规则时自动移除。
- `zapd/src/db/menu_seed.rs`：服务器配置下新增「四层转发」菜单（R_ADMIN，仅新库随种子建出；存量库需手工补一条，按"不做兼容"处理）。

**前端**
- 套餐页：新增「PHP 站点」/「容器」两列与开关，容器开关带风险提示文案。
- 运行环境页：新增「容器运行时」选择（auto / docker / podman）+ 说明。
- 新增 `web/src/views/server/stream/index.vue`（页面 + `api/stream.ts` + i18n）。

**安装脚本**
- `infra/docker/install.sh`：新增 `INSTALL_RUNTIME` 选项，`podman` 走系统包管理器安装（含 podman-compose、podman.socket），装完提示到面板把运行时设为 podman。
- `infra/docker/uninstall.sh`：检测到只有 podman 时走对应卸载分支（数据目录默认保留）。
- `infra/docker/app.yaml`：新增 `INSTALL_RUNTIME` 选项。

**未做（本期范围外）**：容器用户隔离（rootless / subuid / 每用户 daemon）、端口冲突检测（stream 规则 vs 站点监听 vs `ss`）、stream 的 TLS 终止与 SNI 分流。
