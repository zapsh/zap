# Upgrade Guide

> Source: `UPGRADE.md` (repo root).
> Copied to `data/www/html/UPGRADE.md` by `build.sh`,
> rendered to HTML by `GET /api/docs/upgrade` (both copies are kept in sync).

## Ways to upgrade

| Method | When to use | Entry point |
| --- | --- | --- |
| Panel | day-to-day: progress, history, scheduled auto-update | System Settings → About ZAP → System Update → Check / Update now |
| CLI | panel is down, services won't start, fleet ops | `zapupgrade upgrade --to latest` |
| Offline | air-gapped / intranet: install or upgrade from a local tarball | install: `bash install.sh --pkg <tarball> --offline` / upgrade: `zapupgrade upgrade --pkg <tarball>` |
| Install script | first deploy, release tarball already at hand | `bash scripts/install.sh [version]` |

The CLI and the panel share the exact same replacement routine
(backup → atomic replace → restart → rollback on failure); they only differ in who downloads
the release package.

## Offline install (air-gapped / intranet hosts)

The installer itself needs no internet; only three steps do: resolving `latest`, downloading the
release tarball, and cloning the AppStore repo. Offline mode skips all three — everything else
(deploy, DB init, systemd, admin bootstrap) is identical to an online install.

**1. Obtain the release tarball and `install.sh` on a machine with internet access**

```bash
# download the release package (or grab it from the GitHub Release / mirror by hand)
wget https://mirrors.zap.cn/zap/releases/zap-v1.5.0-linux-amd64.tar.gz
wget https://mirrors.zap.cn/zap/install.sh
# move both files to the target host via USB / an internal file share / etc.
```

**2. Install on the target host from the local tarball**

```bash
sudo bash install.sh --pkg ./zap-v1.5.0-linux-amd64.tar.gz --offline
```

Notes:

- A `-pro` tarball installs as Zap Pro (the edition is baked into the binary); no need to pass `--pro`
- AppStore falls back to the seed packages shipped inside the release; retry the update from the panel

## Offline upgrade (air-gapped / intranet hosts)

A host that already has ZAP installed upgrades from a local tarball — no reinstall needed.
`zapupgrade` is already installed at `/usr/local/zap` (and on `PATH`); use its `--pkg` mode:

```bash
sudo zapupgrade upgrade --pkg ./zap-v1.5.0-linux-amd64.tar.gz --dir /usr/local/zap
```

`zapupgrade upgrade --pkg <tarball>` runs **the same replace flow as an online upgrade**
(backup → atomic replace → restart → auto-rollback on failure → `zapupgrade rollback`).
Offline only swaps "download" for "read a local file"; the upgrade is just as safe.

Offline upgrades reject these **before** any binary is replaced (copying the wrong file over
is far more likely offline than a bad download):

| Rejected | Why | What to do |
| --- | --- | --- |
| version unparseable from the filename | offline the filename is the only metadata | name it `zap-v<version>[-pro]-linux-<arch>.tar.gz`, or pass `--to` |
| wrong architecture | would brick the host | copy the tarball for this architecture |
| no binaries inside | usually an outer bundle, not the release tarball | use the `zap-v*.tar.gz` release package |
| sha256 mismatch | corrupted in transit, or wrong file | re-copy; `--no-verify` if you're sure |
| different edition than installed | avoid clobbering Pro with Community (or vice versa) | pass `--pro` / `--community` to confirm |

## CLI upgrade

Must run as root (writes `/usr/local/zap` and restarts services):

```bash
zapupgrade upgrade                     # latest version
zapupgrade upgrade --to v1.0.12        # specific version
zapupgrade upgrade --to 1.0.12 --force # reinstall even if already on that version
zapupgrade upgrade --pkg ./zap-v1.0.12-linux-amd64.tar.gz   # offline, from a local tarball
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
| `--pkg` | **offline upgrade**: use a local tarball, no network (version / edition / arch from the filename) | none |
| `--sha256` | expected hash for the local tarball (defaults to `<tarball>.sha256`; no hash → skip) | none |
| `--no-verify` | skip the sha256 check for the local tarball | off |

Notes:

- With both `--pkg` and `--to`, the version comes from `--to` (a mismatch is logged), but the
  architecture and edition are always taken from the filename — offline, that is the trustworthy part.
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
auto-update settings can be re-saved from **System Settings → About ZAP → System Update**.
