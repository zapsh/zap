# 常见问题（FAQ）

> 文档源：`FAQ_zh-CN.md`（位于仓库根）。  
> 由 `build.sh` 拷贝到 `data/www/html/FAQ_zh-CN.md`，  
> 后端 `GET /api/docs/faq?lang=zh-CN` 渲染为 HTML 后展示。

## Q1：如何修改端口

- 修改 zap.yaml 文件
  修改 
  ```yaml
  server:
    address: 0.0.0.0
    port: 2663
    #如果设置为 zap , 后端访问必须加前缀 : https://<server>:port/zap, 否则404
    url_prefix: ""
  ```
