---
title: 参与贡献
---

# 参与贡献

面向通过 fork 向 [zapsh/zap](https://github.com/zapsh/zap) 提交代码的开发者：仓库模型、分支约定、日常协作流程与提交门槛。

本地起服务与目录约定见 [本地开发](./development)。

## 仓库模型

| 远程 | 地址 | 用途 |
| --- | --- | --- |
| `origin` | `git@github.com:<你的用户名>/zap.git` | 你的 fork，日常推送 |
| `upstream` | `git@github.com:zapsh/zap.git` | 官方主库，只拉取 |

首次搭建：

```bash
git clone git@github.com:<你的用户名>/zap.git
cd zap
git remote add upstream git@github.com:zapsh/zap.git
git fetch --tags upstream
```

> `main` 建议跟踪 `upstream/main`，这样 `git pull` 就是同步主库，推送时显式写 `git push origin main`。
> 若跟踪的是 `origin/main`，则同步必须显式写 `git pull --tags upstream main` —— 裸 `git pull` 只会拉你自己的 fork，永远拿不到上游更新。

## 分支约定

- **`main` 只做上游镜像**：不写代码、不直接提交。
- 一个改动一个分支（`feature/xxx` / `fix/xxx`），从 `upstream/main` 的最新提交切出。
- 分支推到 `origin`，通过 PR 合入 `upstream`。
- PR 合并后删除本地与 fork 上的分支。

## 日常流程

### 1. 开工前先同步主库

```bash
git checkout main
git pull --tags upstream main
git push origin main          # 顺手让 fork 跟上
```

### 2. 开分支

```bash
git checkout -b feature/xxx
```

### 3. 开发、提交

```bash
git add -A
git commit -m "feat: xxx"
```

### 4. 推送并开 PR

```bash
git push -u origin feature/xxx
```

然后在 GitHub 上从 `<你>/zap:feature/xxx` 向 `zapsh/zap:main` 开 PR。

### 5. PR 未合并时上游又更新了

```bash
git fetch upstream
git rebase upstream/main        # 在自己的 feature 分支上执行
git push --force-with-lease origin feature/xxx
```

用 rebase 而不是 merge，保持提交历史线性。

## 提交前检查

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cd web && npm run type-check
```

或一条命令：`./rundev.sh --check`（只检查，不构建、不启动）。

## 提交规范

- 涉及系统级变更（写文件、改服务、装软件）一律通过 `zapexec` 白名单动词实现，**不要新增任意 shell 执行入口**。
- 前端改动同步更新 `web/src/i18n/locales/` 下的 `zh-CN.ts` 与 `en-US.ts`（缺 key 会编译失败）。
- 面向用户的文档改动请中英双语同步（`docs/` 与仓库根目录的 `*_zh-CN.md` / `.md`）。

## 发布打包

发布由维护者执行：

```bash
export COS_ID=xxx COS_KEY=xxx
./build.sh
```

**必须先构建前端再 `cargo build`**：`zapd` 通过 `rust-embed` 把 `web/dist` 编进二进制，顺序反了会导致页脚 / 系统更新页显示的版本落后一个发布。版本号取自根 `Cargo.toml` 的 `[workspace.package].version`。

## 速查表

| 场景 | 命令 |
| --- | --- |
| 同步主库 | `git checkout main && git pull --tags upstream main && git push origin main` |
| 开功能分支 | `git checkout -b feature/xxx` |
| 同步 PR 分支 | `git fetch upstream && git rebase upstream/main` |
| 起本地服务 | `./rundev.sh` |
| 只改前端 | `./rundev.sh --skip-build` + `cd web && npm run dev` |
| 只改 Rust | `./rundev.sh --skip-web` |
| 库结构变了 | `./rundev.sh --reset-db` |
| 提交前检查 | `./rundev.sh --check` |
| 误提交到 main | `git reset --soft HEAD~1`（改动保留）/ `--hard`（丢弃） |
| 找回丢失的提交 | `git reflog` → `git cherry-pick <sha>` |
| fork 历史被上游重写 | `git push --force-with-lease origin main`（会移出 fork 上的旧提交） |

## 许可

ZAP 为 AGPL-3.0 / 商业许可**双许可**。提交 PR 即视为同意以 [AGPL-3.0](https://www.gnu.org/licenses/agpl-3.0.html) 授权；闭源集成或使用企业版功能需购买商业许可。
