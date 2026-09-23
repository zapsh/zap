---
title: 安装
---

# 安装

## 环境要求

| 项 | 要求 |
| --- | --- |
| 操作系统 | Linux（systemd 作为服务管理器） |
| 架构 | amd64 / arm64（另有 ppc64le、s390x 包） |
| 权限 | root |
| 端口 | 2600（面板，可自行调整） |

## 在线安装

安装脚本只依赖 `bash` 与 `wget`，联网的三处：查最新版本号、下载发行包、克隆 AppStore 仓库。

```bash
# 社区版，管理员密码随机生成并打印
sudo bash install.sh

# 指定版本与管理员凭据
sudo bash install.sh latest --admin-user zapops --admin-pass 'S3cret-Pass'

# 商业版 Zap Pro（包名带 -pro）
sudo bash install.sh latest --pro
```

装完会打印访问地址 `https://<服务器 IP>:2600` 与管理员凭据。

### 常用参数

| 参数 | 说明 |
| --- | --- |
| `[VERSION]` | 要安装的版本号，默认 `latest` |
| `--pro` | 安装 Zap Pro；发行线会记进 `/etc/zap/edition`，之后升级自动跟同一条线 |
| `--admin-user <name>` | 管理员用户名（也是 Linux 账号名，家目录 `/home/<name>`），默认 `admin` |
| `--admin-pass <pass>` | 管理员密码，可用字符 `字母 数字 . _ -`；不指定则随机生成 |
| `--join-url <url>` | **Zap Pro**：接入主控的完整基址 |
| `--join-token <code>` | 注册口令（建议用环境变量 `ZAP_JOIN_TOKEN` 传，避免留在 history） |
| `--pkg <路径>` | 使用本地安装包，不联网下载 |
| `--offline` | 全程不访问外网 |
| `--lang <zh\|en>` | 安装脚本输出语言（默认按 `$LANG` 推断，兜底中文；也可用环境变量 `ZAP_LANG`） |

管理员凭据也可用环境变量 `ZAP_ADMIN_USER` / `ZAP_ADMIN_PASSWORD` 传入（命令行参数优先）。

::: tip 升级与回滚
已安装的机器重新执行安装脚本即等于升级。升级前自动备份，可用
`zapupgrade rollback --list` 查看与回滚，详见 [升级](./upgrade.md)。
:::

## 离线安装（内网 / 无外网）

内网机器全程不访问网络，先在有外网的机器上制作离线包：

```bash
bash scripts/offline-pack.sh                        # 最新社区版 + 本机架构
bash scripts/offline-pack.sh --pro --version 1.2.3  # 指定版本的商业版
bash scripts/offline-pack.sh --arch arm64           # 给另一架构的机器准备
bash scripts/offline-pack.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz   # 手上已有发行包
```

产物 `dist/zap-offline-v<版本>[-pro]-linux-<架构>.tar.gz` 拷到内网机器后：

```bash
tar zxf zap-offline-v<版本>-linux-<架构>.tar.gz
cd zap-offline
sudo bash install-offline.sh
```

- `install-offline.sh` 自动挑本目录下版本号最大的发行包，也可 `--pkg` 指定
- 安装前按 `SHA256SUMS` 校验，确认无误但仍报不一致时加 `--no-verify`
- AppStore 用发行包内置的种子包，面板里可随时重试更新

## 已安装机器的离线升级

同一个离线包也能升级（不必重跑安装），用 `upgrade-offline.sh`：

```bash
sudo bash upgrade-offline.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz
```

它走的是与在线升级同一套替换流程（备份 → 原子替换 → 重启 → 失败自动回滚），
拷错架构 / 发行线的包会在替换二进制之前被拒绝。

## 安装目录

| 路径 | 内容 |
| --- | --- |
| `/usr/local/zap` | 程序与数据（`zapd` / `zapctl` / `zapexec` / `zapupgrade`、`data/`） |
| `/etc/zap` | 配置（`zap.yaml`）、证书、发行线标记 `/etc/zap/edition` |
| `/etc/systemd/system/zapd.service` 等 | systemd 单元 |

服务管理：

```bash
systemctl status zapd      # 面板服务（zapexec 是它的前置）
systemctl restart zapd
```

## 卸载

```bash
sudo bash uninstall.sh
```
