# Zap Changelog

## [v1.0.18] - Release Date : 2026-9-29

### Security Hardening

- **#6 Privilege check: `zapexec` trusted `zapd` authorization (Medium)** — the exec side no longer blindly trusts panel authorization; two independent checks added (defense-in-depth):
  - **File-op path sandbox**: before running as root, `list` / `read` / `write` / `delete` / `mkdir` / `rename` / `download` / `upload` / `chmod` / `chown` / `copy` / `archive` / `info` re-validate that a non-admin user's resolved path stays inside the user's own **home dir (`pw_dir` from `/etc/passwd`) / private tmp `/tmp/zap-<user>` / panel data dir `data/users/<user>`**; existing paths are additionally `canonicalize`d to block symlink escapes (e.g. `/home/u/evil -> /etc`). Admin/root behavior unchanged.
  - **App deploy ownership check**: `AppDeploy` carries the requester identity; `deploy` requires a non-admin user's `owner_user` to equal the requester, preventing apps from being launched as another site's account (cross-tenant tampering / privilege escalation). Admin is exempt (`skip_owner_check`); site ownership still enforced by `zapd`.
  - Non-admin users without a bound system account are now rejected instead of running as root.

- (same series) JWT startup fails when the default key is missing (fail-closed); advanced proxy `raw` body and custom `rewrite` rules are hardened against `include` / system-path / cloud-metadata injection.

## [v1.0.10] - Release Date : 2026-9-14

* 增加i18n
* 增加zh-CN和en-US

