---
title: User Manual
---

# User Manual

> Source document: `USER_MANUAL_zh-CN.md` (repository root).
> `build.sh` copies it to `data/www/html/USER_MANUAL_zh-CN.md`, and the backend
> `GET /api/docs/manual?lang=zh-CN` renders it as HTML.

## Installation / Upgrade

See the [Upgrade Guide](./upgrade).

## Common entry points

| Feature | Entry point | Notes |
|---|---|---|
| Add a site | Sites → Add Site | PHP / reverse proxy / static |
| Deploy a certificate | SSL/TLS | Request and deploy in one click |
| Cron jobs | Docs → User Manual / Cron | cron expressions |
| File manager | File Manager | Remote server + desktop client |
| App Store | App Store | One-click install of apps published on GitHub |
| Download mirror | System Settings → ZAP Settings → Download Source | Where the App Store fetches source packages; switch between China / Cloudflare / a local directory |
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

## Download mirror (including offline environments)

Installing software from the App Store fetches source packages (nginx / php / mysql / pcre2, …) from a mirror.
**This address is configurable** under "System Settings → ZAP Settings → Download Source", in three forms:

| Form | Value | Use case |
|---|---|---|
| China mirror (default) | `https://mirrors.zap.cn/pkg` | Machines in mainland China |
| Cloudflare mirror | `https://mirrors.zap.sh/pkg` | Overseas machines, or when the China mirror is unreachable |
| Local directory | `/opt/zap-pkg` (normalized to `file:///opt/zap-pkg` on save) | **Offline data centers**: no internet access |

A local directory uses the same layout as the mirror — just mirror its subpaths:

```text
/opt/zap-pkg/nginx/nginx-1.28.0.tar.gz
/opt/zap-pkg/php/php-8.3.6.tar.gz
/opt/zap-pkg/openssl/openssl-3.5.6.tar.gz
/opt/zap-pkg/mariadb/mariadb-11.4.5-….tar.gz
```

Key points:

- The directory must **already exist** (otherwise saving is rejected — this avoids configuring an empty path and
  only discovering at install time that nothing can be fetched).
- Changes only affect the **next** installation; running tasks are unaffected.
- The setting lives in `{ZAP_PATH}/data/mirror.yaml` (written by zapd, read by zapexec), so editing it by hand also works.
- Scripts in third-party package repositories that do not read `pkg_mirror` keep using their own hard-coded address.

## Application deployment (Application Manager)

Entry point: **Sites → Applications**. It hosts a long-running process (from a Git repository or a directory
inside the site) as a systemd service and reverse-proxies it under the site's domain.
Supported types: **Python / Node.js / Static / Go / Rust**.

### General flow

1. Pick a code source: a Git repository (branch, subdirectory and shallow-clone depth can be specified)
   or reuse an existing directory in the site.
2. Fill in the **build command** (optional): executed first at deploy time, e.g. `npm run build`,
   `go build -o bin/app .`, `cargo build --release`.
3. Fill in the **start command / entry point**: how the process is launched at runtime.
4. The application must listen on the **`PORT` environment variable**: the panel allocates a port, injects
   `PORT=xxxx`, and Nginx reverse-proxies to `127.0.0.1:PORT`.

### Go / Rust (compiled, single version)

Go and Rust are **not version-managed**: whichever version is installed on the server is used, installed
manually by the administrator. This differs from Python / Node — the platform installs multiple versions of
those automatically via uv / fnm, whereas Go / Rust have a single global version.

- The **build stage** needs `go` / `cargo` (to run `go build` / `cargo build`), but **at runtime it is a native
  binary in the working directory**, no longer depending on the toolchain.
- The executor looks for the toolchain in this order and uses the first hit:
  `/usr/local/bin`, `/usr/bin`, `/usr/local/go/bin`, `/root/.cargo/bin`.
- **Installing into `/usr/local/bin` is recommended** (system-wide, world-readable) so that all site users can
  use it — deployment runs as the site's unix user, not root. rustup installs into `~/.cargo/bin` (current user
  only) by default, so symlink it globally for site users:

  ```bash
  # Rust: rustup installs into /root/.cargo — symlink into the global path
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ln -s /root/.cargo/bin/cargo  /usr/local/bin/cargo
  ln -s /root/.cargo/bin/rustc  /usr/local/bin/rustc
  ln -s /root/.cargo/bin/rustup /usr/local/bin/rustup

  # Go: the official package lands in /usr/local/go — symlink bin globally
  ln -s /usr/local/go/bin/go /usr/local/bin/go
  ```

- The application must read the `PORT` environment variable and bind `127.0.0.1` (same convention as Python / Node).
- Go example: build command `go build -o bin/app .`, entry point (executable path relative to the working
  directory) `bin/app`.
- Rust example: build command `cargo build --release`, entry point `target/release/app`.
- Notes: Rust release builds are slow (machine dependent); Go (CGO) / Rust linking need a C toolchain such as `cc`;
  exclude the `target/` directory when backing up the site.

### Generic deployment (custom command / uploading a jar)

The "Generic" type neither compiles nor prepares dependencies: you write the **full start command** yourself,
and the platform only hosts it as a systemd service and reverse-proxies it. Suited to uploaded `jar` packages
or any interpreter / language (as long as you place the files needed to run it in the working directory).

- Choose the "Generic" app type; the **start command is required**, e.g. `java -jar app.jar`, or any executable
  script / interpreter command.
- Choose "existing directory in the site" as the code source: upload `app.jar` / your script to that site
  directory with the **file manager** first, then point the working directory at it (instead of a Git repository).
- Build command (optional): run before deploying, e.g. `unzip app.zip`, `chmod +x bin/app`; leave empty for no build.
- It must likewise read the **`PORT` environment variable** and bind `127.0.0.1` (same convention as Python / Node),
  otherwise the reverse proxy will not reach it.
- Note: the start command runs as the **site user**, so that user needs read / execute permission on the working
  directory and the artifacts.

> Generic deployment is broader in permission scope than Python / Node (it can execute arbitrary commands) and is
> therefore **disabled by default**: enable the "Generic" type in the package to allow it, at the administrator's discretion.

### Package capability control

The master switch for application management is "Allow application management" in the package;
**the deployable types** are selected in the package (Python / Node.js / Static / Go / Rust / Generic) —
leaving it empty means no restriction (all supported types allowed).

In addition, application management has its own **dedicated permission points**
(`app:view` view / `app:deploy` deploy / `app:manage` start-stop-restart / `app:delete` delete), granted to the
built-in "regular user" role by default. Administrators can refine them per role under "System → Roles & Permissions",
for example allowing a role to edit site configuration but not deploy applications, or to view applications
without being able to delete them.

## Feedback

If you hit a problem, collect `data/zap.log` and a screenshot of the browser Network tab, and send them to the
issue tracker.
