# Frequently Asked Questions (FAQ)

> Source document: `FAQ_zh-CN.md` (located at the root of the repository).
> Copied by `build.sh` to `data/www/html/FAQ_zh-CN.md`.
> The backend `GET /api/docs/faq?lang=zh-CN` is rendered into HTML and displayed.

## Q1: How to modify the port

- Modify the zap.yaml file
  ```yaml
  server:
  address: 0.0.0.0
  port: 2663
  # If set to 'zap', the backend access must include the prefix: https://<server>:port/zap; otherwise 404 error
  url_prefix: ""
  ```