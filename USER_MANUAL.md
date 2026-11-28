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
| API token (manual) | Create a `zap_` token under "Personal Center → API Tokens" on the node, then paste it into the master | A token already exists, issued by an **administrator** on that node |
| Username / password (automatic) | Node login username and password (plus a 6-digit code if two-factor auth is enabled) | A freshly installed panel with no token yet. On save, the master logs in and issues a long-lived token; the password is never stored |

- The node token must belong to an **administrator**: roles are checked via `/user/info` when adding or testing. Network rates and the Top 10 process snapshot come from Admin endpoints on the node and a regular user's token is rejected immediately.
- The **Test** button reads node system info and checks admin permission in token mode; in credential mode it only verifies that the login works — **no token is issued**, so it is safe to click repeatedly.
- Tokens are stored encrypted and never shown again. If a token is deleted or expires on the node, edit the node and enter the credentials again to issue a new one.
- Permissions in the node view follow **the account owning the token on that node**: a regular user's token suffices for monitoring; to change sites / files / processes through the node view, use a token issued by an administrator on that node.

### Taking over a node

Click **Take over** in the node list or monitor dashboard: after a full page reload, the sidebar, login identity and permissions all come from that node, and all reads, writes, terminal sessions and downloads happen on the managed machine.

- Read-only browsing requires `system.monitor:view` on the master; writes, terminal and WebSocket sessions additionally require `node:edit`.
- Master-owned paths (login, node registry, API tokens, etc.) are never forwarded; use **Back to master** in the header to leave the node view at any time.
- An invalid node token only affects node-side requests: update the credentials under "Cluster → Nodes" to reconnect. It will not send the master back to its login page.

### Groups, filters and batch operations

- "Manage groups" creates, renames and deletes groups. Deleting a group only makes its nodes ungrouped; it never deletes nodes. Both the node list and the monitor screen can filter by group.
- The list supports search by name / URL / IP / remark, filters by status, online state and group, and sorting by name / CPU / memory / last seen.
- Select multiple nodes to enable, disable or regroup them in bulk, or dispatch an **existing App Store script** (arbitrary commands are never run). Results are reported per node.

### Credential expiry and alerts

- Optionally set a token lifetime (days) when adding a node. The list shows Valid / Unknown / Invalid / Expired; use **Rotate credential** to reissue via username and password when a token is invalid or expired.
- "Alert settings" configures CPU / memory / disk thresholds, offline delay and cooldown, with optional email to administrators. Checks run once a minute; see "Alert history" for events. Duplicate alerts for the same node and kind are suppressed during the cooldown.
- Metrics are retained for 14 days by a daily cleanup job.

### Ownership and permissions

- Every node has an owner: administrators see all nodes and can reassign them; other roles only see and operate nodes they own. The takeover proxy applies the same ownership check.
- Read-only browsing requires `system.monitor:view`; writes and terminal sessions also require `node:edit`.

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