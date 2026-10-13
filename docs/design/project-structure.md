---
title: 项目结构
---

# 项目结构

```
zap/
├── zapd/           # 面板主服务：Axum 路由、业务内核、任务调度
├── zapexec/        # root 特权守护进程：白名单动词执行
├── zap-proto/      # zapd ↔ zapexec 共享协议（帧编解码 + HMAC 认证）
├── zap-crypto/     # 统一加密库（AES-256-GCM）与服务凭据存储
├── zapctl/         # 命令行运维工具
├── zapupgrade/     # 系统升级器（upgrade / rollback）
├── zap-platform/   # 跨 UNIX 平台抽象层（开发中，尚未接入 zapd / zapexec）
├── zappro/         # 商业版模块（开源构建下为空占位）
├── web/            # Vue 3 + Element Plus 前端
├── docs/           # 文档站（VitePress，中英双语）
└── scripts/        # 安装 / 卸载、离线打包、systemd 单元
```

## 进程模型

| 进程 | 身份 | 职责 |
| --- | --- | --- |
| `zapd` | 普通用户（面板账号） | HTTP API、业务逻辑、数据库、任务调度 |
| `zapexec` | root | 以白名单动词执行特权操作 |

两者经 Unix Socket 通信，每个请求带 HMAC 签名。**`zap-proto` 的 `Request` 枚举是硬边界**：只有枚举里的动词能被执行，枚举中刻意不存在、将来也不会有任意 shell 执行入口。

因此面板自身不需要以 root 运行 —— 需要 root 的动作一律下沉到 `zapexec`。

## 前端

`web/` 是 Vue 3 + Element Plus + TypeScript 单页应用：

- `src/api/` —— 后端接口封装，每类资源一个文件
- `src/views/` —— 页面组件；侧栏菜单由**后端 `menus` 表下发**，经 `src/utils/menu-to-routes.ts` 映射成路由，组件路径须与菜单记录的 `component` 字段一致，否则该菜单会被跳过
- `src/i18n/locales/` —— `zh-CN.ts` 与 `en-US.ts` 成对维护
- `src/icons/` —— 图标统一从 Element Plus 图标具名导出
