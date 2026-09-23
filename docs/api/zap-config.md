---
title: Zap 设置（面板自身配置）
---

# Zap 设置（面板自身配置）

> 动态修改 zap.yaml 的 server.*：绑定 IP / 监听端口 / 面板 HTTPS 证书 / URL 前缀；仅 admin，保存后需重启 zapd 生效

## GET `/system/config/zap`

读取 Zap 设置：server 配置、当前面板证书信息（CN / 域名 / 有效期 / 是否与私钥匹配）、证书库可选证书、zap.yaml 路径与内容

```bash
curl -X GET "https://<host>/api/system/config/zap" \
  -H "Authorization: Bearer <token>"
```

## POST `/system/config/zap`

保存 Zap 设置，按 Tab 部分提交（只传本次修改的部分，未传字段保持不变）

```bash
curl -X POST "https://<host>/api/system/config/zap" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
     "server": "<object>",
     "ssl": "<object>"
   }'
```

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | :-: | --- |
| server | object | 否 | 服务设置：address（绑定 IP，如 0.0.0.0）、port（1-65535）、url_prefix（留空表示不启用前缀） |
| ssl | object | 否 | 证书设置：source（self-signed / library / manual）、cert_id（library 时必填）、cert_file、key_file、cert_content、key_content（manual 时必填，PEM 且需与证书匹配） |

## POST `/system/config/zap/ssl/self-sign`

删除现有证书文件并重新签发自签证书（重启 zapd 后生效）

```bash
curl -X POST "https://<host>/api/system/config/zap/ssl/self-sign" \
  -H "Authorization: Bearer <token>"
```

