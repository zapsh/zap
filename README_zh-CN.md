<div align="center">

# ZAP

**现代化 Linux 服务器 / VPS 控制面板**

使用 Rust 编写的轻量、高性能 Linux 服务器控制面板。

[![Rust](https://img.shields.io/badge/Rust-2024%20Edition-orange?logo=rust)](https://www.rust-lang.org)
[![Vue](https://img.shields.io/badge/Vue-3.5-brightgreen?logo=vue.js)](https://vuejs.org)
[![Version](https://img.shields.io/badge/version-1.3.11-blue)](https://github.com/zapsh/zap)
[![Platform](https://img.shields.io/badge/platform-Linux%20amd64%20%7C%20arm64-lightgrey?logo=linux)](https://www.kernel.org)
[![License](https://img.shields.io/badge/license-AGPL--3.0-green)](./LICENSE)

</div>

---

## 简介

ZAP 是一款面向 Linux 服务器与 VPS 打造的现代化、轻量级、高安全性控制面板，致力于让日常管理与安全维护更简单高效。我们的使命是让 Linux 服务器使用更简单，让强大易用的服务器管理工具服务更多用户。

## 支持的 Stack

| Stack | 说明 |
| --- | --- |
| Nginx + PHP-FPM | 经典 PHP 运行环境 |
| Nginx + Python 应用 | 反向代理 Python 应用 |
| Nginx + Node.js 应用 | 反向代理 Node.js 应用 |
| Nginx + Golang 应用 | 反向代理 Golang 应用 |
| Nginx + Rust Web 应用 | 反向代理 Rust Web 应用 |
| Nginx + 通用应用 | 支持 PHP Swoole 等自定义启动方式 |
| Nginx + Docker | 反向代理 Docker 容器服务 |

## 核心特性

- **站点**：多域名 / 多 IP 绑定、Rewrite、PHP-FPM 多版本、App Deploy
- **SSL / TLS**：手动导入、自签证书、Let's Encrypt 自动签发（HTTP-01 / DNS-01，含泛域名）、到期预警
- **WAF**：集成Mod_Security
- **服务器运维**：实时监控、文件管理、Web SSH 终端、计划任务、防火墙（firewalld / ufw / nftables / iptables）
- **数据库与容器**：MySQL / MariaDB 可视化管理
- **Docker**: Docker 容器、镜像、网络与 Compose 一站式管理
- **多用户体系**：Admin/Reseller/User、套餐与分销、TOTP 两步验证、审计日志
- **升级与备份**：面板 / 命令行 / 安装脚本三种升级入口，备份 → 原子替换 → 失败自动回滚
- **云存储**：集成 OpenDAL，并支持与 S3 兼容的云存储。

## 快速开始

```bash
wget -O install.sh https://mirrors.zap.cn/zap/install.sh && bash install.sh
```

安装完成后访问 `https://<服务器IP>:2663`，使用安装结束时输出的账号密码登录。

**支持的系统**

| 发行版 | 版本 |
| --- | --- |
| RHEL / Rocky Linux / AlmaLinux | 8+ |
| CentOS Stream | 9+ |
| Ubuntu LTS | 22.04 / 24.04 / 26.04 |
| Debian | 12+ |

内网离线安装与离线升级见 [安装指南](./docs/guide/install.md)。

## 文档

完整文档位于 [`docs/`](./docs/)（VitePress，中英双语，`npm run dev` 本地预览）：

| 文档 | 内容 |
| --- | --- |
| [安装](./docs/guide/install.md) | 在线 / 离线安装、参数说明 |
| [升级](./docs/guide/upgrade.md) | 三种升级方式、回滚、脚本化 |
| [用户手册](./docs/guide/user-manual.md) | 面板功能详解 |
| [常见问题](./docs/guide/faq.md) | FAQ 与排障 |
| [API 参考](./docs/api/) | REST 接口（由后端 `api_docs.json` 自动生成） |

## 参与贡献

欢迎提交 Issue 与 PR。提交前请运行 `cargo fmt --all && cargo clippy --all-targets` 与 `cd web && npm run type-check`；涉及系统级变更请通过 `zapexec` 白名单动词实现，不要新增任意 shell 执行入口。

完整流程（fork 搭建、分支模型、本地运行、发布打包）见 [参与贡献 / 开发文档](./CONTRIBUTING_zh-CN.md)。

提交 PR 即视为同意以 [AGPL-3.0](./LICENSE) 授权，或（如适用）以 [商业许可](./COMMERCIAL-LICENSE.md) 授权。

## 许可证

ZAP 采用**双重许可（Dual Licensing）**：

- **AGPL-3.0（开源版）**：可免费使用、修改与分发。受 AGPL 第 13 条约束，若以 SaaS / 网络服务形式向他人提供 ZAP 功能，必须一并提供完整源代码。
- **商业许可（Zap Pro）**：闭源集成、修改或再分发，或需要使用集群管理等企业版功能，须购买 [商业许可](./COMMERCIAL-LICENSE.md)。

简而言之：**依规开源 → 免费使用 AGPL-3.0；闭源集成 / 企业商用 → 购买商业许可。**

商业授权请联系 <sales@zap.cn>。
