# Upgrade Guide

> Source: `UPGRADE.md` (repo root).
> Copied to `data/www/html/UPGRADE.md` by `build.sh`,
> rendered to HTML by `GET /api/docs/upgrade` (both copies are kept in sync).

## Three ways to upgrade

| Method | When to use | Entry point |
| --- | --- | --- |
| Panel | day-to-day: progress, history, scheduled auto-update | System Settings → System Update → Check / Update now |
| CLI | panel is down, services won't start, fleet ops | `zapupgrade upgrade --to latest` |
| Install script | first deploy, offline, release tarball already at hand | `bash scripts/install.sh [version]` |

The CLI and the panel share the exact same replacement routine
(backup → atomic replace → restart → rollback on failure); they only differ in who downloads
the release package.

## CLI upgrade

Must run as root (writes `/usr/local/zap` and restarts services):

```bash
zapupgrade upgrade                     # latest version
zapupgrade upgrade --to v1.0.12        # specific version
zapupgrade upgrade --to 1.0.12 --force # reinstall even if already on that version
```

Flow: read `latest.txt` from the channel → download
`zap-v<version>-linux-<arch>.tar.gz` → verify sha256 (when a sibling `.sha256` exists
upstream) → unpack → back up the current binaries to
`data/upgrade/backup/<timestamp>-<version>/` → atomic replace → restart `zapexec`, then `zapd`.

| Flag | Meaning | Default |
| --- | --- | --- |
| `--to` | target version: `latest` / `v1.2.3` / `1.2.3` | `latest` |
| `--channel` | update channel (mirror URL), same as the panel's channel | `https://mirrors.zap.cn/zap/releases` |
| `--dir` | ZAP installation root | `/usr/local/zap` |
| `--log` | upgrade log (append) | `data/upgrade/logs/run-cli-<timestamp>.log` |
| `--force` | install even when the target is not newer | off |

Notes:

- The panel is briefly unavailable during the upgrade (`zapd` restarts last).
- Without systemd (docker, bare `rundev.sh` processes) the binaries are replaced but not
  restarted; the log says a manual restart is required.
- CLI upgrades do **not** emit the `__ZAP_DONE__` marker, so no run is recorded in
  **System Update** history.

## Rollback

Every upgrade backs up the previous binaries first:

```bash
zapupgrade rollback --list                     # list available backups
zapupgrade rollback                            # roll back to the most recent one
zapupgrade rollback --to 1712345678-v1.0.12    # roll back to a specific backup
```

Backup dir: `/usr/local/zap/data/upgrade/backup/<timestamp>-<version>/` (override with `--dir`).
Rollback restores only the binaries actually present in the backup and restarts the matching
services.

## Scripting (optional)

Handy for cron or fleet ops; needs an API token issued in **Development → API Token**:

```bash
curl -k -X POST https://127.0.0.1:2600/api/system/update/apply \
  -H "Authorization: Bearer $ZAP_TOKEN"
curl -k https://127.0.0.1:2600/api/system/update/log/<run_id> \
  -H "Authorization: Bearer $ZAP_TOKEN"
```

Requires zapd to be reachable — use the CLI when the panel is down.

## Before Upgrading

1. **Back up the database**: `cp data/zap.db data/zap.db.bak.$(date +%s)`
   (panel/CLI upgrades back up binaries, never the database)
2. **Back up the config**: `cp -Rf /etc/zap/ /root/zap.bak.$(date +%s)`
3. **Back up sites**: `tar` custom paths such as `/var/www` as well.

## After Upgrading

1. Open the panel and confirm **Dashboard** shows the target version.
2. Verify all cron tasks are still present.
3. Diff your custom snippets against the new `nginx.conf` template.

## Settings Moved Out of the Database

Two settings are now stored as YAML files next to `zap.db` instead of SQL tables:

| File | Contents |
| --- | --- |
| `data/server_env.yaml`   | environment snapshot (`auto`) + panel defaults (`conf`) |
| `data/update_config.yaml` | auto-update switch / cron / channel / last check result |

Both files are loaded at start-up (created with defaults when missing) and written back
atomically immediately after every change; external edits are picked up by mtime without
restarting the panel.

Any leftover `server_env` / `update_config` tables are dropped on the first start after
the upgrade and their rows are **not** migrated — the environment is re-detected and the
auto-update settings can be re-saved from **System Settings → System Update**.
