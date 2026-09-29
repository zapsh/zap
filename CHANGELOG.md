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

- (same series) JWT startup fails when the default key is missing (fail-closed); advanced proxy `raw` body and custom `rewrite` rules are hardened against `include` / system-path / cloud-metadata injection.

## [v1.0.10] - Release Date : 2026-9-14

* 增加i18n
* 增加zh-CN和en-US

