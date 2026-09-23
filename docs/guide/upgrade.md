---
title: 
---

# 升级指南

> 文档源：`UPGRADE_zh-CN.md`（位于仓库根）。
> 由 `build.sh` 拷贝到 `data/www/html/UPGRADE_zh-CN.md`，
> 后端 `GET /api/docs/upgrade?lang=zh-CN` 渲染为 HTML 后展示（两份内容保持一致）。

## 升级方式

| 方式 | 适用场景 | 入口 |
| --- | --- | --- |
| 面板 | 日常升级：有进度条、历史记录、可配自动更新 | 系统设置 → About ZAP → 系统更新 → 检查更新 / 立即更新 |
| 命令行 | 面板打不开、服务起不来、批量运维 | `zapupgrade upgrade --to latest` |
| 离线脚本 | 内网 / 无外网：用本地发行包安装或升级 | `bash install-offline.sh` / `bash upgrade-offline.sh` |
| 安装脚本 | 首次部署、手上已有发行包 | `bash scripts/install.sh [版本]` |

命令行与面板走的是同一套替换流程（备份 → 原子替换 → 重启 → 失败回滚），区别只在谁去下载发行包。

## 离线安装（内网 / 无外网机器）

安装脚本本身不依赖外网，联网的只有三处：查最新版本号、下载发行包、克隆 AppStore 仓库。
离线模式下这三处全部跳过，**其余（部署、建库、systemd、初始化管理员）与在线安装完全一致**。

**1. 在有外网的机器上制作离线包**

```bash
bash scripts/offline-pack.sh                        # 最新社区版 + 本机架构
bash scripts/offline-pack.sh --pro --version 1.2.3  # 指定版本的商业版
bash scripts/offline-pack.sh --arch arm64           # 给另一架构的机器准备
bash scripts/offline-pack.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz   # 手上已有发行包
```

产物 `dist/zap-offline-v<版本>[-pro]-linux-<架构>.tar.gz`，解开后是：
发行包本体 + `install.sh` / `install-offline.sh` / `upgrade-offline.sh` / `uninstall.sh`
+ `SHA256SUMS` + `<发行包>.sha256` + 说明。同一个包既能装也能升。

**2. 拷到内网机器上安装**

```bash
tar zxf zap-offline-v<版本>-linux-<架构>.tar.gz
cd zap-offline
sudo bash install-offline.sh          # --admin-pass / --join-url 等参数原样透传给 install.sh
```

`install-offline.sh` 自动挑本目录下版本号最大的发行包、按 `SHA256SUMS` 校验，
再以 `--pkg <包> --offline` 交给 `install.sh`。也可以直接：

```bash
sudo bash install.sh --pkg ./zap-v<版本>-linux-amd64.tar.gz --offline
```

要点：

- 包名带 `-pro` 就按商业版装（写入 `/etc/zap/edition`），不必再加 `--pro`
- AppStore 用发行包内置的种子包，不克隆远端仓库；面板里可随时重试更新
- 升级走另一个入口：`upgrade-offline.sh`，见下一节

## 离线升级（内网 / 无外网机器）

已装过 ZAP 的机器用**同一个**离线包升级，不必重跑安装：

```bash
tar zxf zap-offline-v<版本>-linux-<架构>.tar.gz
cd zap-offline
sudo bash upgrade-offline.sh                      # 自动挑本目录里版本最大的发行包
sudo bash upgrade-offline.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz
sudo bash upgrade-offline.sh --force              # 版本不高于当前也要升（重装 / 回退）
```

它挑包与校验的规则和 `install-offline.sh` 一样，然后把包交给
`zapupgrade upgrade --pkg <包>` —— **与在线升级同一套替换流程**
（备份 → 原子替换 → 重启 → 失败自动回滚 → 可 `zapupgrade rollback`）。
离线只是把「下载」换成「读本地文件」，升级质量与在线一致。

也可以直接调升级器：

```bash
sudo zapupgrade upgrade --pkg ./zap-v1.2.3-linux-amd64.tar.gz --dir /usr/local/zap
```

离线升级会在**替换二进制之前**拦住这几类包（离线拷错包的概率比在线下载高得多）：

| 拦什么 | 为什么 | 怎么办 |
| --- | --- | --- |
| 文件名解析不出版本 | 离线时文件名是唯一的元信息来源 | 按 `zap-v<版本>[-pro]-linux-<架构>.tar.gz` 命名，或用 `--to` 指定版本 |
| 包是别的架构 | 拷错架构会直接把机器搞挂 | 拷对应架构的发布包 |
| 包内没有二进制 | 常是拿外层 `zap-offline-*.tar.gz` 当发布包了 | 用解开后里面那个 `zap-v*.tar.gz` |
| sha256 不匹配 | 传递中损坏或拿错版本 | 重拷；确认无误时用 `--no-verify` |
| 发行线与本机不同 | 免得把 Pro 覆盖成社区版（或反向） | 确认就显式加 `--pro` / `--community` |

## 命令行升级

需要 root（要写 `/usr/local/zap` 并重启服务）：

```bash
zapupgrade upgrade                   # 升级到最新版
zapupgrade upgrade --to v1.0.12      # 升级到指定版本
zapupgrade upgrade --to 1.0.12 --force  # 已是该版本也要重装
zapupgrade upgrade --pkg ./zap-v1.0.12-linux-amd64.tar.gz   # 离线：用本地包
```

流程：查渠道 `latest.txt` → 下载 `zap-v<版本>-linux-<架构>.tar.gz` → sha256 校验
（远端有同名 `.sha256` 文件时）→ 解包 → 备份当前二进制到
`data/upgrade/backup/<时间戳>-<版本>/` → 原子替换 → 依次重启 `zapexec`、`zapd`。

| 参数 | 说明 | 默认值 |
| --- | --- | --- |
| `--to` | 目标版本：`latest` / `v1.2.3` / `1.2.3` | `latest` |
| `--channel` | 更新渠道（镜像地址），与面板「系统更新」里的渠道一致 | `https://mirrors.zap.cn/zap/releases` |
| `--dir` | ZAP 安装根目录 | `/usr/local/zap` |
| `--log` | 升级日志（追加写） | `data/upgrade/logs/run-cli-<时间戳>.log` |
| `--force` | 目标版本不高于当前版本时仍然安装 | 关 |
| `--pkg` | **离线升级**：用本地发行包，不联网（版本 / 发行线 / 架构从文件名解析） | 无 |
| `--sha256` | 本地包的校验值（缺省读 `<包>.sha256`，都没有则跳过校验） | 无 |
| `--no-verify` | 跳过本地包的 sha256 校验 | 关 |

注意：

- `--pkg` 与 `--to` 并存时：版本以 `--to` 为准（不一致会记一条日志），
  但架构与发行线始终按文件名判定 —— 离线时文件名才是可信的元信息。
- 升级过程中面板会短暂不可用（`zapd` 最后重启）。
- 非 systemd 环境（docker、`rundev.sh` 裸进程）二进制会替换成功，但不会自动重启，
  日志会提示「请手动重启 zapd 生效」。
- 命令行升级**不会**写面板用的 `__ZAP_DONE__` 标记，也就不会在「About ZAP → 系统更新」里留下运行记录。

## 回滚

每次升级前都会自动备份旧二进制，可直接回滚：

```bash
zapupgrade rollback --list                     # 列出可回滚的备份
zapupgrade rollback                            # 回滚到最近一次备份
zapupgrade rollback --to 1712345678-v1.0.12    # 回滚到指定备份
```

备份目录：`/usr/local/zap/data/upgrade/backup/<时间戳>-<版本>/`（`--dir` 可改）。
回滚同样只按备份里实际存在的二进制恢复，并依次重启对应服务。

## 脚本化（可选）

适合 cron 或批量运维，需要面板「开发 → API Token」签发的 token：

```bash
curl -k -X POST https://127.0.0.1:2600/api/system/update/apply \
  -H "Authorization: Bearer $ZAP_TOKEN"
curl -k https://127.0.0.1:2600/api/system/update/log/<run_id> \
  -H "Authorization: Bearer $ZAP_TOKEN"
```

依赖 zapd 在线；面板不可用时请用上面的命令行方式。

## 升级前

1. **备份数据库**：`cp data/zap.db data/zap.db.bak.$(date +%s)`
   （命令行/面板升级会自动备份二进制，但不会备份数据库）
2. **备份配置**：`cp -Rf /etc/zap/ /root/zap.bak.$(date +%s)`
3. **备份站点**：若使用了 `/var/www` 之类的自定义路径，一并 `tar`。

## 升级后

1. 打开面板，确认「仪表盘」显示的版本号与目标版本一致。
2. 核对 cron 任务是否仍然存在。
3. 比对 `nginx.conf` 模板与你的自定义片段。
