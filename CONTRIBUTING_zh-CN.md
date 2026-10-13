# 参与贡献 / 开发文档

本文覆盖从搭建 fork 到提交 PR 的完整流程：仓库结构、分支模型、本地运行、代码检查、发布打包。

在线版见 [参与贡献](./docs/guide/contributing.md)，本地起服务细节见 [本地开发](./docs/guide/development.md)。

## 1. 仓库结构

Rust workspace（根 `Cargo.toml`）：

| Crate | 作用 |
| --- | --- |
| `zapd` | 主服务：面板后端 + REST API，通过 `rust-embed` 内嵌 `web/dist` |
| `zapexec` | root 特权守护进程（unix socket `data/run/sock/exec.sock`） |
| `zapctl` | 命令行工具 |
| `zapupgrade` | 系统升级器 |
| `zap-proto` / `zap-crypto` / `zap-platform` | 公共协议 / 加密 / 平台抽象 |

前端 `web/`：Vue 3 + Vite + Element Plus + Pinia。
脚本：`rundev.sh`（本地开发）、`build.sh`（发布打包）、`scripts/`（install / uninstall / systemd / offline-pack）。

## 2. 仓库模型

| 远程 | 地址 | 用途 |
| --- | --- | --- |
| `origin` | `git@github.com:<你的用户名>/zap.git` | 你的 fork，日常推送 |
| `upstream` | `git@github.com:zapsh/zap.git` | 官方主库，只拉取 |

```bash
git clone git@github.com:<你的用户名>/zap.git
cd zap
git remote add upstream git@github.com:zapsh/zap.git
git fetch --tags upstream
```

> `main` 建议跟踪 `upstream/main`，这样 `git pull` 就是同步主库，推送时显式写 `git push origin main`。
> 若跟踪的是 `origin/main`，同步必须显式写 `git pull --tags upstream main` —— 裸 `git pull` 只会拉自己的 fork。

## 3. 分支模型

- **`main` 只做上游镜像**：不写代码、不直接提交。
- 一个改动一个分支（`feature/xxx` / `fix/xxx`），从最新的 `upstream/main` 切出。
- 分支推到 `origin`，通过 PR 合入 `upstream`，合并后删除。

## 4. 日常开发流程

```bash
# ① 开工前同步上游
git checkout main
git pull --tags upstream main
git push origin main

# ② 开分支
git checkout -b feature/xxx

# ③ 开发提交
git add -A && git commit -m "feat: xxx"

# ④ 推送并开 PR（bufw/zap → zapsh/zap）
git push -u origin feature/xxx

# ⑤ PR 未合、上游又更新了：rebase 而不是 merge
git fetch upstream
git rebase upstream/main
git push --force-with-lease origin feature/xxx
```

误提交到 `main`：`git reset --soft HEAD~1` 保留改动（再 `git stash` + 切分支），`--hard` 丢弃。提交不会真丢，`git reflog` 里能捞回来。

## 5. 本地运行

```bash
./rundev.sh                # 构建前端 + 后端，启动 zapexec(root) + zapd
./rundev.sh --skip-web     # 跳过前端构建（只改 Rust）
./rundev.sh --skip-build   # 跳过 cargo build（只改前端）
./rundev.sh --release      # release 构建
./rundev.sh --reset-db     # 删库重建（改了表结构时）
./rundev.sh --check        # 只跑 fmt + clippy，不构建不启动
./rundev.sh --fmt          # 只自动格式化
./rundev.sh --clippy-fix   # 只自动修 clippy 告警
./rundev.sh --check-code   # 构建流程内跑 fmt + clippy
./rundev.sh --admin-user zapops --admin-pass secret
```

- 地址 `https://127.0.0.1:2663`（自签证书），默认管理员 `admin / 123456`，**仅在首次建库时生效**。
- 开发配置首次运行生成在 `data/run/zap.dev.yaml`。
- `zapexec` 需要 root，脚本用 `sudo` 启动；`Ctrl+C` 停止全部服务。
- `ZAP_PATH` 必须指向仓库根目录（脚本已注入），否则 `zapd` 与 `zapexec` 数据目录错位，典型症状是插件日志流报「非法日志路径」。

前端单独开发：

```bash
cd web
npm install          # 首次
npm run dev          # vite 热更新
npm run type-check   # vue-tsc 类型检查
npm run format       # prettier
npm run build:prod   # 生产构建 -> web/dist
```

## 6. 提交前必跑

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cd web && npm run type-check
```

或 `./rundev.sh --check`。

## 7. 提交规范

- 系统级变更一律通过 `zapexec` 白名单动词实现，**不要新增任意 shell 执行入口**。
- 前端改动同步更新 `web/src/i18n/locales/` 下的 `zh-CN.ts` 与 `en-US.ts`（缺 key 编译失败）。
- 文档改动中英双语同步（`docs/` 与根目录 `*_zh-CN.md` / `.md`）。

## 8. 发布打包

```bash
export COS_ID=xxx COS_KEY=xxx
./build.sh
```

**必须先构建前端再 `cargo build`**（`rust-embed` 会把 `web/dist` 编进二进制），否则页脚 / 系统更新页版本会落后一个发布。版本号取自根 `Cargo.toml` 的 `[workspace.package].version`，产物 `dist/zap-v<version>-linux-<arch>.tar.gz`。

## 9. 速查表

| 场景 | 命令 |
| --- | --- |
| 同步上游 | `git checkout main && git pull --tags upstream main && git push origin main` |
| 开功能分支 | `git checkout -b feature/xxx` |
| 同步 PR 分支 | `git fetch upstream && git rebase upstream/main` |
| 起服务 | `./rundev.sh` |
| 只改前端 | `./rundev.sh --skip-build` + `cd web && npm run dev` |
| 只改 Rust | `./rundev.sh --skip-web` |
| 库坏了 | `./rundev.sh --reset-db` |
| 提交前检查 | `./rundev.sh --check` |
| 撤销误提交 | `git reset --soft HEAD~1` |
| 找回丢失提交 | `git reflog` → `git cherry-pick <sha>` |
| fork 历史被上游重写 | `git push --force-with-lease origin main` |

## 10. 许可

ZAP 为 AGPL-3.0 / 商业许可**双许可**。提交 PR 即视为同意以 [AGPL-3.0](./LICENSE) 授权；闭源集成或企业版功能需购买 [商业许可](./COMMERCIAL-LICENSE.md)。
