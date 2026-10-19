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
| 端口 | 2663（面板，可自行调整） |

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

装完会打印访问地址 `https://<服务器 IP>:2663` 与管理员凭据。

### 常用参数

| 参数 | 说明 |
| --- | --- |
| `[VERSION]` | 要安装的版本号，默认 `latest` |
| `--pro` | 安装 Zap Pro；发行线由所安装的二进制决定（pro / 社区），之后升级自动跟同一条线（也可显式 `--pro` / `--community`） |
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

内网机器全程不访问网络。先在一台能联网的机器上把发行包和安装脚本取下来，再拷到内网机器：

```bash
# 在能联网的机器上：下载发行包（install.sh 也可从镜像 / 仓库手动取）
wget https://mirrors.zap.cn/zap/releases/zap-v1.5.0-linux-amd64.tar.gz
wget https://mirrors.zap.cn/zap/install.sh
# 用 U 盘 / 内网文件服务等方式把上面两个文件拷到目标机
```

目标机上：

```bash
sudo bash install.sh --pkg ./zap-v1.5.0-linux-amd64.tar.gz --offline
```

- 离线安装用 `install.sh --pkg <发行包> --offline`，需显式指定发行包（不自动挑包、不校验）
- AppStore 用发行包内置的种子包，面板里可随时重试更新

## 已安装机器的离线升级

已装过 ZAP 的机器，把新版本发行包拷到目标机后，用已安装的 `zapupgrade` 直接离线升级
（无需重跑安装脚本）：

```bash
sudo zapupgrade upgrade --pkg ./zap-v1.5.0-linux-amd64.tar.gz
```

它走的是与在线升级同一套替换流程（备份 → 原子替换 → 重启 → 失败自动回滚），
拷错架构 / 发行线的包会在替换二进制之前被拒绝。

## 安装目录

| 路径 | 内容 |
| --- | --- |
| `/usr/local/zap` | 程序与数据（`zapd` / `zapctl` / `zapexec` / `zapupgrade`、`data/`） |
| `/etc/zap` | 配置（`zap.yaml`）、证书 |
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
