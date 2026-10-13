<div align="center">

# ZAP

**Modern Linux Server / VPS Control Panel**

A lightweight, high-performance Linux server control panel written in Rust.

[![Rust](https://img.shields.io/badge/Rust-2024%20Edition-orange?logo=rust)](https://www.rust-lang.org)
[![Vue](https://img.shields.io/badge/Vue-3.5-brightgreen?logo=vue.js)](https://vuejs.org)
[![Version](https://img.shields.io/badge/version-1.3.1-blue)](https://github.com/zapsh/zap)
[![Platform](https://img.shields.io/badge/platform-Linux%20amd64%20%7C%20arm64-lightgrey?logo=linux)](https://www.kernel.org)
[![License](https://img.shields.io/badge/license-AGPL--3.0-green)](./LICENSE)

</div>

---

## Overview

ZAP is a modern, lightweight and security-focused control panel built for Linux servers and VPS instances, designed to make day-to-day administration and security maintenance simple and efficient. Our mission is to make Linux servers easier to use, and to bring powerful yet approachable server management tools to more users.

## Supported Stacks

| Stack | Description |
| --- | --- |
| Nginx + PHP-FPM | Classic PHP runtime |
| Nginx + Python app | Reverse proxy for Python applications |
| Nginx + Node.js app | Reverse proxy for Node.js applications |
| Nginx + Go app | Reverse proxy for Go applications |
| Nginx + Rust web app | Reverse proxy for Rust web applications |
| Nginx + generic app | Custom startup commands (PHP Swoole and similar) |
| Nginx + Docker | Reverse proxy for containerized services |

## Features

- **Sites**: multi-domain / multi-IP binding, rewrite rules, multiple PHP-FPM versions, app deployment
- **SSL / TLS**: manual import, self-signed certificates, automated Let's Encrypt issuance (HTTP-01 / DNS-01, wildcards included), expiry alerts
- **WAF**: integrated ModSecurity
- **Server operations**: real-time monitoring, file manager, web SSH terminal, cron jobs, firewall (firewalld / ufw / nftables / iptables)
- **Databases**: visual MySQL / MariaDB management
- **Docker**: all-in-one management of containers, images, networks and Compose
- **Multi-tenant**: Admin / Reseller / User roles, plans and reselling, TOTP two-factor authentication, audit log
- **Upgrade & backup**: three upgrade entry points (panel / CLI / install script), backup → atomic replace → automatic rollback on failure

## Quick Start

```bash
wget -O install.sh https://mirrors.zap.cn/zap/install.sh && bash install.sh
```

When installation finishes, visit `https://<server-ip>:2663` and log in with the credentials printed at the end of the installation.

**Supported systems**

| Distribution | Versions |
| --- | --- |
| RHEL / Rocky Linux / AlmaLinux | 8+ |
| CentOS Stream | 9+ |
| Ubuntu LTS | 22.04 / 24.04 / 26.04 |
| Debian | 12+ |

For offline installation and upgrades in isolated networks, see the [Installation Guide](./docs/guide/install.md).

## Documentation

Full documentation lives in [`docs/`](./docs/) (VitePress, available in Chinese and English; preview locally with `npm run dev`):

| Document | Contents |
| --- | --- |
| [Installation](./docs/guide/install.md) | Online / offline installation and parameters |
| [Upgrade](./docs/guide/upgrade.md) | Three upgrade paths, rollback, scripting |
| [User Manual](./docs/guide/user-manual.md) | Panel features in detail |
| [FAQ](./docs/guide/faq.md) | FAQ and troubleshooting |
| [API Reference](./docs/api/) | REST endpoints (generated from the backend `api_docs.json`) |

## Contributing

Issues and pull requests are welcome. Before submitting, run `cargo fmt --all && cargo clippy --all-targets` and `cd web && npm run type-check`. System-level changes must go through `zapexec`'s whitelisted verbs — never introduce an arbitrary shell execution entry point.

For the full workflow (fork setup, branch model, local development, release packaging) see [Contributing / Development Guide](./CONTRIBUTING.md).

By submitting a PR you agree to license your contribution under [AGPL-3.0](./LICENSE), or (where applicable) under the [Commercial License](./COMMERCIAL-LICENSE.md).

## License

ZAP is **dual licensed**:

- **AGPL-3.0 (open source)**: free to use, modify and distribute. Under AGPL §13, if you offer ZAP's functionality to others over a network (e.g. as SaaS), you must also provide the complete corresponding source code.
- **Commercial License (Zap Pro)**: required for closed-source integration, modification or redistribution, or for enterprise features such as cluster management.

In short: **open source with proper disclosure → free under AGPL-3.0; closed-source integration / commercial use → purchase a commercial license.**

For commercial licensing, contact <sales@zap.cn>.
