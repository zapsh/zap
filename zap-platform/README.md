# zap-platform

ZAP 的**跨 UNIX 平台抽象层**。

** 该项目还在开发中，暂未接入zapd 和 zapexec ，ZAP 暂时支持Linux - Systemd **


## 目的

ZAP 面板的后端（如 `zapd`、`zapexec`）大量依赖操作系统底层能力：文件与目录操作、系统用户与权限、运行时环境探测、系统服务管理。这些逻辑目前散落在各 crate 中，且隐含了对 Linux 的假设，难以在 BSD 等其它 UNIX 上复用。

本 crate 的职责是**把这些操作系统相关的能力抽象成统一的 trait**，并提供 UNIX 系列（Linux + BSD）的实现，使上层业务只依赖接口、不感知具体系统。它是「先把抽象抽出来」的第一步：

- 定义四个子系统的 trait 边界；
- 实现 Linux / BSD 后端；
- **暂不接入任何现有 crate**（`zapd` / `zapexec` 等仍沿用各自内联实现，行为不受影响）；
- **不支持 macOS / Windows**（见下方「平台支持」）。

## 覆盖的子系统

| 子系统 | trait | 能力 |
| --- | --- | --- |
| 运行时环境探测 | `PlatformEnv` | 识别 OS / 发行版、探测 init 系统（systemd / SysV / OpenRC / Runit / BSD rc）、统一路径约定（`run_dir` / `service_dir`） |
| 文件系统 | `FileSystem` | 读写、权限（`chmod`）、属主（`chown`）、目录遍历、删除 |
| 用户与权限 | `UserManager` | 用户 / 组查询、由 root 降权（`drop_privileges`，即 `setgroups`+`setgid`+`setuid`）、当前 uid/gid |
| 服务管理 | `ServiceManager` | 启停 / 重启 / 重载 / 开机自启 / 状态查询，按探测到的 init 系统派发到 `systemctl` / `sv` / `/etc/init.d` / `/etc/rc.d` |
| Linux capabilities | `caps` | 读取进程有效 capability 集合（`/proc/self/status` 的 `CapEff`） |

## 设计

- **调用方只依赖 trait**：通过 `Platform::detect()` 取得当前平台的统一门面，门面内持有一组 `Box<dyn ...>` trait 对象，业务代码完全不感知 Linux 还是 BSD。
- **声明与实现分离**：`options` 式的字段定义之外，平台细节收敛在 `unix` 实现中；未来若要支持新平台，只需新增对应 `impl` 并扩展 `detect()`，调用方无需改动。
- **零后端侵入**：当前所有实现都基于标准 `std` 与 `libc`，不引入重量级依赖，也不修改其它 crate。

```rust
use zap_platform::Platform;

let p = Platform::detect()?;
println!("OS = {:?}, init = {:?}", p.env.os(), p.env.init_system());
p.service.start("nginx")?;
let me = p.user.lookup_user("root")?;
```

## 平台支持

- ✅ **Linux**（含发行版识别）
- ✅ **BSD**（FreeBSD / OpenBSD / NetBSD）
- ❌ **macOS / Windows 不支持** 

## 构建与测试

```bash
cargo build -p zap-platform
cargo test  -p zap-platform
```

> 说明：`caps::set_effective`（设置 capability）为占位实现，当前返回 `NotSupported`，写入能力需 libcap 绑定，留作后续扩展点。
