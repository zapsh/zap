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

## Q5: A node goes offline / the node view returns to the login page

- If the node token was deleted or expired, edit the node under "Cluster → Nodes" and enter its credentials again to issue a new token; you can also paste a new token manually.
- The **Test** button only verifies login and issues no token; click **Save** to apply the new credentials.
- With only `system.monitor:view`, writes in the node view are rejected by the node; writes and terminal sessions also require `node:edit` on the master.
- A node-side 401 no longer sends the master back to its login page: fix node credentials under "Cluster → Nodes"; if the master session itself expires, log in to the master normally.
