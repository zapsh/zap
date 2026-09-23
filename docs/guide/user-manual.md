---
title: 
---

# 用户手册

> 文档源：`USER_MANUAL_zh-CN.md`（位于仓库根）。  
> 由 `build.sh` 拷贝到 `data/www/html/USER_MANUAL_zh-CN.md`，  
> 后端 `GET /api/docs/manual?lang=zh-CN` 渲染为 HTML 后展示。

## 安装 / 升级

详见 [升级指南](./upgrade.md)。

## 常用入口

| 功能 | 入口 | 备注 |
|---|---|---|
| 添加站点 | 站点 → 添加站点 | 支持 PHP / 反向代理 / 静态 |
| 部署证书 | SSL/TLS | 申请 + 部署一键完成 |
| 定时任务 | 文档 → 用户手册 / 定时任务 | cron 表达式 |
| 文件管理 | 文件管理 | 远端服务器 + 桌面客户端 |
| 应用商店 | 应用商店 | 一键安装 GitHub 上架的应用 |
| 下载源 | 系统设置 → Zap 设置 → 下载源 | 应用商店取源码包的地址，可切国内 / Cloudflare / 本地目录 |

## 下载源（含离线环境）

应用商店安装软件时要从镜像取源码包（nginx / php / mysql / pcre2 等）。**这个地址是可配的**，
在「系统设置 → Zap 设置 → 下载源」里改，三种形态：

| 形态 | 取值 | 适用 |
|---|---|---|
| 国内镜像（默认） | `https://mirrors.zap.cn/pkg` | 国内机器 |
| Cloudflare 镜像 | `https://mirrors.zap.sh/pkg` | 海外机器，或国内源不通时 |
| 本地目录 | `/opt/zap-pkg`（保存后规范为 `file:///opt/zap-pkg`） | **离线机房**：没有外网 |

本地目录的目录结构与镜像一致，照着镜像的子路径放即可：

```text
/opt/zap-pkg/nginx/nginx-1.28.0.tar.gz
/opt/zap-pkg/php/php-8.3.6.tar.gz
/opt/zap-pkg/openssl/openssl-3.5.6.tar.gz
/opt/zap-pkg/mariadb/mariadb-11.4.5-….tar.gz
```

要点：

- 目录必须**预先存在**（否则保存时会拒绝：避免配了个空路径，装包时才发现取不到）
- 改动只对**下一次**安装生效，正在跑的任务不受影响
- 配置落在 `{ZAP_PATH}/data/mirror.yaml`（zapd 写、zapexec 读），手工改也生效
- 第三方包仓库的脚本若不读 `pkg_mirror`，仍会用它自己写死的地址

## 反馈

遇到问题请收集 `data/zap.log` 与浏览器 Network 截图，发到 issue tracker。