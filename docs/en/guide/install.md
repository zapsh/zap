---
title: Installation
---

# Installation

## Requirements

| Item | Requirement |
| --- | --- |
| OS | Linux (systemd as the service manager) |
| Architecture | amd64 / arm64 (ppc64le and s390x packages also exist) |
| Privileges | root |
| Port | 2600 (panel; adjustable) |

## Online installation

The install script only depends on `bash` and `wget`. It touches the network in three places:
resolving the latest version, downloading the release package, and cloning the AppStore repository.

```bash
# Community edition; admin password is generated randomly and printed
sudo bash install.sh

# Pin a version and set the admin credentials explicitly
sudo bash install.sh latest --admin-user zapops --admin-pass 'S3cret-Pass'

# Zap Pro (commercial edition; package name carries the -pro suffix)
sudo bash install.sh latest --pro
```

When it finishes it prints the access URL `https://<server IP>:2600` together with the admin credentials.

### Common options

| Option | Description |
| --- | --- |
| `[VERSION]` | Version to install; defaults to `latest` |
| `--pro` | Install Zap Pro. The release line is decided by the binary being installed (pro / community) and subsequent upgrades follow the same line (can also be forced with `--pro` / `--community`) |
| `--admin-user <name>` | Admin username (also the Linux account name, home directory `/home/<name>`); defaults to `admin` |
| `--admin-pass <pass>` | Admin password. Allowed characters: `letters digits . _ -`. If omitted, one is generated randomly |
| `--join-url <url>` | **Zap Pro**: full base URL of the controller to join |
| `--join-token <code>` | Registration token (prefer the `ZAP_JOIN_TOKEN` environment variable so it does not linger in shell history) |
| `--pkg <path>` | Use a local release package instead of downloading |
| `--offline` | Never access the internet |
| `--lang <zh\|en>` | Language of the installer output (inferred from `$LANG`, falls back to Chinese; `ZAP_LANG` also works) |

The admin credentials can also be supplied via `ZAP_ADMIN_USER` / `ZAP_ADMIN_PASSWORD`
(command-line options take precedence).

::: tip Upgrading and rollback
Re-running the installer on a machine that already has ZAP installed is equivalent to an upgrade.
A backup is taken automatically beforehand; use `zapupgrade rollback --list` to inspect and roll back.
See [Upgrade](./upgrade).
:::

## Offline installation (intranet / no internet)

Intranet machines never reach the network. Obtain the release tarball and `install.sh` on a machine
with internet access, then copy both to the intranet host:

```bash
# on a machine with internet: download the release package (install.sh too, or grab it from the repo/mirror)
wget https://mirrors.zap.cn/zap/releases/zap-v1.5.0-linux-amd64.tar.gz
wget https://mirrors.zap.cn/zap/install.sh
# move both files to the target host via USB / an internal file share / etc.
```

On the target host:

```bash
sudo bash install.sh --pkg ./zap-v1.5.0-linux-amd64.tar.gz --offline
```

- Offline installation uses `install.sh --pkg <release package> --offline`; the package must be given
  explicitly (it is neither auto-selected nor verified).
- The AppStore uses the seed package bundled in the release; it can be retried from the panel at any time.

## Offline upgrade of an installed machine

For a machine that already has ZAP installed, copy the new release tarball over and upgrade with the
installed `zapupgrade` directly (no need to re-run the installer):

```bash
sudo zapupgrade upgrade --pkg ./zap-v1.5.0-linux-amd64.tar.gz
```

It goes through exactly the same replacement flow as an online upgrade
(backup → atomic replace → restart → automatic rollback on failure).
A package for the wrong architecture or release line is rejected before any binary is replaced.

## Installation layout

| Path | Contents |
| --- | --- |
| `/usr/local/zap` | Program and data (`zapd` / `zapctl` / `zapexec` / `zapupgrade`, `data/`) |
| `/etc/zap` | Configuration (`zap.yaml`) and certificates |
| `/etc/systemd/system/zapd.service` etc. | systemd units |

Service management:

```bash
systemctl status zapd      # panel service (zapexec is its prerequisite)
systemctl restart zapd
```

## Uninstallation

```bash
sudo bash uninstall.sh
```
