---
title: Local Development
---

# Local Development

## Requirements

| Item | Requirement |
| --- | --- |
| Rust | 2024 edition (the `stable` toolchain from `rustup` is fine) |
| Node.js | Needed to build the frontend (`web/`) |
| Privileges | root or sudo — `zapexec` must run as root |

## One-command startup

```bash
./rundev.sh
```

The script runs in order: build the frontend → build the Rust binaries → start `zapexec` (the root-privileged
process) → start `zapd` → bring up the frontend. The default administrator account is created when the database
is first initialized and printed in the terminal.

The panel URL is printed at the end of startup (the dev config `data/run/zap.dev.yaml` is generated on first run;
default port `2663`).

## Common options

| Option | Description |
| --- | --- |
| `--release` | Build in release mode |
| `--skip-web` | Skip the frontend build (backend only) |
| `--skip-build` | Skip the Rust build and reuse the existing binaries |
| `--skip-install` | Skip `npm install` |
| `--reset-db` | Delete the dev database before startup and build a fresh one |
| `--check` | Only run fmt + clippy checks — no build, no startup |
| `--fmt` | Only run `cargo fmt --all` and exit |
| `--clippy-fix` | Only run `cargo clippy --fix` and exit |
| `--check-code` | Run fmt fixing + clippy as part of the build flow |

See `./rundev.sh --help` for the full list.

## Code checks

Run these before submitting:

```bash
cargo fmt --all && cargo clippy --all-targets
cd web && npm run type-check
```

## Path conventions in the dev environment

| Item | Location |
| --- | --- |
| `ZAP_PATH` | The project root (the script injects `ROOT_DIR`, distinct from production `/usr/local/zap`) |
| Database | `data/zap.db` |
| Runtime (sockets / keys / config) | `data/run/` |
| Logs | `data/appstore/logs/`, `data/logs/` |

`zapd` and `zapexec` must be given **the same** `ZAP_PATH`; otherwise their data directories diverge
(typical symptom: task logs reported as an "invalid log path"). `rundev.sh` injects the same value into both —
keep that in mind if you start the processes manually.

## Contribution rules

- Any system-level change (writing files, modifying services, installing software) must go through `zapexec`'s
  whitelisted verbs — **never** add an arbitrary shell execution entry point.
- When changing the frontend, update both `zh-CN.ts` and `en-US.ts` under `web/src/i18n/locales/`
  (they are type-checked against each other, so a missing key breaks the build).
