# User Manual

> Source: `USER_MANUAL.md` (repo root).  
> Copied to `data/www/html/USER_MANUAL.md` by `build.sh`,  
> rendered to HTML by `GET /api/docs/manual`.

## Install / Upgrade

See the [Upgrade Guide](./UPGRADE.md).

## Common Entry Points

| Feature | Entry | Notes |
|---|---|---|
| Add a site | Sites → Add Site | PHP / reverse-proxy / static |
| Deploy a certificate | SSL/TLS | Issue + deploy in one click |
| Cron tasks | Docs → User Manual / Cron | cron expressions |
| File manager | Files | Remote server + desktop client |
| App Store | App Store | One-click GitHub apps |
| Download source | System Settings → Zap Settings → Download Source | Where AppStore pulls source tarballs: CN / Cloudflare / local dir |
| Manage / take over nodes | Cluster → Nodes / Monitor | Manual token or credential-based token bootstrap |

## Cluster (managed nodes and node view)

Entry points: **Cluster → Nodes** (registration) and **Cluster → Monitor** (overview). The master stores one encrypted credential per node and samples metrics into its own database every 15 seconds; a node going offline breaks its curve but never affects the master itself.

### Adding a node: two credential modes

| Mode | What to fill in | When to use |
|---|---|---|
| API token (manual) | Create a `zap_` token under "Personal Center → API Tokens" on the node, then paste it into the master | A token already exists; a regular user's token is enough for monitoring only |
| Username / password (automatic) | Node login username and password (plus a 6-digit code if two-factor auth is enabled) | A freshly installed panel with no token yet. On save, the master logs in and issues a long-lived token; the password is never stored |

- The **Test** button reads node system info in token mode, but in credential mode only verifies that the login works — **no token is issued**, so it is safe to click repeatedly.
- Tokens are stored encrypted and never shown again. If a token is deleted or expires on the node, edit the node and enter the credentials again to issue a new one.
- Permissions in the node view follow **the account owning the token on that node**: a regular user's token suffices for monitoring; to change sites / files / processes through the node view, use a token issued by an administrator on that node.

### Taking over a node

Click **Take over** in the node list or monitor dashboard: after a full page reload, the sidebar, login identity and permissions all come from that node, and all reads, writes, terminal sessions and downloads happen on the managed machine.

- Read-only browsing requires `system.monitor:view` on the master; writes, terminal and WebSocket sessions additionally require `node:edit`.
- Master-owned paths (login, node registry, API tokens, etc.) are never forwarded; use **Back to master** in the header to leave the node view at any time.
- An invalid node token only affects node-side requests: update the credentials under "Cluster → Nodes" to reconnect. It will not send the master back to its login page.

## Download source (incl. air-gapped hosts)

Installing from the App Store fetches source tarballs (nginx / php / mysql / pcre2, …) from a
mirror. **That base is configurable** at System Settings → Zap Settings → Download Source:

| Kind | Value | Use it when |
|---|---|---|
| CN mirror (default) | `https://mirrors.zap.cn/pkg` | hosts inside China |
| Cloudflare mirror | `https://mirrors.zap.sh/pkg` | overseas hosts, or when the CN mirror is unreachable |
| Local directory | `/opt/zap-pkg` (stored as `file:///opt/zap-pkg`) | **air-gapped networks**, no internet at all |

A local directory mirrors the same layout as the remote mirror:

```text
/opt/zap-pkg/nginx/nginx-1.28.0.tar.gz
/opt/zap-pkg/php/php-8.3.6.tar.gz
/opt/zap-pkg/openssl/openssl-3.5.6.tar.gz
/opt/zap-pkg/mariadb/mariadb-11.4.5-….tar.gz
```

Notes:

- The directory must **already exist** — saving is rejected otherwise, so a typo surfaces now
  rather than halfway through an install
- The change applies to the **next** install; tasks already running are unaffected
- The setting lives in `{ZAP_PATH}/data/mirror.yaml` (written by zapd, read by zapexec);
  editing it by hand works too
- Scripts from third-party repos that don't call `pkg_mirror` keep using their own hard-coded URL

Collect `data/zap.log` and the browser Network screenshots when opening an issue.