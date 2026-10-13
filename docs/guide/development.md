---
title: 本地开发
---

# 本地开发

## 环境要求

| 项 | 要求 |
| --- | --- |
| Rust | 2024 edition（`rustup` 安装的 stable 即可） |
| Node.js | 用于构建前端（`web/`） |
| 权限 | root 或 sudo —— `zapexec` 必须以 root 运行 |

## 一键启动

```bash
./rundev.sh
```

脚本会依次：构建前端 → 构建 Rust 二进制 → 启动 `zapexec`（root 特权进程）→ 启动 `zapd` → 拉起前端。默认管理员账号在首次建库时初始化并打印在终端。

面板地址见启动末尾的输出（开发配置 `data/run/zap.dev.yaml` 首次运行时生成，默认端口 `2663`）。

## 常用参数

| 参数 | 说明 |
| --- | --- |
| `--release` | 以 release 构建 |
| `--skip-web` | 跳过前端构建（只跑后端） |
| `--skip-build` | 跳过 Rust 构建，直接复用已有二进制 |
| `--skip-install` | 跳过 `npm install` |
| `--reset-db` | 启动前删除开发数据库，重建全新库 |
| `--check` | 只跑 fmt + clippy 检查，不构建、不启动 |
| `--fmt` | 只跑 `cargo fmt --all` 并退出 |
| `--clippy-fix` | 只跑 `cargo clippy --fix` 并退出 |
| `--check-code` | 构建流程内执行 fmt 修复 + clippy |

完整列表见 `./rundev.sh --help`。

## 代码检查

提交前请跑通：

```bash
cargo fmt --all && cargo clippy --all-targets
cd web && npm run type-check
```

## 开发环境的路径约定

| 项 | 位置 |
| --- | --- |
| `ZAP_PATH` | 项目根目录（脚本用 `ROOT_DIR` 注入，与生产 `/usr/local/zap` 区分） |
| 数据库 | `data/zap.db` |
| 运行时（socket / 密钥 / 配置） | `data/run/` |
| 日志 | `data/appstore/logs/`、`data/logs/` |

`zapd` 与 `zapexec` 必须拿到**同一个** `ZAP_PATH`，否则两者的数据目录会错位（典型表现：任务日志被判为「非法日志路径」）。`rundev.sh` 已给两者注入同一个值，自己手动起进程时要留意。

## 提交规范

- 涉及系统级变更（写文件、改服务、装软件）一律通过 `zapexec` 白名单动词实现，**不要新增任意 shell 执行入口**。
- 前端改动同步更新 `web/src/i18n/locales/` 下的 `zh-CN.ts` 与 `en-US.ts`（两者有类型约束，缺 key 会编译失败）。
