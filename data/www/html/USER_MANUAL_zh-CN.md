# 用户手册

> 文档源：`USER_MANUAL_zh-CN.md`（位于仓库根）。  
> 由 `build.sh` 拷贝到 `data/www/html/USER_MANUAL_zh-CN.md`，  
> 后端 `GET /api/docs/manual?lang=zh-CN` 渲染为 HTML 后展示。

## 安装 / 升级

详见 [升级指南](./UPGRADE.md)。

## 常用入口

| 功能 | 入口 | 备注 |
|---|---|---|
| 添加站点 | 站点 → 添加站点 | 支持 PHP / 反向代理 / 静态 |
| 部署证书 | SSL/TLS | 申请 + 部署一键完成 |
| 定时任务 | 文档 → 用户手册 / 定时任务 | cron 表达式 |
| 文件管理 | 文件管理 | 远端服务器 + 桌面客户端 |
| 应用商店 | 应用商店 | 一键安装 GitHub 上架的应用 |

## 应用部署（Application Manager）

入口：**站点 → 应用管理**。把一个长驻进程（Git 仓库或站点目录里的程序）托管成 systemd 服务运行，
并自动反代到站点域名下。支持类型：**Python / Node.js / 静态 / Go / Rust**。

### 通用流程

1. 选代码来源：Git 仓库（可指定分支、子目录、浅克隆深度）或直接使用站点已有目录。
2. 填**构建命令**（可选）：部署时先执行，如 `npm run build`、`go build -o bin/app .`、`cargo build --release`。
3. 填**启动命令 / 入口**：运行时如何拉起进程。
4. 应用需监听 **`PORT` 环境变量**：面板会自动分配端口并注入 `PORT=xxxx`，nginx 反代打到 `127.0.0.1:PORT`。

### Go / Rust（编译型，单版本）

Go 与 Rust **不做多版本管理**：服务器装了哪个版本就用哪个，由管理员手动安装。
这与 Python / Node 不同——后两者由平台用 uv / fnm 自动安装多版本，而 Go / Rust 是全局唯一版本。

- **构建阶段**需要 `go` / `cargo`（执行 `go build` / `cargo build`），
  但**运行时是工作目录下的原生二进制**，不再依赖工具链。
- 执行端按以下顺序查找工具链，落到其一即可：
  `/usr/local/bin`、`/usr/bin`、`/usr/local/go/bin`、`/root/.cargo/bin`。
- **推荐装到 `/usr/local/bin`**（系统级、全局可读），这样所有站点用户（部署以站点归属的 unix 用户运行，非 root）都能用到。
  rustup 默认把工具链装到 `~/.cargo/bin`（仅当前用户），需要软链到全局才能让站点用户使用：

  ```bash
  # Rust：rustup 默认装在 /root/.cargo，软链到全局
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ln -s /root/.cargo/bin/cargo  /usr/local/bin/cargo
  ln -s /root/.cargo/bin/rustc  /usr/local/bin/rustc
  ln -s /root/.cargo/bin/rustup /usr/local/bin/rustup

  # Go：官方包默认在 /usr/local/go，软链 bin 到全局
  ln -s /usr/local/go/bin/go /usr/local/bin/go
  ```

- 应用必须读取 `PORT` 环境变量并绑定 `127.0.0.1`（与 Python / Node 约定一致）。
- Go 示例：构建命令 `go build -o bin/app .`，入口（相对工作目录的可执行路径）`bin/app`。
- Rust 示例：构建命令 `cargo build --release`，入口 `target/release/app`。
- 注意：Rust release 构建较慢（视机器而定）；Go（CGO）/ Rust 链接阶段需要 `cc` 等 C 工具链；
  备份站点时建议排除 `target/` 目录。

### 套餐能力控制

应用管理的总开关是套餐里的「允许应用管理」；**允许部署的类型**在套餐中勾选
（Python / Node.js / Go / Rust），留空 = 不限制（允许全部已支持类型）。

## 反馈

遇到问题请收集 `data/zap.log` 与浏览器 Network 截图，发到 issue tracker。