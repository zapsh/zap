# Contributing / Development Guide

Everything from setting up a fork to opening a PR: repository layout, branch model, running locally, checks and release packaging.

Online version: [Contributing](./docs/en/guide/contributing.md); running the stack: [Local Development](./docs/en/guide/development.md).

## 1. Repository Layout

Rust workspace (root `Cargo.toml`):

| Crate | Purpose |
| --- | --- |
| `zapd` | Main service: panel backend + REST API, embeds `web/dist` via `rust-embed` |
| `zapexec` | Root-privileged daemon (unix socket `data/run/sock/exec.sock`) |
| `zapctl` | CLI tool |
| `zapupgrade` | System upgrader |
| `zap-proto` / `zap-crypto` / `zap-platform` | Shared protocol / crypto / platform layer |

Frontend `web/`: Vue 3 + Vite + Element Plus + Pinia.
Scripts: `rundev.sh` (local development), `build.sh` (release packaging), `scripts/` (install / uninstall / systemd / offline-pack).

## 2. Repository Model

| Remote | URL | Purpose |
| --- | --- | --- |
| `origin` | `git@github.com:<your-user>/zap.git` | Your fork — push here |
| `upstream` | `git@github.com:zapsh/zap.git` | Official repository — pull only |

```bash
git clone git@github.com:<your-user>/zap.git
cd zap
git remote add upstream git@github.com:zapsh/zap.git
git fetch --tags upstream
```

> Tracking `upstream/main` on `main` is recommended: `git pull` then means "sync with the official repo", and you push explicitly with `git push origin main`.
> If `main` tracks `origin/main`, syncing must be explicit: `git pull --tags upstream main`. A bare `git pull` only fetches your own fork.

## 3. Branch Model

- **`main` is a mirror of upstream only** — never commit to it.
- One branch per change (`feature/xxx` / `fix/xxx`), cut from the latest `upstream/main`.
- Push the branch to `origin`, merge upstream through a PR, delete the branch afterwards.

## 4. Day-to-Day Workflow

```bash
# 1. Sync with upstream before you start
git checkout main
git pull --tags upstream main
git push origin main

# 2. Create a branch
git checkout -b feature/xxx

# 3. Develop and commit
git add -A && git commit -m "feat: xxx"

# 4. Push and open a PR (bufw/zap -> zapsh/zap)
git push -u origin feature/xxx

# 5. Upstream moved while the PR is open: rebase, don't merge
git fetch upstream
git rebase upstream/main
git push --force-with-lease origin feature/xxx
```

Committed to `main` by mistake? `git reset --soft HEAD~1` keeps the changes (then `git stash` and switch branches), `--hard` discards them. Nothing is really lost — `git reflog` will find it.

## 5. Running Locally

```bash
./rundev.sh                # Build frontend + backend, start zapexec (root) + zapd
./rundev.sh --skip-web     # Skip the frontend build (Rust-only work)
./rundev.sh --skip-build   # Skip cargo build (frontend-only work)
./rundev.sh --release      # Release build
./rundev.sh --reset-db     # Delete and recreate the dev database
./rundev.sh --check        # fmt + clippy only, no build, no services
./rundev.sh --fmt          # Auto-format and exit
./rundev.sh --clippy-fix   # Auto-fix clippy warnings and exit
./rundev.sh --check-code   # Run fmt + clippy inside the build flow
./rundev.sh --admin-user zapops --admin-pass secret
```

- Panel URL `https://127.0.0.1:2663` (self-signed cert); default admin `admin / 123456`, **only applied when the database is created for the first time**.
- The dev config is generated at `data/run/zap.dev.yaml` on first run.
- `zapexec` requires root and is started via `sudo`; `Ctrl+C` stops everything.
- `ZAP_PATH` must point at the repo root (already injected by the script), otherwise `zapd` and `zapexec` resolve different data directories — the classic symptom is plugin log streams failing with "invalid log path".

Frontend-only development:

```bash
cd web
npm install          # first time
npm run dev          # vite with HMR
npm run type-check   # vue-tsc
npm run format       # prettier
npm run build:prod   # production build -> web/dist
```

## 6. Checks Before Submitting

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cd web && npm run type-check
```

Or simply `./rundev.sh --check`.

## 7. Submission Rules

- System-level changes must go through `zapexec`'s whitelisted verbs — **never introduce an arbitrary shell execution entry point**.
- Frontend changes must update both `zh-CN.ts` and `en-US.ts` under `web/src/i18n/locales/` (missing keys fail the build).
- Documentation changes must be bilingual (`docs/` and the `*_zh-CN.md` / `.md` pairs in the repo root).

## 8. Release Packaging

```bash
export COS_ID=xxx COS_KEY=xxx
./build.sh
```

**The frontend must be built before `cargo build`** (`rust-embed` bakes `web/dist` into the binary), otherwise the version shown in the footer / system-update page lags one release behind. The version comes from `[workspace.package].version` in the root `Cargo.toml`; artifacts land in `dist/zap-v<version>-linux-<arch>.tar.gz`.

## 9. Cheat Sheet

| Task | Command |
| --- | --- |
| Sync with upstream | `git checkout main && git pull --tags upstream main && git push origin main` |
| Start a feature branch | `git checkout -b feature/xxx` |
| Rebase an open PR branch | `git fetch upstream && git rebase upstream/main` |
| Run the stack | `./rundev.sh` |
| Frontend-only work | `./rundev.sh --skip-build` + `cd web && npm run dev` |
| Rust-only work | `./rundev.sh --skip-web` |
| Broken database | `./rundev.sh --reset-db` |
| Pre-submit checks | `./rundev.sh --check` |
| Undo a bad commit | `git reset --soft HEAD~1` |
| Recover a lost commit | `git reflog` → `git cherry-pick <sha>` |
| Upstream rewrote history | `git push --force-with-lease origin main` |

## 10. License

ZAP is **dual-licensed** under AGPL-3.0 and a commercial license. Submitting a PR means you agree to license your contribution under [AGPL-3.0](./LICENSE); closed-source integration or enterprise features require a [commercial license](./COMMERCIAL-LICENSE.md).
