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

## 项目简介

**ZAP 是一套面向 Linux 服务器与 VPS 的现代化虚拟主机管理系统。**

它将网站、域名、SSL 证书、Nginx、PHP-FPM、MySQL、Docker、文件和服务器运维能力集中到一个简洁的管理面板中，帮助个人开发者、团队与服务商更高效地部署和维护网站。无论是创建 Nginx 虚拟主机、切换 PHP 版本、管理数据库与 Docker 服务，还是通过浏览器在线编辑网站文件，ZAP 都提供统一、直观的操作体验。

ZAP 的后端以 **Rust** 编写（Axum + Tokio + SQLx），前端采用 **Vue 3 + Element Plus**，并通过现代化文件管理器提供在线浏览、编辑、上传下载、新建、重命名与删除等能力。部署方便，资源占用低。

ZAP 以 **[Apache-2.0](./LICENSE)** 开源许可发布：个人与企业均可免费使用。


---

## 核心特性

### 网站与域名

- 站点全生命周期管理：创建 / 编辑 / 删除 / 停用 / 维护模式（三态切换）
- 多域名绑定、多 IP 绑定、自定义站点目录、运行目录
- Nginx 虚拟主机配置在线编辑与保存，配置语法校验
- Nginx 服务状态、启停重载、状态监控
- PHP-FPM 多版本实例管理（php74 / php81 …）与 `fpm_spec` 参数调优
- 站点目录与文件结构浏览，目录保护

### SSL / TLS

- 手动导入证书（PEM 粘贴上传，自动解析域名与有效期）
- 一键自签名证书（rcgen）
- **Let's Encrypt** 自动签发（ACME HTTP-01 / DNS-01，含泛域名与 DNS 服务商 API 自动处理）
- 证书列表、详情、续期、删除，到期预警


### 服务器运维

| 能力 | 说明 |
| --- | --- |
| 实时监控 | CPU、内存、磁盘、网络、负载、进程（sysinfo / systemstat） |
| 文件管理 | 在线浏览、编辑（CodeMirror）、上传下载、新建 / 重命名 / 删除 |
| Web 终端 | 浏览器内 SSH 终端（xterm.js + ssh2），支持密钥推送 |
| SSH 管理 | SSH 服务状态 / 重启 / 安装、SSH 密钥管理、主机连接管理 |
| 计划任务 | Cron 任务增删改查与执行记录 |
| 防火墙 | firewalld / ufw / nftables / iptables 统一抽象管理 |
| 系统配置 | 时间同步、时区、主机名、DNS Resolver、IP 池、环境变量 |
| 服务管理 | systemd 服务状态与进程管理 |
| 备份升级 | 面板备份 / 还原；面板一键升级或命令行升级（zapupgrade：下载校验 → 备份 → 原子替换 → 重启 → 可回滚） |

### 多用户与分销能力

- **用户 / 角色 / 权限**：RBAC 角色体系，菜单级权限控制
- **套餐（Package）与分销（Reseller）**：资源配额与代理账号体系
- **TOTP 两步验证**：登录二次校验（二维码绑定）, Google Authenticator / Mircrosoft Authenticator / 2FA 支持
- **站内通知**：系统公告与用户消息
- **审计日志**：所有关键操作留痕，可追溯


## Quick Start

### 支持的操作系统
  * CentOS Stream 9 / 10
  * RHEL 8 / 9 / 10
  * Rocky Linux 8 / 9 / 10
  * AlmaLinux 8 / 9 / 10
  * Ubuntu 20 LTS ( End of Life ) / 22 LTS / 24 LTS / 26 LTS
  * Debian 11 LTS (End of Life ) / 12 LTS / 13 LTS


### Install

```bash
wget -O install.sh https://mirrors.zap.cn/zap/install.sh && bash install.sh

# 指定初始管理员（用户名同时作为 Linux 账号名，家目录 /home/<name>）
bash install.sh latest --admin-user zapops --admin-pass 'S3cret-Pass'
# 不指定密码时自动生成 16 位随机密码，安装完成后打印（不会再次显示）
```

脚本幂等：检测到 `/usr/local/zap/zapd` 已存在时按**升级**处理（覆盖二进制与脚本资源，保留数据与配置）。
也支持 `bash install.sh v1.0.12` 指定版本。

> **bash**：脚本用到 bash 特性，`sh install.sh` 执行时会自动切到 bash 重入；
> 通用脚本片段走 `bash`，AppStore 包脚本与计划任务用的绝对路径由 zapexec 自行解析。

初始管理员可用 `--admin-user` / `--admin-pass` 指定，也可用环境变量 `ZAP_ADMIN_USER` / `ZAP_ADMIN_PASSWORD`（命令行优先）。安装末尾会执行 `zapd --init-admin <用户> --admin-password <密码>` 建库并写入管理员（凭据只经命令行传递，**不落任何文件**），Linux 账号与家目录 `/home/<用户名>`（www/logs/tmp 骨架）由安装脚本预建。已安装过的机器重跑安装脚本不会改动现有管理员密码。

安装完成后访问 `https://<服务器IP>:2600`，用安装结束时输出的账号登录。

后续升级推荐 `zapupgrade upgrade`（自带备份与回滚），用法见下方[系统升级](#系统升级zapupgrade)或 [UPGRADE_zh-CN.md](./UPGRADE_zh-CN.md)。

### 命令行运维（zapctl）

```bash
zapctl start | stop | restart | status      # 面板服务管理
zapctl logs                                  # 查看运行日志
zapctl backup | restore                      # 备份与还原
zapctl user add|list|passwd                  # 面板用户管理
zapctl config get|set <key>                  # 读写 zap.yaml
zapctl env list|get|set|unset|import         # 运行环境键值（data/server_env.yaml，写操作需 root）
zapctl cred gen [service] [user]             # 生成密码：无参数仅打印；带参加密存入凭据库
zapctl cred set <service> <user> [password]  # 录入服务侧既有密码（缺省经 stdin）；存在则覆盖
zapctl cred show|exists|ls|rm <service> <user>  # 凭据读取 / 判断 / 列表 / 删除
```

#### 服务凭据（密码加密存储）

应用安装产生的服务密码（如 MySQL 的 `root` / `zapadm`）统一由 `zapctl cred` 托管：

```bash
zapctl cred gen mysql root        # 生成并加密保存 → /etc/zap/credentials/mysql_root.cred
zapctl cred gen mysql zapadm
zapctl cred show mysql root       # 解密输出（root，脚本可直接取值）
zapctl cred set mysql root '<现有密码>'   # 录入服务侧已有密码（如管理员手动建库 / 改密后同步）
zapctl cred ls                    # 列出已保存凭据
```

- 凭据目录 `/etc/zap/credentials`（目录 `0700`、文件 `0400`，仅 root 可读写）；
- 内容为单行 AES-256-GCM 密文（`v1:nonce:cipher`），密钥复用面板主密钥 `/etc/zap/secret.key`；
- `zapd` / `zapexec` / `zapctl` 共用同一把密钥，可互通解密 —— 安装脚本通过 `zapctl cred show` 取回密码，`zapexec` 提供 `cred.read` 动词供面板调用；
- 卸载对应应用时（`uninstall.sh`）会一并删除其凭据；
- `zapctl backup zap` 归档内含主密钥 `secret.key` 与凭据目录，支持整机迁移；还原时以归档密钥覆盖本机密钥（还原前自动备份当前状态，可回滚）。

### 系统升级（zapupgrade）

三种入口共用同一套替换流程（备份 → 原子替换 → 先 `zapexec` 后 `zapd` 重启 → 失败自动回滚），区别只在谁去下载发行包：

| 方式 | 适用场景 | 入口 |
| --- | --- | --- |
| 面板 | 日常升级：进度、历史记录、可配自动更新 | 系统设置 → About ZAP → 系统更新 |
| 命令行 | 面板打不开 / 服务起不来 / 批量运维 | `zapupgrade upgrade --to latest` |
| 安装脚本 | 首次部署、离线、已有发行包 | `bash scripts/install.sh [版本]` |

```bash
zapupgrade upgrade                     # 升级到最新版
zapupgrade upgrade --to v1.0.12        # 升级到指定版本
zapupgrade upgrade --to 1.0.12 --force # 已是该版本也要重装

zapupgrade rollback --list             # 列出可回滚的备份
zapupgrade rollback                    # 回滚到最近一次备份
zapupgrade rollback --to 1712345678-v1.0.12   # 回滚到指定备份
```

- 需要 root（写 `/usr/local/zap` 并重启服务），`--dir` 可换安装根，`--channel` 可换更新渠道
- 升级前自动备份旧二进制到 `/usr/local/zap/data/upgrade/backup/{时间戳}-{版本}/`，回滚即从此处取回
- 升级过程中面板会短暂不可用（`zapd` 最后重启）；非 systemd 环境（docker / `rundev.sh`）需手动重启
- 完整说明见 [UPGRADE_zh-CN.md](./UPGRADE_zh-CN.md)

### Development

```bash
git clone https://github.com/zapsh/zap.git && cd zap

./rundev.sh                 # 构建前端 + 后端并启动（https://127.0.0.1:2600）
./rundev.sh --release       # release 模式
./rundev.sh --skip-web      # 跳过前端构建
./rundev.sh --reset-db      # 重建数据库（默认 admin / A123456）
./rundev.sh --admin-user zapops --admin-pass secret  # 指定初始管理员（配合 --reset-db）
./rundev.sh --check         # 只做 fmt / clippy 检查（提交前跑，默认构建不带检查）
```

> `zapexec` 需要 root 权限，脚本会通过 `sudo` 启动。软件安装根目录默认为 `/usr/local/apps`，可通过 `export ZAP_APPS_DIR=...` 覆盖。

### 发布打包

```bash
COS_ID=xxx COS_KEY=xxx ./build.sh
```

产物为 `zap-v{version}-linux-{amd64|arm64}.tar.gz`，含 sha256 校验文件并上传至镜像站。

---

## Directory Structure

```
zap-rs/
├── zapd/           # 面板主服务（Axum 路由、业务内核、调度器）
├── zapexec/        # root 特权守护进程（白名单动词执行）
├── zap-proto/      # zapd ↔ zapexec 共享协议（帧编解码 + HMAC 认证）
├── zapctl/         # 命令行运维工具
├── zapupgrade/     # 系统升级器（面板触发 / 命令行 upgrade、rollback）
├── web/            # Vue 3 前端
├── scripts/        # 安装/卸载脚本、systemd 单元、服务配置模板
├── conf/           # 开发用配置与自签证书
└── data/           # 运行时数据（SQLite、应用商店、已安装应用）
```

生产环境路径约定：

| 用途 | 路径 |
| --- | --- |
| 配置文件 | `/etc/zap/zap.yaml` |
| TLS 证书 | `/etc/zap/zap.crt`、`/etc/zap/zap.key` |
| HMAC 密钥 | `/etc/zap/exec.key`（首次启动自动生成） |
| 程序目录 | `/usr/local/zap/` |
| 运行时 | `/run/zap/`（Unix socket） |
| 软件安装 | `/usr/local/apps/` |

### 配置示例（`zap.yaml`）

```yaml
server:
  address: 0.0.0.0
  port: 2600
  cert_file: /etc/zap/zap.crt
  key_file: /etc/zap/zap.key
  url_prefix: ""          # 反代路径前缀，可选
jwt:
  jwt_secure: <随机密钥>
  jwt_expire: 3600
exec:
  socket_path: /run/zap/exec.sock
  secret_path: /etc/zap/exec.key
db:
  path: /usr/local/zap/data/zap.db
```

---

## AppStore

```
data/appstore/
├── repos.yaml     # Git 源配置列表（多源：id / 名称 / 地址 / 同步状态）
├── repos/         # 所有 Git 源（一个源一个目录，目录名 = 源 id）
│   └── zap-appstore/   # 内置官方源（可更新、不可删除）
├── cache/ tmp/    # 下载缓存与原子升级暂存
└── logs/          # run-{id}.log 运行日志
```

**包格式**（`{category}/{name}/app.yaml`）：

```yaml
name: mariadb
version: "11.4.4"
category: database        # infra | application | webapps | database | library
title: MariaDB
description: ...
deps: []
default_port: 3306
scripts:
  install: install.sh
  uninstall: uninstall.sh
  upgrade: upgrade.sh
```

安装脚本由 `zapexec` 以 root 运行，注入环境变量 `ZAP_PATH` / `ZAPCTL` / `APPS_DIR` / `PKG_PATH` / `APP_ID` / `APP_VERSION`。

包冲突优先级：**内置源 < 后添加的源 < `custom/`**。

---

## Roadmap

- [x] 站点管理、SSL 证书、应用商店
- [x] Web SSH 终端、文件管理、计划任务
- [x] 防火墙、服务配置、系统监控
- [x] 多用户 / 角色 / 套餐 / 分销体系
- [x] 在线 / 命令行升级、备份还原与回滚
- [x] TOTP 两步验证、审计日志
- [ ] **Docker 容器管理**：镜像、容器、网络、卷、Compose 编排与容器化站点托管
- [ ] 集群管理：多机统一纳管与批量运维
- [ ] 应用市场插件生态
- [x] 多语言支持

---

## 参与贡献

欢迎提交 Issue 与 Pull Request。

```bash
# 提交前请执行
cargo fmt --all && cargo clippy --all-targets
cd web && npm run type-check
```

- 遵循现有代码风格，新增接口请补充审计日志与权限校验
- 涉及系统变更的能力请通过 `zapexec` 白名单动词实现，不要在业务进程内直接提权
- 提交 PR 即表示同意你的贡献以本项目许可证（[Apache-2.0](./LICENSE)）一并授权给社区

---

## 许可证

ZAP 采用 **[Apache License 2.0](./LICENSE)**（Apache-2.0）开源许可。

- 许可正文：[LICENSE](./LICENSE)（Apache-2.0 全文）
- 官方条款：<https://www.apache.org/licenses/LICENSE-2.0>

