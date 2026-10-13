---
title: Project Structure
---

# Project Structure

```
zap/
├── zapd/           # Panel service: Axum routes, business core, task scheduling
├── zapexec/        # Root-privileged daemon: executes whitelisted verbs
├── zap-proto/      # Shared zapd ↔ zapexec protocol (frame codec + HMAC auth)
├── zap-crypto/     # Unified crypto library (AES-256-GCM) and service credential storage
├── zapctl/         # Command-line operations tool
├── zapupgrade/     # System upgrader (upgrade / rollback)
├── zap-platform/   # Cross-UNIX platform abstraction layer (in progress, not wired in yet)
├── zappro/         # Commercial module (empty placeholder in open-source builds)
├── web/            # Vue 3 + Element Plus frontend
├── docs/           # Documentation site (VitePress)
└── scripts/        # Install / uninstall, offline packaging, systemd units
```

## Process model

| Process | Identity | Responsibility |
| --- | --- | --- |
| `zapd` | Unprivileged user (panel account) | HTTP API, business logic, database, task scheduling |
| `zapexec` | root | Executes privileged operations via whitelisted verbs |

They talk over a Unix Socket, and every request carries an HMAC signature. **The `Request` enum in `zap-proto` is
the hard boundary**: only verbs in that enum can ever be executed, and there is deliberately no — and will be no —
arbitrary shell execution entry point.

That is why the panel itself does not need to run as root: anything requiring root is pushed down into `zapexec`.

## Frontend

`web/` is a Vue 3 + Element Plus + TypeScript single-page application:

- `src/api/` — backend API wrappers, one file per resource type
- `src/views/` — page components. The sidebar menu is **served by the backend `menus` table** and mapped to routes
  by `src/utils/menu-to-routes.ts`; a component path must match the menu record's `component` field, otherwise that
  menu entry is skipped
- `src/i18n/locales/` — `zh-CN.ts` and `en-US.ts` maintained as a pair
- `src/icons/` — icons re-exported by name from Element Plus icons
