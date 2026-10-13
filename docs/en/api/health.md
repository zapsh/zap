---
title: Health Check
---

# Health Check

> 无需认证

## GET `/health`

服务健康检查，返回版本与时间戳

```bash
curl -X GET "https://<host>/api/health" \
  -H "Authorization: Bearer <token>"
```

