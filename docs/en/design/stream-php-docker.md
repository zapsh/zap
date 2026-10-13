---
title: Stream / PHP / User Containers
---

# Design: Layer-4 Forwarding (stream) / PHP Capability / User Containers

Status: **Implemented** (decisions in §0; change list in §11)

## 0. Review conclusions (confirmed — implementation follows these)

| # | Conclusion |
|---|---|
| 1 | Layer-4 forwarding: **available and manageable for the admin group**, not part of packages, invisible/unusable to regular users |
| 2 | `allow_php` **defaults to 1 (PHP enabled by default)**; no backward compatibility — once a package disables PHP, existing PHP sites cannot be modified either |
| 3 | `allow_docker` **defaults to 0 (not granted to users)**; the frontend must state the risk next to the toggle (a container ≈ root on the host) |
| 4 | Container capability **only takes effect for non-admins when the runtime is Podman**; with the Docker runtime it is not granted even if `allow_docker` is on |
| 5 | The panel **may modify the main nginx.conf** (automatically add / remove `include zap-stream.conf;`, back up before writing, roll back on failure) |
| 6 | stream backend addresses are **not restricted** (the administrator knowingly accepts the risk) |
| 7 | **resellers are bound by `allow_docker` too** (only admin always passes) |
| 8 | the Docker install script **gains a Podman option**; the current runtime can be set explicitly in "Runtime Environment" (`auto` / `docker` / `podman`) |

## 1. Goals and scope

Three items, all on the "who may use what" axis:

| # | Capability | Control | Isolation/migration in this iteration |
|---|---|---|---|
| 1 | Nginx stream (layer-4 forwarding) | **admin role only** (not part of packages) | No |
| 2 | Whether PHP can be used when creating a site | package toggle `allow_php` | No |
| 3 | Users running their own containers | package toggle `allow_docker`, and **only when the runtime is Podman** for regular users | This iteration only adds the toggle — **no user isolation** (containers still share root) |

Explicitly out of scope:

- **No compatibility for legacy data**: new columns take effect under the new semantics immediately; no backfill
  migration, no branches that special-case existing sites/users (e.g. "old sites skip the PHP check" is never written).
- **No container isolation**: rootless, subuid/subgid, per-user daemons and socket-proxy authentication are all out
  of scope. This iteration is just "toggle + runtime detection", leaving the interface open for rootless Podman later.
- The stream basic mode only does TCP/UDP forwarding; TLS termination, `ssl_preread` (SNI-based routing),
  `resolver` / `map` / multi-backend upstreams belong to **advanced mode** (custom snippets) — see §6.1.

## 2. Current state (where changes land)

- Packages: the `packages` table already has two capability toggles, `allow_ssh` / `allow_proxy`; new columns go
  through idempotent `ensure_column`:

*zapd/src/db/init_db.rs:67*

```rust
/// Idempotent column add: skip if the column exists, otherwise `ALTER TABLE ... ADD COLUMN`.
```

- The existing pattern for site capability gates (the `allow_proxy` gate; admin/reseller always pass):

*zapd/src/routers/site.rs:779*

```rust
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

- Site type: `php` / `static` / `proxy`, normalized in `site_add` before persisting:

*zapd/src/routers/site.rs:1836*

```rust
    let site_type = {
        let t = payload.site_type.trim().to_lowercase();
        norm_site_type(if t.is_empty() { "php" } else { &t })?
    };
```

- All container commands go through a single constructor (hard-coded `docker`, 9 occurrences of the `"docker"` literal) —
  this is the focus of the runtime abstraction work:

*zapexec/src/verbs/docker.rs:1586*

```rust
fn docker_cmd<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new("docker");
```

- **stream does not exist at all yet**: `zapexec` only parses the conf directory tree and renders site vhosts
  (`<prefix>/conf/sites-enabled/zap-site-{id}.conf`); there is no code generating a `stream { }` block.

## 3. Data model

### 3.1 Two new columns on `packages`

```sql
ALTER TABLE packages ADD COLUMN allow_php    INTEGER NOT NULL DEFAULT 1;  -- TBD: 0 or 1
ALTER TABLE packages ADD COLUMN allow_docker INTEGER NOT NULL DEFAULT 0;
```

(In practice using idempotent `ensure_column("packages", "allow_php", "INTEGER NOT NULL DEFAULT 1")`; no backfill script.)

| Column | Meaning | Suggested default | admin/reseller |
|---|---|---|---|
| `allow_php` | Whether users on this package may choose the PHP type when creating a site | 1 (matches current behavior) **TBD** | always allowed, package ignored |
| `allow_docker` | Whether users on this package may use container features | 0 | always allowed, package ignored |

`PackageRow` gains `allow_php: i32` / `allow_docker: i32`; `COLS`, `row_json` and the `Option<bool>` inputs of
add/update follow suit (mirroring how `allow_proxy` is written).

### 3.2 Layer-4 forwarding rules (new table)

```sql
CREATE TABLE nginx_stream (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         VARCHAR(64) NOT NULL UNIQUE,      -- rule name, identifier only
    listen_ip    TEXT NOT NULL DEFAULT '0.0.0.0',  -- listen address
    listen_port  INTEGER NOT NULL,                 -- 1..65535
    protocol     TEXT NOT NULL DEFAULT 'tcp',      -- tcp | udp
    target_host  TEXT NOT NULL,                    -- backend address
    target_port  INTEGER NOT NULL,
    remark       TEXT NOT NULL DEFAULT '',
    status       INTEGER NOT NULL DEFAULT 1,       -- 1 enabled / 0 disabled (disabled = not rendered)
    created_at   INTEGER,
    updated_at   INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_nginx_stream_port ON nginx_stream(listen_ip, listen_port, protocol);
```

Both the rule name and the backend address are character-validated (`target_host` rejects spaces, newlines, quotes
and other injection characters; `listen_ip` accepts only an IP literal or `0.0.0.0` / `::`).

## 4. Permission matrix

| Capability | admin | reseller | regular user |
|---|---|---|---|
| stream management | ✅ | ❌ (admin only, as requested) | ❌ |
| Choosing PHP for a site | ✅ always | ✅ always | package `allow_php=1` |
| Container features | ✅ always | TBD (suggested: always, or bound by `allow_docker` as well) | `allow_docker=1` **and** runtime is Podman |

stream uses a route-level role gate (`Required::Admin` plus a secondary `is_admin` check); it is **not part of
packages and not part of the permission-point catalog**.

## 5. Interfaces

### 5.1 Packages

- `POST /system/package/add`, `POST /system/package/update`: request body gains `allow_php?: boolean`,
  `allow_docker?: boolean`, defaulting to `false` (omitted on update means unchanged).
- `GET /system/package/list`: `row_json` gains `allow_php` and `allow_docker`.

### 5.2 Sites (PHP gate)

`POST /site/add` and `POST /site/update`: insert a check after normalizing `site_type` and before writing to the
database; on failure return an explicit message:

```rust
// operator (admin/reseller) always passes; regular users depend on their package
async fn php_allowed_for(claims: &jwt::Claims) -> bool
```

Error text: `PHP sites are not enabled for this account (ask an administrator to enable "PHP sites" under System → Packages)`.

### 5.3 Container capability (runtime detection)

`docker.info` (the capabilities response already produced by zapexec) gains fields:

```jsonc
{ "installed": true, "daemon": true, "runtime": "podman", "rootless": false, "user_usable": false, ... }
```

- `runtime`: `"docker"` | `"podman"` | `""` (neither installed)
- `rootless`: derived from `podman info` / the running identity (reported only, not used for gating in this iteration)
- `user_usable`: computed by zapd — "is this usable by a regular user"; the frontend uses it directly

New gate function on the zapd side (admin passes through; otherwise package + runtime):

```rust
async fn docker_usable_by(claims: &jwt::Claims) -> bool {
    is_admin(claims)
        || (package_of(claims).allow_docker == 1 && container_runtime().await == Podman)
}
```

### 5.4 Layer-4 forwarding (new, admin-only)

| Method | Path | Description |
|---|---|---|
| GET | `/nginx/stream/list` | Rule list |
| POST | `/nginx/stream/add` | Create; write DB + render config + `nginx -t` + reload, rollback if any step fails |
| POST | `/nginx/stream/update` | Same as above |
| POST | `/nginx/stream/delete` | Delete from DB + re-render |
| POST | `/nginx/stream/apply` | Re-render and reload only (one-click rebuild if something got broken) |

`add/update` conflict detection for `listen_port`: compare against other rules in the table + listening ports of
enabled sites (`listen` 80/443 and others) + actual listeners (`ss -lntu`); conflicts are rejected outright.

## 6. Nginx stream configuration

### 6.1 Where it is written

`stream { }` must live in the **main context**; it cannot go into `sites-enabled` (that directory is included
inside `http { }`). Approach:

- The panel renders a single file: `<nginx prefix>/conf/zap-stream.conf` (idempotent full render — changing one rule
  rewrites the whole file, which makes rollback easy)
- `nginx.conf` needs one top-level line: `include zap-stream.conf;`

When enabling stream, if that line is missing from the main config: back up the original → insert it at the
**top level** → verify with `nginx -t` → restore the backup and report an error on failure. **TBD**: whether the
panel is allowed to modify the main config at all (see §9).

### 6.2 Template

```nginx
# Generated by the panel — do not edit by hand; go to "System → Layer-4 Forwarding" instead
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

Notes:

- UDP rules add the `udp` parameter and set `proxy_responses` as needed.
- Disabled rules are not rendered; when the table has no rules, an empty `stream { }` is rendered
  (or the file is not written at all and the include is removed).
- Each render writes a temporary file first → `nginx -t` → replaces only if it passes → reload;
  on failure the old file is kept.

### 6.3 Preconditions

- `nginx -V 2>&1` must contain `--with-stream`, otherwise the whole stream feature is disabled with a notice
  (parse the `nginx -V` output; when the module is missing, do not render and do not write the include).
- Ports below 1024 can be listened on (the nginx master runs as root); no restriction.

## 7. Container runtime abstraction (paving the way for Podman)

### 7.1 What changes

Replace `docker_cmd()` with a runtime-aware constructor:

```rust
enum Runtime { Docker, Podman }

fn container_cmd<I, S>(args: I) -> Command { /* Command::new(runtime_bin()) + args */ }
```

Detection order (result cached in-process; overridable with `ZAP_CONTAINER_RUNTIME=docker|podman`):

1. `podman version --format json` succeeds → Podman
2. `docker version` succeeds → Docker
3. Neither → empty (the panel shows "no container runtime detected")

### 7.2 Command differences (verify each one during implementation)

| Scenario | Docker | Podman | Handling |
|---|---|---|---|
| Engine API | `bollard` talks to `/var/run/docker.sock` directly | Compatible socket via `podman system service` (or the CLI) | This iteration: CLI-first under Podman, API calls annotated with the differences |
| compose | `docker compose` (plugin) | `podman compose` (podman-compose / built into 5.x) | capability detection `compose_available()` per runtime |
| List containers | `ps -a` identical | `ps -a` identical | No difference |
| Logs | `logs --tail N` identical | identical | No difference |
| Networks/volumes | identical | largely identical (slirp4netns networking when rootless) | annotate in the UI |
| Run identity | root daemon | `${XDG_RUNTIME_DIR}/podman/podman.sock` when rootless | only report `rootless` in this iteration |

### 7.3 Gating semantics for this iteration (as requested)

```
runtime = Docker  → unavailable to everyone but admin (containers still run as root, no isolation, so not granted)
runtime = Podman  → regular users granted allow_docker may use it
```

That is: under Docker, `allow_docker` has **no effect** for regular users (the UI greys it out with the hint
"Current runtime is Docker; user containers are only available on Podman (rootless)").

## 8. Frontend changes

- `web/src/views/system/packages/index.vue`: two toggles (`allow_php` / `allow_docker`) + list columns + form
  defaults, mirroring `allow_ssh` / `allow_proxy`.
- `web/src/views/site/…`: disable the site-type selector with a hint when unavailable (the backend enforces it too).
- Docker page: when `user_usable=false`, non-admins see a greyed-out hint stating the current runtime.
- New stream page (visible to admin menus only): table + create/edit dialog (listen address, port, protocol,
  target address, target port, remark) + "Re-apply configuration".
- i18n: `zh-CN.ts` / `en-US.ts` gain the package toggle texts, stream page texts and error messages.

## 9. Risks and open questions

1. ~~Should `allow_php` default to 1 or 0~~ → **decided: 1** (PHP enabled by default).
2. ~~Existing PHP sites~~ → **decided: hard cutover**; once a package disables PHP, existing PHP sites cannot be
   modified either (no "old sites are exempt" branch).
3. ~~May the panel modify the main nginx.conf~~ → **decided: yes**; add/remove the include automatically, back up
   before writing, roll back if `nginx -t` fails.
4. ~~Should stream backend addresses be restricted~~ → **decided: no**.
5. ~~Are resellers bound by `allow_docker`~~ → **decided: yes** (only admin always passes).
6. Should Podman **running as root** (not rootless) count as "grantable"? Suggested: no (isolation is not in place).
   In this iteration `user_usable` only checks `runtime == podman`; the `rootless` condition comes later.

## 10. Acceptance

- Unit tests: the admin/package matrix of `php_allowed_for`; runtime selection of `container_cmd` (injectable);
  stream config rendering (including UDP, disabled rules, empty rule set); port conflict detection.
- Manual: on a Docker machine a regular user cannot see or use containers; on a Podman machine a regular user with
  `allow_php` / `allow_docker` can create PHP sites and use containers; after adding/editing/deleting stream rules
  `nginx -t` passes and `ss -lntp` shows the listener; a failed render leaves the old configuration intact.

## 11. Change list (implemented)

**Data model**
- `packages`: `allow_php` (default 1) / `allow_docker` (default 0), added via incremental `ensure_column` plus
  the create-table DDL.
- New `nginx_stream` table: rules (listen IP/port/protocol → backend address/port),
  `UNIQUE(listen_ip, listen_port, protocol)`.

**Backend**
- `zapd/src/routers/package.rs`: both toggles added to `COLS` / `row_json` / add / update.
- `zapd/src/routers/site.rs`: `php_allowed_for` + `require_php_allowed`, applied in `site_add` / `site_update` after
  type normalization (operators always pass).
- `zapd/src/routers/docker.rs`: `require_user_docker` layered onto `require_container_view` /
  `require_container_manage` / `require_builder` — non-admins must satisfy "package allow_docker" +
  "runtime is podman" + the original permission point.
- `zapd/src/routers/system_stream.rs` (new): stream rule CRUD + rendering + dispatch; all `/system/stream/*` routes
  are `Required::Admin`. Supports two modes: **basic** (backend mode = single backend direct `proxy_pass` or a
  load-balanced upstream group; timeouts, listen parameters, response packet count, listen-side TLS termination,
  `ssl_preread`+`proxy_pass` variables and extra in-server directives are all on the basic form) and **advanced**
  (`raw`: arbitrary configuration inside the stream block — `map` + SNI routing can be pasted directly).
  Listen-side TLS certificates can be **picked from the certificate store** (`ssl_certificate_id`; on apply the PEM
  is written to `{data}/ssl/stream/zap-stream-{id}.crt|.key` with mode 0600 on the key, cleaned up when the rule is
  disabled or deleted), or a path can be entered manually. There is also a **global custom snippet**
  `nginx_stream_global` (single-row table, for `resolver` / `map` / shared `upstream`) rendered before the rules so
  they can reference it — its UI lives on the **Service Configuration → Nginx** page (not the layer-4 page).
  Custom snippets uniformly reject nested `stream { }`, and `nginx -t` still backs up/rolls back before writing.
- `zapd/src/routers/system_env.rs` + `zapexec/src/verbs/docker.rs`: `container_runtime` setting
  (`auto` / `docker` / `podman`); on the exec side `runtime()` decides the CLI, socket, installed detection and
  compose detection.
- `zapexec/src/verbs/nginx.rs`: `NginxStreamStatus` / `NginxStreamApply` — write to disk → `nginx -t` → roll back on
  failure → reload; automatically add the include when missing from the main config and remove it when the last rule
  is withdrawn.
- `zapd/src/db/menu_seed.rs`: new "Layer-4 Forwarding" menu under server configuration (R_ADMIN; created by the seed
  only on fresh databases — existing databases need one row added manually, per the "no compatibility" rule).

**Frontend**
- Packages page: two new columns/toggles, "PHP sites" and "Containers", with a risk notice on the container toggle.
- Runtime Environment page: new "Container runtime" selector (auto / docker / podman) plus an explanation.
- New `web/src/views/server/stream/index.vue` (page + `api/stream.ts` + i18n).

**Install scripts**
- `infra/docker/install.sh`: new `INSTALL_RUNTIME` option; `podman` installs via the system package manager
  (including podman-compose and podman.socket), and reminds you to set the runtime to podman in the panel afterwards.
- `infra/docker/uninstall.sh`: takes the matching uninstall branch when only podman is detected (data directories
  are kept by default).
- `infra/docker/app.yaml`: new `INSTALL_RUNTIME` option.

**Not done (out of scope for this iteration)**: container user isolation (rootless / subuid / per-user daemon),
port conflict detection (stream rules vs site listeners vs `ss`), stream TLS termination and SNI routing.
