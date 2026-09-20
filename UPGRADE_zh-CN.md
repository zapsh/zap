# 升级指南

> 文档源：`UPGRADE_zh-CN.md`（位于仓库根）。
> 由 `build.sh` 拷贝到 `data/www/html/UPGRADE_zh-CN.md`，
> 后端 `GET /api/docs/upgrade?lang=zh-CN` 渲染为 HTML 后展示（两份内容保持一致）。

## 三种升级方式

| 方式 | 适用场景 | 入口 |
| --- | --- | --- |
| 面板 | 日常升级：有进度条、历史记录、可配自动更新 | 系统设置 → About ZAP → 系统更新 → 检查更新 / 立即更新 |
| 命令行 | 面板打不开、服务起不来、批量运维 | `zapupgrade upgrade --to latest` |
| 安装脚本 | 首次部署、离线、手上已有发行包 | `bash scripts/install.sh [版本]` |

命令行与面板走的是同一套替换流程（备份 → 原子替换 → 重启 → 失败回滚），区别只在谁去下载发行包。

## 命令行升级

需要 root（要写 `/usr/local/zap` 并重启服务）：

```bash
zapupgrade upgrade                   # 升级到最新版
zapupgrade upgrade --to v1.0.12      # 升级到指定版本
zapupgrade upgrade --to 1.0.12 --force  # 已是该版本也要重装
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

注意：

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
