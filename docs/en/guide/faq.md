---
title: FAQ
---

# FAQ

> Source document: `FAQ_zh-CN.md` (repository root).
> `build.sh` copies it to `data/www/html/FAQ_zh-CN.md`, and the backend
> `GET /api/docs/faq?lang=zh-CN` renders it as HTML.

## Q1: Login fails / token expired

- Check whether the browser blocks cookies.
- Check whether `jwt_secret` in `data/zapd.yaml` was reset after a restart.

## Q2: A newly added site returns 502

- Confirm PHP-FPM is running.
- Confirm the site `path` exists and has correct permissions.

## Q3: Some menu entries disappeared after an upgrade

- Check whether `data/zap.db` was modified by a migration script; the seed for the new version lives in
  `init_db.rs`. During development you can simply recreate the database.

## Q4: Cross-origin API calls return 401

- In `system/zap` → `JWT`, turn off `cookie_only`, or configure `Access-Control-Allow-Credentials` properly.
