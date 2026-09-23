<div align="center">

# ZAP

**现代化 Linux 服务器 / VPS 控制面板**

轻量级 · 高性能 · 特权分离 · 单文件部署

[![Rust](https://img.shields.io/badge/Rust-2024%20Edition-orange?logo=rust)](https://www.rust-lang.org)
[![Vue](https://img.shields.io/badge/Vue-3.5-brightgreen?logo=vue.js)](https://vuejs.org)
[![Version](https://img.shields.io/badge/version-1.0.3-blue)](https://github.com/zapsh/zap)
[![Platform](https://img.shields.io/badge/platform-Linux%20amd64%20%7C%20arm64-lightgrey?logo=linux)](https://www.kernel.org)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](./LICENSE)

</div>

---

## 简介

ZAP 将**网站、SSL 证书、数据库、Docker、文件与服务器运维**集中到一个简洁的 Web 面板，面向个人开发者、团队与服务商。后端 Rust（Axum + Tokio + SQLx），前端 Vue 3 + Element Plus，特权分离（`zapd` 业务 / `zapexec` root 守护），单文件部署、资源占用低。

## 核心特性

- **站点与域名**：站点全生命周期管理、多域名 / 多 IP 绑定、Nginx 配置在线编辑、PHP-FPM 多版本
- **SSL / TLS**：手动导入、一键自签、Let's Encrypt 自动签发（HTTP-01 / DNS-01 含泛域名）、到期预警
- **服务器运维**：实时监控、文件管理、Web SSH 终端、计划任务、防火墙统一抽象（firewalld / ufw / nftables / iptables）
- **数据库与 Docker**：MySQL / MariaDB 可视化管理；Docker 容器管理（Roadmap）
- **多用户体系**：RBAC 角色、套餐与分销、TOTP 两步验证、审计日志
- **升级与备份**：面板 / 命令行 / 安装脚本三种升级入口，备份 → 原子替换 → 自动回滚

## 快速开始

支持 CentOS Stream 9+、RHEL / Rocky / AlmaLinux 8+、Ubuntu 22/24 LTS、Debian 12+。

```bash
wget -O install.sh https://mirrors.zap.cn/zap/install.sh && bash install.sh
```

装完访问 `https://<服务器IP>:2600`，用安装结束时输出的账号密码登录。
内网离线安装与升级见 [安装指南](./docs/guide/install.md)。

## 文档

完整文档在 [`docs/`](./docs/)（VuePress 2，`npm run dev` 本地预览）：

| 文档 | 内容 |
| --- | --- |
| [安装](./docs/guide/install.md) | 在线 / 离线安装、参数说明、离线升级 |
| [升级](./docs/guide/upgrade.md) | 三种升级方式、回滚、脚本化 |
| [用户手册](./docs/guide/user-manual.md) | 面板功能详解 |
| [常见问题](./docs/guide/faq.md) | FAQ 与排障 |
| [API 参考](./docs/api/) | 116+ 个 REST 接口（由后端 `api_docs.json` 自动生成） |

命令行运维速查：

```bash
zapctl start|stop|restart|status   # 服务管理
zapctl logs                        # 运行日志
zapctl backup|restore              # 备份还原
zapupgrade upgrade                 # 升级到最新版（自动备份，可回滚）
zapctl cred gen|show|set|ls        # 服务密码托管（AES-256-GCM 加密）
```

## 目录结构

```
zap/
├── zapd/           # 面板主服务（Axum 路由、业务内核、调度器）
├── zapexec/        # root 特权守护进程（白名单动词执行）
├── zap-proto/      # zapd ↔ zapexec 共享协议（帧编解码 + HMAC 认证）
├── zapctl/         # 命令行运维工具
├── zapupgrade/     # 系统升级器（upgrade / rollback）
├── web/            # Vue 3 前端
├── docs/           # 文档站（VuePress 2：使用指南 + API 参考）
└── scripts/        # 安装/卸载脚本、离线打包、systemd 单元
```

## 开发

```bash
git clone https://github.com/zapsh/zap.git && cd zap
./rundev.sh                 # 构建并启动（https://127.0.0.1:2600）
./rundev.sh --check         # fmt / clippy 检查（提交前跑）
```

发布打包：`COS_ID=xxx COS_KEY=xxx ./build.sh`。

## Roadmap

- [x] 站点、SSL、应用商店、Web 终端、文件管理、计划任务、防火墙
- [x] 多用户 / 角色 / 套餐 / 分销、TOTP、审计日志
- [x] 集群管理（Zap Pro）：多机纳管、一键 SSH、主控主动拉取
- [ ] Docker 容器管理：镜像、容器、网络、Compose 编排
- [ ] 应用市场插件生态

## 参与贡献

欢迎提交 Issue 与 PR。提交前请跑 `cargo fmt --all && cargo clippy --all-targets` 与 `cd web && npm run type-check`；系统级变更请通过 `zapexec` 白名单动词实现。提交 PR 即视为同意以 [Apache-2.0](./LICENSE) 授权。
