# Zap Changelog

## [v1.0.18] - Release Date : 2026-9-29

### Security Hardening

- **#6 Privilege check: `zapexec` trusted `zapd` authorization (Medium)** — the exec side no longer blindly trusts panel authorization; two independent checks added (defense-in-depth):
  - **File-op path sandbox**: before running as root, `list` / `read` / `write` / `delete` / `mkdir` / `rename` / `download` / `upload` / `chmod` / `chown` / `copy` / `archive` / `info` re-validate that a non-admin user's resolved path stays inside the user's own **home dir (`pw_dir` from `/etc/passwd`) / private tmp `/tmp/zap-<user>` / panel data dir `data/users/<user>`**; existing paths are additionally `canonicalize`d to block symlink escapes (e.g. `/home/u/evil -> /etc`). Admin/root behavior unchanged.
  - **App deploy ownership check**: `AppDeploy` carries the requester identity; `deploy` requires a non-admin user's `owner_user` to equal the requester, preventing apps from being launched as another site's account (cross-tenant tampering / privilege escalation). Admin is exempt (`skip_owner_check`); site ownership still enforced by `zapd`.
  - Non-admin users without a bound system account are now rejected instead of running as root.

- **#7 Blocking I/O inside async handlers (High)** — a blocking `std::fs` call on a tokio worker stalls the whole async pool:
  - Replaced every blocking file op in `routers/**` (site log-dir `rename`; appstore `provision`/`info.yaml` reads and script-tree traversal; phpMyAdmin FPM socket probe `read_dir`/`metadata`/`canonicalize`; stream-cert `create_dir_all`/`write`/`set_permissions`/`read_dir`/`remove_file`; Zap settings cert read/self-sign/delete; MySQL `my.cnf` parse; upload tmp-dir create and chmod current-mode read) with `tokio::fs` (async path) or `tokio::task::spawn_blocking`.
  - `parse_mycnf` is now async and cached via `tokio::sync::OnceCell`; recursive reads go through `tokio::fs`.
  - Added a clippy guard: root `clippy.toml` uses `disallowed-types` to forbid `routers/**` from directly using the blocking `std::fs` I/O types (`File`/`OpenOptions`/`ReadDir`/`DirEntry`/`FileType`), enforced by `#![deny(clippy::disallowed_types)]` at the top of `routers/mod.rs` (test builds exempt). `Metadata`/`Permissions` are intentionally allowed since they are returned/required by `tokio::fs`.

- **#9 Write-path `unwrap_or(false)` masks uniqueness check (Medium)** — at `system_stream.rs:968/1139` a failed duplicate-port check defaulted to "not duplicate", allowing two rules to bind the same `listen_ip:port` and causing `nginx -t` failure / inconsistent state; the DB error is now propagated with `?` so a check failure returns an error instead of silently allowing the write.

- **#12 Inconsistent response envelope (Medium)** — ~35 `json!({code})` sites varied in shape (some missing `message`, `database.rs` used a private `ok()` with no `message`, a few nested `ok:true` in `data`). Added unified envelope helpers `zap::api_ok` / `api_ok_msg` / `api_ok_data` / `api_err` (success is always `{code:0,message,data}`, error `{code,message}`); `database.rs`'s private `ok()` now routes through `api_ok`, and the `code:0,data`-without-`message` success responses were migrated to `api_ok`. Note: the `data.ok` field in database responses is retained for the frontend `DbStatus.ok` etc. contract.

- **#15 CI noise / gaps (Medium)** — `ci.yml`: removed the dead step that created a `zappro` placeholder `mod.rs` (no `Cargo.toml` references it and CI never enables the `commercial` feature); `web/tsconfig.node.json`: dropped the dangling `eslint.config.*` (no eslint dependency in the project); `release.yml`: pinned `cross` to the published `0.2.5` instead of git HEAD, and pinned all third-party actions (`checkout`, `setup-node`, `cache`, `rust-toolchain`, `upload-artifact`, `download-artifact`, `action-gh-release`) to their full commit SHAs (supply-chain hardening).

- **#19 README out of sync with code (Low)** — the README listed Docker container management and the app-store plugin ecosystem as Roadmap, but both are already shipped (`routers/docker.rs` with container/image/network/Compose management plus Docker/Podman runtime awareness, and a full `web/src/views/docker/` UI; the App Store is documented in `docs/api/appstore.md`). The README core-features bullet and Roadmap section now reflect the delivered state.

- (same series) JWT startup fails when the default key is missing (fail-closed); advanced proxy `raw` body and custom `rewrite` rules are hardened against `include` / system-path / cloud-metadata injection.

## [v1.0.10] - Release Date : 2026-9-14

* 增加i18n
* 增加zh-CN和en-US


### Plugin platform

- **Host UIKit**: `data/plugins/_lib/ui.css` / `ui.js` (shipped with the release — also fixed `.gitignore` and the packaging scripts dropping them) are auto-injected into a plugin's HTML by `plugin_ui`, providing theme variables + `.zui-*` component classes and the `zap.ui.*` runtime (`notify` / `confirm` / `prompt` / `toast` / `table` / `tabs` / `diff` / `busy` / `lang` / `t` / `localize`).
- **Multi-value slots + two new slots**: `ui.placement` accepts a list (e.g. both `file.editor` and `file.context`), plus new `file.context` (file-manager right-click menu, passing the selected files) and `dashboard.card` (admin dashboard card).
- **Per-action `async` / `dangerous`**: destructive actions get a second confirmation on the frontend **and are rejected server-side for read-only demo accounts**; only the selected long-running actions stream logs / stay cancelable.
- **Plugin i18n (optional)**: manifests accept an `i18n` table (`zh-CN` / `en-US`) overriding title / description / entry label / action names / option labels. **Nothing is mandatory here**: plugins without translations keep displaying their base copy, and missing keys fall back individually, so partial translations are fine. The language follows Element Plus (switching it reloads the list and any open plugin UI). UIKit exposes `zap.ui.lang` / `t()` / `localize()` for self-hosted HTML UIs.
- **Scheduled / webhook triggers**: the plugin manager can attach `cron` or `webhook` triggers to any plugin. Triggers run **as their owner** (same account, roles and permission checks as pressing the button manually); the public `/api/plugin/hook/<token>` endpoint authenticates via a random token and supports disabling / rotating it.
- **Per-plugin persistent config**: new `zap.config.get/set/number/bool`, bucketed per plugin + panel user (`$ZAP_PATH/data/plugins/config/<plugin>.yaml`) — Lua finally has somewhere to keep preferences and small credentials.
- **Network access**: the shared library gained `zap.http.get/post/request/json/download` (backed by `curl`, inheriting scope de-escalation, http/https only).
- **Requirements & signature**: manifests support `requires.commands` (install is refused when a command is missing) and `signature` (HMAC-SHA256 digest of the plugin files; tampered packages are refused).
- **Smoke tests**: drop a `tests.yaml` in the plugin directory to run each action and assert on log output; results (pass / fail / skipped) are viewable from the plugin manager.
