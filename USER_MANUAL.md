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