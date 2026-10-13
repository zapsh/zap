---
title: Contributing
---

# Contributing

For developers submitting code to [zapsh/zap](https://github.com/zapsh/zap) via a fork: repository model, branch conventions, day-to-day workflow and submission requirements.

To run the stack locally, see [Local Development](./development).

## Repository Model

| Remote | URL | Purpose |
| --- | --- | --- |
| `origin` | `git@github.com:<your-user>/zap.git` | Your fork — push here |
| `upstream` | `git@github.com:zapsh/zap.git` | Official repository — pull only |

First-time setup:

```bash
git clone git@github.com:<your-user>/zap.git
cd zap
git remote add upstream git@github.com:zapsh/zap.git
git fetch --tags upstream
```

> Tracking `upstream/main` on `main` is recommended: `git pull` then means "sync with the official repo", and you push explicitly with `git push origin main`.
> If `main` tracks `origin/main`, every sync must be explicit: `git pull --tags upstream main`. A bare `git pull` only fetches your own fork and will never pick up upstream changes.

## Branch Conventions

- **`main` is a mirror of upstream only** — never commit to it.
- One branch per change (`feature/xxx` / `fix/xxx`), cut from the latest `upstream/main`.
- Push the branch to `origin` and merge it upstream through a PR.
- Delete the branch locally and on the fork once the PR is merged.

## Day-to-Day Workflow

### 1. Sync with upstream before you start

```bash
git checkout main
git pull --tags upstream main
git push origin main          # keep the fork in sync too
```

### 2. Create a branch

```bash
git checkout -b feature/xxx
```

### 3. Develop and commit

```bash
git add -A
git commit -m "feat: xxx"
```

### 4. Push and open a PR

```bash
git push -u origin feature/xxx
```

Then open a PR on GitHub from `<you>/zap:feature/xxx` to `zapsh/zap:main`.

### 5. Upstream moved while your PR is open

```bash
git fetch upstream
git rebase upstream/main        # run this on your feature branch
git push --force-with-lease origin feature/xxx
```

Rebase (not merge) to keep the history linear.

## Checks Before Submitting

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cd web && npm run type-check
```

Or a single command: `./rundev.sh --check` (checks only, no build, no services started).

## Submission Rules

- System-level changes (writing files, managing services, installing software) must go through `zapexec`'s whitelisted verbs — **never introduce an arbitrary shell execution entry point**.
- Frontend changes must update both `zh-CN.ts` and `en-US.ts` under `web/src/i18n/locales/` (missing keys fail the build).
- User-facing documentation changes must be bilingual (`docs/` and the `*_zh-CN.md` / `.md` pairs in the repo root).

## Release Packaging

Releases are cut by maintainers:

```bash
export COS_ID=xxx COS_KEY=xxx
./build.sh
```

**The frontend must be built before `cargo build`**: `zapd` embeds `web/dist` via `rust-embed`, so building in the wrong order makes the version shown in the footer / system-update page lag one release behind. The version comes from `[workspace.package].version` in the root `Cargo.toml`.

## Cheat Sheet

| Task | Command |
| --- | --- |
| Sync with upstream | `git checkout main && git pull --tags upstream main && git push origin main` |
| Start a feature branch | `git checkout -b feature/xxx` |
| Rebase an open PR branch | `git fetch upstream && git rebase upstream/main` |
| Run the stack locally | `./rundev.sh` |
| Frontend-only work | `./rundev.sh --skip-build` + `cd web && npm run dev` |
| Rust-only work | `./rundev.sh --skip-web` |
| Database schema changed | `./rundev.sh --reset-db` |
| Pre-submit checks | `./rundev.sh --check` |
| Committed to main by mistake | `git reset --soft HEAD~1` (keep changes) / `--hard` (discard) |
| Recover a lost commit | `git reflog` → `git cherry-pick <sha>` |
| Upstream rewrote history | `git push --force-with-lease origin main` (drops old commits from the fork) |

## License

ZAP is **dual-licensed** under AGPL-3.0 and a commercial license. Submitting a PR means you agree to license your contribution under [AGPL-3.0](https://www.gnu.org/licenses/agpl-3.0.html); closed-source integration or enterprise features require a commercial license.
