---
title: Upgrade
---

# Upgrade Guide

> Source document: `UPGRADE_zh-CN.md` (repository root).
> `build.sh` copies it to `data/www/html/UPGRADE_zh-CN.md`, and the backend
> `GET /api/docs/upgrade?lang=zh-CN` renders it as HTML (both copies stay in sync).

## Upgrade methods

| Method | Use case | Entry point |
| --- | --- | --- |
| Panel | Day-to-day upgrades: progress bar, history, configurable automatic updates | System settings → About ZAP → System Update → Check for updates / Update now |
| CLI | Panel unreachable, service won't start, batch operations | `zapupgrade upgrade --to latest` |
| Offline | Intranet / no internet: install or upgrade from a local release package | install: `bash install.sh --pkg <package> --offline` / upgrade: `zapupgrade upgrade --pkg <package>` |
| Install script | First deployment, or you already have a release package | `bash scripts/install.sh [version]` |

The CLI and the panel share the same replacement flow (backup → atomic replace → restart → rollback on failure).
The only difference is who downloads the release package.

## Offline installation (intranet / machines without internet)

The installer itself does not depend on the internet; only three things do: resolving the latest version,
downloading the release package, and cloning the AppStore repository. Offline mode skips all three —
**everything else (deployment, database creation, systemd, admin initialization) is identical to an online install**.

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

Key points:

- A `-pro` package name means install as the commercial edition (the release line is carried by the binary),
  so `--pro` is not needed.
- The AppStore uses the seed package bundled in the release instead of cloning a remote repository;
  it can be retried from the panel at any time.

## Offline upgrade (intranet / machines without internet)

A machine that already has ZAP installed upgrades from a local tarball — no need to re-run the installer.
`zapupgrade` is already installed at `/usr/local/zap` (and on `PATH`); use its `--pkg` mode:

```bash
sudo zapupgrade upgrade --pkg ./zap-v1.5.0-linux-amd64.tar.gz --dir /usr/local/zap
```

`zapupgrade upgrade --pkg <package>` runs **the same replacement flow as an online upgrade**
(backup → atomic replace → restart → automatic rollback → `zapupgrade rollback` available).
Going offline only swaps "download" for "read a local file"; upgrade quality is identical to online.

Offline upgrades reject these kinds of packages **before replacing any binary**
(copying the wrong package offline is far more likely than downloading the wrong one):

| Rejected | Why | How to fix |
| --- | --- | --- |
| Version cannot be parsed from the filename | Offline, the filename is the only source of metadata | Name it `zap-v<version>[-pro]-linux-<arch>.tar.gz`, or specify the version with `--to` |
| Package is for another architecture | Loading a wrong-architecture binary breaks the machine outright | Copy the release package for the matching architecture |
| Package contains no binary | Usually you passed the outer bundle, not the release tarball | Use the `zap-v*.tar.gz` release package |
| sha256 mismatch | Corrupted in transit, or wrong version | Re-copy; use `--no-verify` when you are certain |
| Release line differs from this machine | Prevents overwriting Pro with community (or vice versa) | Confirm and pass `--pro` / `--community` explicitly |

## CLI upgrade

Requires root (it writes to `/usr/local/zap` and restarts services):

```bash
zapupgrade upgrade                   # upgrade to the latest version
zapupgrade upgrade --to v1.0.12      # upgrade to a specific version
zapupgrade upgrade --to 1.0.12 --force  # reinstall even if already on that version
zapupgrade upgrade --pkg ./zap-v1.0.12-linux-amd64.tar.gz   # offline: use a local package
```

Flow: read the channel's `latest.txt` → download `zap-v<version>-linux-<arch>.tar.gz` → verify sha256
(when a `.sha256` file of the same name exists remotely) → extract → back up the current binaries to
`data/upgrade/backup/<timestamp>-<version>/` → atomic replace → restart `zapexec` then `zapd`.

| Option | Description | Default |
| --- | --- | --- |
| `--to` | Target version: `latest` / `v1.2.3` / `1.2.3` | `latest` |
| `--channel` | Update channel (mirror URL), same as in the panel's "System Update" | `https://mirrors.zap.cn/zap/releases` |
| `--dir` | ZAP installation root | `/usr/local/zap` |
| `--log` | Upgrade log (appended) | `data/upgrade/logs/run-cli-<timestamp>.log` |
| `--force` | Install even when the target version is not newer than the current one | off |
| `--pkg` | **Offline upgrade**: use a local release package, no network (version / release line / architecture parsed from the filename) | none |
| `--sha256` | Checksum for the local package (defaults to reading `<package>.sha256`; verification is skipped if neither exists) | none |
| `--no-verify` | Skip sha256 verification of the local package | off |

Notes:

- When `--pkg` and `--to` are both given, `--to` wins for the version (a mismatch is logged),
  but architecture and release line are always derived from the filename — offline, the filename
  is the only trustworthy metadata.
- The panel is briefly unavailable during the upgrade (`zapd` restarts last).
- On non-systemd environments (Docker, bare processes started by `rundev.sh`) the binaries are replaced
  successfully but not restarted automatically; the log says "please restart zapd manually".
- A CLI upgrade does **not** write the `__ZAP_DONE__` marker used by the panel, so it leaves no run record
  under "About ZAP → System Update".

## Rollback

Every upgrade backs up the old binaries automatically, so you can roll back directly:

```bash
zapupgrade rollback --list                     # list available backups
zapupgrade rollback                            # roll back to the most recent backup
zapupgrade rollback --to 1712345678-v1.0.12    # roll back to a specific backup
```

Backup directory: `/usr/local/zap/data/upgrade/backup/<timestamp>-<version>/` (changeable with `--dir`).
Rollback likewise only restores binaries that actually exist in the backup, then restarts the corresponding services.

## Scripting (optional)

Suited to cron or batch operations; requires a token issued from "Development → API Token" in the panel:

```bash
curl -k -X POST https://127.0.0.1:2600/api/system/update/apply \
  -H "Authorization: Bearer $ZAP_TOKEN"
curl -k https://127.0.0.1:2600/api/system/update/log/<run_id> \
  -H "Authorization: Bearer $ZAP_TOKEN"
```

Depends on zapd being online; if the panel is unavailable, use the CLI method above.

## Before upgrading

1. **Back up the database**: `cp data/zap.db data/zap.db.bak.$(date +%s)`
   (CLI/panel upgrades back up the binaries automatically, but not the database).
2. **Back up the configuration**: `cp -Rf /etc/zap/ /root/zap.bak.$(date +%s)`
3. **Back up your sites**: if you use custom paths such as `/var/www`, `tar` them as well.

## After upgrading

1. Open the panel and confirm the version shown on the dashboard matches the target version.
2. Check that your cron jobs are still there.
3. Compare the `nginx.conf` template against your custom snippets.
