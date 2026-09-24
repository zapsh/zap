use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// serde `skip_serializing_if` 用的 false 判定（bool 字段默认值不写进 JSON）。
fn is_false(b: &bool) -> bool {
    !*b
}

/// 日志归档默认保留天数（site.log_rotate）
fn default_keep_days() -> u32 {
    30
}

/// 日志读取默认行数（site.log_read）
fn default_log_lines() -> usize {
    200
}

/// ACME HTTP-01 的一条验证材料：`token` 决定验证文件名，
/// `key_auth` 是 validation 服务端期望的响应体内容。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AcmeChallengeEntry {
    /// 挑战 token（文件名固定为该值）
    pub token: String,
    /// token + 账户指纹，验证端点取到的内容须与之一致
    pub key_auth: String,
}

/// 镜像构建参数：`--build-arg KEY=VALUE`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerBuildArg {
    /// 参数名（`[A-Za-z_][A-Za-z0-9_]*`，由调用方校验）
    pub key: String,
    #[serde(default)]
    pub value: String,
}

/// 反代 upstream 定义：vhost 渲染为 nginx `upstream <name> { ... }` 块。
/// server 行一律以 `servers_ext` 表单字段维护（开发期不兼容旧版文本 servers）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpstreamSpec {
    /// 上游组名（字母/数字/下划线，渲染前会再校验）
    pub name: String,
    /// 表单化的 server 行
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers_ext: Vec<UpstreamServer>,
    /// 负载均衡策略：空 = 默认（轮询）/ least_conn / ip_hash
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub balance: String,
}

/// upstream 内单个 server 行（表单化，渲染为
/// `server <addr> [weight=n] [max_fails=n] [fail_timeout=Ns] [backup] [down];`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpstreamServer {
    /// 上游地址：host:port / ip:port / http(s)://host[:port]
    pub addr: String,
    /// 权重（0 = 不设置，默认轮询权重 1）
    #[serde(default)]
    pub weight: u32,
    /// 失败上限次数（0 = 不设置，默认 1）
    #[serde(default)]
    pub max_fails: u32,
    /// 失败判定超时（秒，0 = 不设置）
    #[serde(default)]
    pub fail_timeout: u32,
    /// 备用节点（仅在主节点不可用时启用）
    #[serde(default)]
    pub backup: bool,
    /// 标记为停机（临时摘除，不参与转发）
    #[serde(default)]
    pub down: bool,
}

/// 自定义请求头（渲染为 `proxy_set_header <key> <value>;`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeaderSpec {
    /// Header 名（字母/数字/_/-，可含 `$var`）
    pub key: String,
    /// Header 值（可含 nginx 变量，如 `$host`、`$remote_addr`）
    pub value: String,
}

/// 自定义 location：反代（proxy）/ 跳转（redirect）/ 拒绝（deny）/
/// 站点内目录（alias）/ 自由指令体（raw）。
/// 新增字段均带默认值，旧 site_profile JSON 反序列化不受影响。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocationSpec {
    /// location 匹配路径，必须以 `/` 开头（如 `/`、`/api`）；不支持正则前缀
    pub path: String,
    /// proxy | redirect | deny | alias | raw
    pub kind: String,
    /// proxy：upstream 名 或 `http(s)://host[:port][/uri]`；
    /// redirect：跳转目标 URL；deny：目标为空；alias：站点内静态目录；
    /// raw：忽略，使用 `raw` 指令体
    pub target: String,
    /// redirect：跳转状态码（301/302，0 视为 301）；deny：拒绝状态码（403/404/410/444，0 视为 403）
    #[serde(default)]
    pub code: u16,
    /// proxy 时是否启用 WebSocket 升级（proxy_http_version 1.1 + Upgrade 头）
    #[serde(default)]
    pub ws: bool,
    // ── 高级参数（serde default，兼容旧数据）──
    /// kind=raw 时直接渲染的 location 指令体（多行 nginx 指令，逐行原样输出）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
    /// proxy_connect_timeout（秒，0 = 不输出）
    #[serde(default)]
    pub conn_timeout: u32,
    /// proxy_read_timeout（秒，0 = 不输出）
    #[serde(default)]
    pub read_timeout: u32,
    /// proxy_send_timeout（秒，0 = 不输出）
    #[serde(default)]
    pub send_timeout: u32,
    /// 追加的自定义请求头（proxy_set_header）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderSpec>,
    /// proxy_redirect 指令（off / 替换规则；空 = 不输出）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_redirect: String,
    /// 反代缓存：填 `zap_cache` 启用（由执行端自动创建共享缓存区）；
    /// 空 = 不缓存。仅固定白名单值。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cache: String,
    /// proxy_cache_valid 规则（如 `200 5m`；空 = 不输出）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cache_valid: String,
    /// 关闭代理缓冲（proxy_buffering off，SSE / 流式输出场景）
    #[serde(default)]
    pub no_buffering: bool,
}

/// `zapd` -> `zapexec` 的请求。只有白名单动词，刻意不提供任意 shell 执行。
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Request {
    /// 同步系统时钟（chrony / ntpdate）
    #[serde(rename = "time.sync")]
    TimeSync,
    /// 设置系统时区
    #[serde(rename = "time.set_timezone")]
    TimeSetTimezone { timezone: String },
    /// 列出可用时区
    #[serde(rename = "time.list_timezones")]
    TimeListTimezones,
    /// 读取当前时间/时区
    #[serde(rename = "time.get")]
    TimeGet,
    /// 读取 SSH 服务状态
    #[serde(rename = "ssh.status")]
    SshStatus,
    /// 重启 SSH 服务
    #[serde(rename = "ssh.restart")]
    SshRestart,
    /// 安装 SSH 服务端（openssh-server），日志写 run-{id}.log
    #[serde(rename = "ssh.install")]
    SshInstall { run_id: String },
    /// 列出系统服务（systemd）
    #[serde(rename = "service.list")]
    ServiceList,
    /// 对系统服务执行操作（start/stop/restart/reload/enable/disable）
    #[serde(rename = "service.action")]
    ServiceAction { name: String, action: String },
    /// 列出系统进程（ps）
    #[serde(rename = "process.list")]
    ProcessList,
    /// 终止进程（signal 缺省为 TERM，9 表示 KILL）
    #[serde(rename = "process.kill")]
    ProcessKill { pid: u32, signal: Option<String> },
    /// 把指定的公钥内容写入本机系统用户的 `~/.ssh/authorized_keys`（root 特权，
    /// 用于「我的密钥」的本地回环授权，公钥内容由 zapd 鉴权后下发）
    #[serde(rename = "ssh_key.install_pub")]
    SshKeyInstallPub {
        username: String,
        public_key: String,
    },
    /// 生成面板用户自己的 SSH 密钥（存于该用户家目录 `~/.ssh/zap_<name>`，属主为用户本人）
    #[serde(rename = "ssh_user_key.generate")]
    SshUserKeyGenerate {
        linux_user: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        key_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bits: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
    },
    /// 导入面板用户自己的 SSH 密钥（私钥写入用户家目录）
    #[serde(rename = "ssh_user_key.import")]
    SshUserKeyImport {
        linux_user: String,
        name: String,
        private_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        public_key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
    },
    /// 删除面板用户自己的 SSH 密钥（含 .pub）
    #[serde(rename = "ssh_user_key.delete")]
    SshUserKeyDelete { linux_user: String, name: String },
    /// 读取用户家目录私钥内容（仅终端连接时使用，私钥不落 DB / 不返回前端列表）
    #[serde(rename = "ssh_user_key.private_get")]
    SshUserKeyPrivateGet { linux_user: String, name: String },
    /// 读取用户家目录 ~/.ssh 下的 OpenSSH 默认私钥（id_ed25519 → id_ecdsa → id_rsa）。
    /// 只扫描该用户自己的家目录，不回退到其他账户；私钥同样不落 DB
    #[serde(rename = "ssh_user_key.default_get")]
    SshUserKeyDefaultGet { linux_user: String },
    /// 读取用户家目录公钥内容
    #[serde(rename = "ssh_user_key.public_get")]
    SshUserKeyPublicGet { linux_user: String, name: String },
    /// 列出用户家目录下全部面板密钥（`~/.ssh/zap_<name>.pub`）。
    /// 密钥只存家目录文件、不落 DB，故列表一律以磁盘扫描为准
    #[serde(rename = "ssh_user_key.list")]
    SshUserKeyList { linux_user: String },
    /// 读取主机名与 DNS 解析器配置
    #[serde(rename = "network.get")]
    NetworkGet,
    /// 设置主机名
    #[serde(rename = "network.set_hostname")]
    NetworkSetHostname { hostname: String },
    /// 设置 DNS Resolver（nameserver / search 写入 /etc/resolv.conf）
    #[serde(rename = "network.set_resolver")]
    NetworkSetResolver {
        nameservers: Vec<String>,
        #[serde(default)]
        search: Vec<String>,
    },
    /// 列出目录
    #[serde(rename = "file.list")]
    FileList { path: String },
    /// 读文件（文本）
    #[serde(rename = "file.read")]
    FileRead { path: String },
    /// 写文件（文本）
    ///
    /// `as_user`：以哪个 Linux 账号名义操作（None = root）；新建/覆盖的内容归属该账号。
    /// `skip_owner_check`：管理员置 true —— 内容归属自己，但仍可管理服务器上 root 的文件；
    /// 普通用户保持 false（只能删改本人文件）。
    #[serde(rename = "file.write")]
    FileWrite {
        path: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 删除文件/目录
    #[serde(rename = "file.delete")]
    FileDelete {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 建目录
    #[serde(rename = "file.mkdir")]
    FileMkdir {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 重命名
    #[serde(rename = "file.rename")]
    FileRename {
        path: String,
        new_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 下载（base64 字节）
    #[serde(rename = "file.download")]
    FileDownload { path: String },
    /// 上传（base64 字节）
    #[serde(rename = "file.upload")]
    FileUpload {
        path: String,
        name: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 文件信息
    #[serde(rename = "file.info")]
    FileInfo { path: String },
    /// 修改文件/目录权限（mode 为八进制数值，仅低 12 位有效）
    #[serde(rename = "file.chmod")]
    FileChmod {
        path: String,
        mode: u32,
        /// 递归应用到目录下的所有子项
        #[serde(default)]
        recursive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 修改文件/目录属主与属组（仅 admin）。`owner`/`group` 为 Linux 名称，None 表示不改该项。
    #[serde(rename = "file.chown")]
    FileChown {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<String>,
        /// 递归应用到目录下的所有子项
        #[serde(default)]
        recursive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 复制文件/目录（递归）
    #[serde(rename = "file.copy")]
    FileCopy {
        path: String,
        new_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 打包 zip：给出 `dest_dir` 就把压缩包写到该目录（打包到目录），
    /// 否则返回 zip 的 base64 内容供调用方下载。
    #[serde(rename = "file.archive")]
    FileArchive {
        paths: Vec<String>,
        name: String,
        base_dir: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dest_dir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_user: Option<String>,
        #[serde(default)]
        skip_owner_check: bool,
    },
    /// 添加 AppStore Git 源（clone 到 data/appstore/repos/<id>/）
    #[serde(rename = "appstore.repo_add")]
    AppstoreRepoAdd {
        name: String,
        url: String,
        run_id: String,
    },
    /// 删除 AppStore Git 源（内置源禁止删除）
    #[serde(rename = "appstore.repo_remove")]
    AppstoreRepoRemove { id: String },
    /// 更新单个 AppStore Git 源（clone/fetch + reset）
    #[serde(rename = "appstore.repo_update")]
    AppstoreRepoUpdate { id: String, run_id: String },
    /// 安装包：执行包的 install.sh（或 app.yaml 指定脚本）
    #[serde(rename = "appstore.install")]
    AppstoreInstall {
        /// 形如 database/mariadb 的包相对路径
        pkg_path: String,
        /// custom | official
        source: String,
        /// 包来源的 git 源 id（source=official 时定位 repos/<repo_id>/）
        #[serde(skip_serializing_if = "Option::is_none")]
        repo_id: Option<String>,
        version: String,
        /// 用户点击的操作（app.yaml actions 键，如 bin/build），透传给安装脚本的 ACTION 环境变量
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        /// 用户在安装表单中填写的选项（app.yaml options，键=选项名，值=字符串化表单值）
        #[serde(skip_serializing_if = "Option::is_none")]
        options: Option<BTreeMap<String, String>>,
        /// 实例名：同一个包的多个安装靠它区分（多版本 PHP → `74` / `83`；
        /// 站点类 → `site:<站点id>`）。缺省 `default`（老版本安装就是这个）。
        #[serde(skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        /// 发起操作的面板登录用户名（注入 ZAP_USER，供安装脚本按操作者归属）
        #[serde(skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        /// 面板侧编排结果（建站 / 建库），由 zapd 在入队前准备好并注入脚本 env。
        ///
        /// 键即环境变量名（`SITE_ROOT` / `DB_NAME` / `DB_PASS` …）。与 `options` 的区别：
        /// 这些值由面板产生（不是用户手填），可能含数据库密码，因此**不落 options.env /
        /// options.json**，只进子进程环境（run.json 由 zapexec 以 0600 写出）。
        #[serde(skip_serializing_if = "Option::is_none")]
        provision: Option<BTreeMap<String, String>>,
        /// 虚拟主机运行模式：固定 system（每个面板用户一个独立 Linux 账号）；注入 ZAP_RUN_MODE
        #[serde(skip_serializing_if = "Option::is_none")]
        run_mode: Option<String>,
        run_id: String,
    },
    /// 卸载包：执行 uninstall.sh 并删除已安装目录
    #[serde(rename = "appstore.uninstall")]
    AppstoreUninstall {
        pkg_path: String,
        /// 用户在卸载表单中填写的选项（app.yaml options.uninstall，键=选项名，值=字符串化表单值）
        #[serde(skip_serializing_if = "Option::is_none")]
        options: Option<BTreeMap<String, String>>,
        /// 实例名：同一个包的多个安装靠它区分（多版本 PHP → `74` / `83`；
        /// 站点类 → `site:<站点id>`）。缺省 `default`（老版本安装就是这个）。
        #[serde(skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        /// 安装时面板编排结果（站点 / 数据库），卸载时回传以便脚本备份或清理
        #[serde(skip_serializing_if = "Option::is_none")]
        provision: Option<BTreeMap<String, String>>,
        /// 发起操作的面板登录用户名（注入 ZAP_USER）
        #[serde(skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        /// 虚拟主机运行模式：固定 system（每个面板用户一个独立 Linux 账号）；注入 ZAP_RUN_MODE
        #[serde(skip_serializing_if = "Option::is_none")]
        run_mode: Option<String>,
        run_id: String,
    },
    /// 升级包：执行 upgrade.sh（缺省时先 uninstall.sh 再 install.sh）
    #[serde(rename = "appstore.upgrade")]
    AppstoreUpgrade {
        pkg_path: String,
        source: String,
        /// 包来源的 git 源 id（source=official 时定位 repos/<repo_id>/）
        #[serde(skip_serializing_if = "Option::is_none")]
        repo_id: Option<String>,
        version: String,
        old_version: String,
        /// 用户点击的操作（app.yaml actions 键），透传给升级脚本的 ACTION 环境变量
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        /// 升级表单选项（键=选项名，值=字符串化表单值）；缺省复用安装选项定义
        #[serde(skip_serializing_if = "Option::is_none")]
        options: Option<BTreeMap<String, String>>,
        /// 实例名：同一个包的多个安装靠它区分（多版本 PHP → `74` / `83`；
        /// 站点类 → `site:<站点id>`）。缺省 `default`（老版本安装就是这个）。
        #[serde(skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        /// 安装时的编排结果（站点 / 数据库）：升级脚本要接着用同一个站点与库，
        /// 与 uninstall 同源（provision.json，取不到时由 info.yaml 兜底）
        #[serde(skip_serializing_if = "Option::is_none")]
        provision: Option<BTreeMap<String, String>>,
        /// 发起操作的面板登录用户名（注入 ZAP_USER）
        #[serde(skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        /// 虚拟主机运行模式：固定 system（每个面板用户一个独立 Linux 账号）；注入 ZAP_RUN_MODE
        #[serde(skip_serializing_if = "Option::is_none")]
        run_mode: Option<String>,
        run_id: String,
    },
    /// 运行自定义脚本（仅限 `{data}/users/<username>/scripts/` 内）
    #[serde(rename = "appstore.script_run")]
    AppstoreScriptRun {
        path: String,
        run_id: String,
        username: String,
    },
    /// 停止运行中的任务（按 run_id 杀进程组）
    #[serde(rename = "appstore.script_stop")]
    AppstoreScriptStop { run_id: String },
    /// 读取自定义脚本内容（编辑前读取）
    #[serde(rename = "appstore.script_read")]
    AppstoreScriptRead { path: String, username: String },
    /// 写自定义脚本（仅限 `{data}/users/<username>/scripts/` 内）
    #[serde(rename = "appstore.script_write")]
    AppstoreScriptWrite {
        path: String,
        content: String,
        username: String,
    },
    /// 删除自定义脚本或目录（仅限 `{data}/users/<username>/scripts/` 内）
    #[serde(rename = "appstore.script_delete")]
    AppstoreScriptDelete { path: String, username: String },
    /// 列出一次运行的可编辑脚本快照（runs/<run_id>/pkg/ 递归文件树）
    #[serde(rename = "appstore.run_files")]
    AppstoreRunFiles { run_id: String },
    /// 读取运行快照内文件（仅限 runs/<run_id>/pkg/ 内）
    #[serde(rename = "appstore.run_file_read")]
    AppstoreRunFileRead { run_id: String, path: String },
    /// 写运行快照内文件（仅限 runs/<run_id>/pkg/ 内，用于修改脚本后重跑）
    #[serde(rename = "appstore.run_file_write")]
    AppstoreRunFileWrite {
        run_id: String,
        path: String,
        content: String,
    },
    /// 重跑某次失败的运行：复用其可编辑脚本快照（runs/<old_run_id>/pkg/），
    /// 以 new_run_id 新建日志/pid 重新执行原安装/卸载/升级动作。
    #[serde(rename = "appstore.run_retry")]
    AppstoreRunRetry { run_id: String, new_run_id: String },
    /// 扫描已安装应用列表（data/apps/*/meta.yaml + info.yaml + 运行状态）
    #[serde(rename = "appstore.installed")]
    AppstoreInstalled,
    /// 对已安装应用的实例执行启停（start/stop/restart，走其登记的 systemd 服务）
    #[serde(rename = "appstore.instance_action")]
    AppstoreInstanceAction {
        /// 形如 application/php 的包路径
        pkg_path: String,
        /// 实例名（缺省 default；站点类为 `site:<id>`）
        #[serde(skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        /// start | stop | restart
        action: String,
    },
    /// 同步站点 Nginx vhost：按站点渲染 conf 文件并 reload（幂等）
    #[serde(rename = "site.vhost_sync")]
    SiteVhostSync {
        /// site 表主键（vhost 文件名 zap-site-{id}.conf）
        site_id: i64,
        /// 站点名称（sanitize 后用于文档根目录名）
        name: String,
        /// 站点域名列表（server_name，多个以空格分隔）
        domains: Vec<String>,
        /// true 生成 vhost；false 移除 vhost（站点停用）
        /// 与 `mode` 同时存在时以 `mode` 为准（兼容老版本 zapd）
        enabled: bool,
        /// 站点运行状态：`running`（正常发布）/`stopped`（只撤下 sites-enabled 软链，
        /// 配置留在 sites-available 可秒级恢复）/`maintenance`（发布 503 维护页）。
        /// None 时按 `enabled` 回退为 running 或 stopped（兼容老版本 zapd）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// PHP-FPM 通道（如 unix:/var/run/php-fpm-8.3.sock 或 127.0.0.1:9000）；
        /// None 表示纯静态站点，不生成 PHP location
        #[serde(skip_serializing_if = "Option::is_none")]
        php_socket: Option<String>,
        /// 站点文档根目录（面板按归属用户家目录规划并入库，如 /home/u/www/blog-1）；
        /// None 时回退 {ZAP_PATH}/data/www/{sanitize(name)}-{site_id}
        #[serde(default, skip_serializing_if = "Option::is_none")]
        web_root: Option<String>,
        /// 站点日志目录（access.log / error.log 所在）；
        /// None 时 vhost 不生成日志指令（沿用 nginx 全局日志）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        log_root: Option<String>,
        /// 站点文件属主（Linux 账号名）；Some 时 web_root 整树归 `{owner}:{owner 主组}`
        /// （目录 755 / 文件 644，nginx 走 others 位读静态文件，不归属 www 组），
        /// log_root 整树仍归 www:www（目录 770 / 文件 660，nginx 写日志）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner_user: Option<String>,
        /// 站点类型：php（PHP/PHP+静态，默认）/ static（纯静态）/ proxy（反向代理）。
        /// proxy 类型忽略 web_root/php，按 locations 渲染反代规则。
        #[serde(default)]
        site_type: String,
        /// 伪静态预设 key：none / thinkphp / laravel / codeigniter / wordpress / custom
        #[serde(default)]
        pseudo_static: String,
        /// 伪静态自定义规则（多行 nginx 指令，pseudo_static=custom 时使用；仅 root/admin 可提交）
        #[serde(default)]
        pseudo_custom: String,
        /// web_root 是否为用户在归属家目录下选择的「已有目录」：
        /// true 时不自动创建目录、不写入默认 index.html
        #[serde(default)]
        web_root_custom: bool,
        /// 自定义 upstream 组（渲染于 server 块之前）
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        upstreams: Vec<UpstreamSpec>,
        /// 自定义 location（按提交顺序渲染，排在默认 location 之前）
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        locations: Vec<LocationSpec>,
        /// SSL/TLS：站点绑定的证书库证书内容（fullchain = 叶子 + 中间链 PEM）。
        /// Some 时由 zapexec 落盘并渲染 listen 443 ssl；None = 不启用 HTTPS。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ssl_fullchain: Option<String>,
        /// SSL/TLS：证书私钥 PEM（与 ssl_fullchain 成对出现）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ssl_key: Option<String>,
        /// 允许 HTTP 跳转到 HTTPS（仅 SSL 启用时生效：80 端口只保留 301 跳转）
        #[serde(default)]
        force_https: bool,
        /// TLS 协议版本（空格分隔的 nginx ssl_protocols 值，如 "TLSv1.2 TLSv1.3"）；
        /// 空串时由执行端回退为 TLSv1.2 TLSv1.3
        #[serde(default, skip_serializing_if = "String::is_empty")]
        ssl_protocols: String,
        /// SSL 密码套件（nginx ssl_ciphers 值）；空串 = 不输出 ssl_ciphers 指令（跟随执行端默认）
        #[serde(default, skip_serializing_if = "String::is_empty")]
        ssl_ciphers: String,
        /// 服务端密码套件优先（ssl_prefer_server_ciphers，仅影响 TLSv1.2）
        #[serde(default, skip_serializing_if = "is_false")]
        ssl_prefer_server_ciphers: bool,
        /// 是否启用 HTTP/2（nginx ≥ 1.25.1 渲染 `http2 on;`，旧版回退 `listen 443 ssl http2`）
        #[serde(default, skip_serializing_if = "is_false")]
        ssl_http2: bool,
        /// 共享主机 IPv4：非空时 vhost 监听 `listen {ip}:80` / `listen {ip}:443 ssl`
        /// （来自面板基础设置的「默认 IPv4」）；空 = 沿用 `listen 80`（通配所有地址）。
        #[serde(default, skip_serializing_if = "String::is_empty")]
        listen_ipv4: String,
        /// 共享主机 IPv6：非空时 vhost 监听 `listen [{ip}]:80` / `listen [{ip}]:443 ssl`；
        /// 空 = 沿用 `listen [::]:80`。
        #[serde(default, skip_serializing_if = "String::is_empty")]
        listen_ipv6: String,
    },
    /// 列出目录下的子目录（root 特权）：供面板站点「选择已有站点目录」浏览。
    /// 仅返回目录名（不含点目录），路径必须为绝对路径且存在。
    #[serde(rename = "fs.browse_dirs")]
    FsBrowseDirs { base: String },
    /// 批量统计目录占用的字节数（root 特权）：`du -sb`。
    /// zapd 以 zapadm 运行，而用户家目录多为 0700（属主为各自的 Linux 账号），
    /// 面板进程 du 读不到，必须由 root 代跑。
    /// 返回 `data.usage`：`{ 绝对路径: 字节数 }`；读不到的路径（不存在 / 统计失败）不出现在结果里。
    #[serde(rename = "fs.disk_usage")]
    FsDiskUsage {
        /// 待统计的目录（绝对路径；非法 / 越权路径会被执行端直接丢弃）
        paths: Vec<String>,
    },
    /// 站点 nginx 日志轮转（root）：按天把 access.log / error.log 切割为
    /// `{kind}.log-YYYYMMDD` 并 gzip 归档，清理超期归档，最后通知 nginx 重新打开日志。
    #[serde(rename = "site.log_rotate")]
    SiteLogRotate {
        /// 待轮转的站点日志目录（面板规划 `{home}/logs/{site_id}-{name}`）
        log_roots: Vec<String>,
        /// 归档保留天数（超过即删除）
        #[serde(default = "default_keep_days")]
        keep_days: u32,
    },
    /// 站点日志归档列表（`access.log-YYYYMMDD.gz` / `error.log-YYYYMMDD.gz`）
    #[serde(rename = "site.log_list")]
    SiteLogList { log_root: String },
    /// 读取站点日志（当前日志或归档）尾部行，支持关键词 / 状态码过滤
    #[serde(rename = "site.log_read")]
    SiteLogRead {
        /// 站点日志目录
        log_root: String,
        /// access | error（空 = access）
        #[serde(default)]
        kind: String,
        /// 归档文件名（空 = 当前日志）
        #[serde(default, skip_serializing_if = "String::is_empty")]
        archive: String,
        /// 返回的最大行数（从文件尾部取）
        #[serde(default = "default_log_lines")]
        lines: usize,
        /// 关键词过滤（空 = 不过滤）
        #[serde(default, skip_serializing_if = "String::is_empty")]
        keyword: String,
        /// access.log 状态码过滤（如 `404`；空 = 不过滤）
        #[serde(default, skip_serializing_if = "String::is_empty")]
        status: String,
    },
    /// 清空站点当前日志（truncate）并通知 nginx 重新打开
    #[serde(rename = "site.log_clear")]
    SiteLogClear {
        /// 站点日志目录
        log_root: String,
        /// access | error；空 = 两者都清空
        #[serde(default)]
        kind: String,
    },
    /// 防火墙状态：探测后端（firewalld / ufw / nftables / iptables）并返回规则列表
    #[serde(rename = "firewall.status")]
    FirewallStatus {
        /// 面板自身监听端口（用于标记"受保护"规则，避免把自己锁在外面）
        panel_port: u16,
    },
    /// 新增防火墙规则（放行 / 拒绝）
    #[serde(rename = "firewall.rule_add")]
    FirewallRuleAdd {
        /// 1 - 65535
        port: u16,
        /// tcp | udp
        proto: String,
        /// accept | drop
        action: String,
        /// 来源地址（IP 或 CIDR）；空串表示不限制来源
        #[serde(default)]
        source: String,
        /// 备注（firewalld 写入 rich rule / ufw 写入 comment）
        #[serde(default)]
        comment: String,
        /// 面板自身监听端口（用于拒绝"拒绝面板端口"这类自杀式操作）
        panel_port: u16,
    },
    /// 删除防火墙规则（id 由 status 返回，按后端语义解析）
    #[serde(rename = "firewall.rule_delete")]
    FirewallRuleDelete {
        id: String,
        /// 面板自身监听端口
        panel_port: u16,
    },
    /// 防火墙服务启停 / 开机自启：action = start | stop | enable | disable
    #[serde(rename = "firewall.toggle")]
    FirewallToggle { action: String },

    /// 移除站点 Nginx vhost（站点删除时清理，幂等）
    #[serde(rename = "site.vhost_remove")]
    SiteVhostRemove { site_id: i64, name: String },
    /// 删除站点数据目录（root 特权）：站点删除时若勾选「同时删除网站数据」，
    /// 文档根（网站文件）与日志目录（access.log / error.log 及其归档）一起 rm -rf。
    /// 安全边界由执行端兜底：路径须为绝对路径、不含 `..`，
    /// 且位于用户家目录（/home/*）或面板数据目录（{ZAP_PATH}/data/{www,logs}）之内，
    /// 不允许是这些容器目录本身。
    #[serde(rename = "site.data_remove")]
    SiteDataRemove {
        /// 站点文档根（web_root）列表
        web_roots: Vec<String>,
        /// 站点日志目录（log_root）列表：随网站数据一并删除
        log_roots: Vec<String>,
    },
    /// 探测服务器运行环境快照（OS / Web 服务器 / PHP / 数据库 / 工具链）
    #[serde(rename = "env.detect")]
    EnvDetect,
    /// 初始化面板用户家目录骨架：mkdir -p {home_dir}/www {home_dir}/logs {home_dir}/tmp（root 特权）。
    /// owner 为该面板用户对应的 Linux 账号：
    /// home 711 owner {u}:{u}，www 755 owner {u}:{u 主组}（nginx worker 走 others 位读静态文件），
    /// logs 770 owner www:www（nginx 写日志），tmp 700 owner {u}:{u}。
    #[serde(rename = "user.home_init")]
    UserHomeInit {
        home_dir: String,
        /// 运行账号（Linux 系统用户名），家目录树归该账号所有
        owner: String,
    },
    /// 将用户家目录整体迁移到新挂载点（如 /home → /home2，磁盘扩容场景）。
    /// 支持跨文件系统（mv 失败自动回退 cp -a + 清理源）；完成后按该账号重置属主，
    /// 并同步更新系统账号家目录指针（usermod -d）。
    #[serde(rename = "user.home_migrate")]
    UserHomeMigrate {
        /// 源家目录完整路径（绝对路径，不含 `..`）
        src_home: String,
        /// 目标家目录完整路径（须为挂载点下路径，basename 与源一致）
        dest_home: String,
        /// 运行账号（Linux 系统用户名）
        owner: String,
    },
    /// 为面板用户创建 Linux 系统账号（useradd，nologin，home 指向 home_dir），幂等。
    /// 每个面板用户对应一个 Linux 账号（nologin），在 user.add / 站点同步前调用。
    #[serde(rename = "user.system_init")]
    UserSystemInit {
        /// Linux 账号名（须通过 zap_proto::linux_username 派生，调用方已校验）
        linux_user: String,
        /// 账号 home 目录（面板记录的 home_dir）
        home_dir: String,
    },
    /// 移除 Linux 系统账号（删除面板用户时调用）。
    /// 会先清掉该用户的 PHP-FPM pool 配置文件并 reload，再 userdel。
    #[serde(rename = "user.system_remove")]
    UserSystemRemove {
        /// Linux 账号名
        linux_user: String,
    },
    /// 设置 Linux 系统账号的磁盘配额（root 特权，best-effort）。
    /// quota_mb = 0 表示取消配额（不限）。
    /// 自动适配 ext 系列（setquota）与 xfs（xfs_quota）；工具缺失或文件系统未启用
    /// quota 时返回错误，调用方（zapd）仅记录告警，不阻断用户创建流程。
    #[serde(rename = "user.quota_set")]
    UserQuotaSet {
        /// Linux 账号名（须通过 zap_proto::linux_username 派生，调用方已校验）
        linux_user: String,
        /// 配额（MB，0 = 不限）
        quota_mb: i64,
    },
    /// 按用户生成 PHP-FPM pool 配置并 reload（幂等）：
    /// 写入 {php 安装}/etc/php-fpm.d/{linux_user}.conf，
    /// listen unix:/var/run/php-fpm-{linux_user}-{php版本}.sock，worker 以 {linux_user} 运行，
    /// 规格来自 spec（JSON 字符串，缺省用面板默认值）。
    #[serde(rename = "php.pool_sync")]
    PhpPoolSync {
        /// PHP 实例标识（如 php8.3）
        php_instance: String,
        /// Linux 账号名（worker 运行身份 / pool 名 / socket 名）
        linux_user: String,
        /// 用户家目录（open_basedir / session / upload 隔离根）
        home_dir: String,
        /// fpm pool 规格 JSON 字符串；空 = 使用默认规格
        spec: String,
    },
    /// 查询执行器自身信息（版本 / 安装目录），供系统更新页展示 zapexec 版本
    #[serde(rename = "upgrade.info")]
    UpgradeInfo,
    /// 触发一次系统升级：在独立 systemd scope 中拉起 zapupgrade，
    /// 由它在不依赖 zapd/zapexec 存活的前提下完成 校验/备份/替换/重启。
    /// stage_dir：zapd 下载解包规整后的目录（须位于 {ZAP_PATH}/data/upgrade/stage/ 下）；
    /// log_path：zapd 准备的日志文件（同数据区，upgrade/logs/ 下）。
    #[serde(rename = "upgrade.run")]
    UpgradeRun {
        run_id: String,
        stage_dir: String,
        log_path: String,
    },
    /// 探测 Nginx 运行状态：安装位置 / 版本 / 主配置 / 运行态（未安装时返回 installed=false）
    #[serde(rename = "nginx.status")]
    NginxStatus,
    /// 列出可编辑的 Nginx 配置文件（主配置 + conf 目录白名单，*.conf）
    #[serde(rename = "nginx.conf_list")]
    NginxConfList,
    /// 读取某个白名单内 Nginx 配置文件的内容
    #[serde(rename = "nginx.conf_read")]
    NginxConfRead { path: String },
    /// 保存配置：备份 → 写入 → `nginx -t` 校验 → 失败回滚 → 运行中则重载
    #[serde(rename = "nginx.conf_save")]
    NginxConfSave { path: String, content: String },
    /// Nginx 服务控制：start / stop / restart / reload（优先 systemd unit nginx）
    #[serde(rename = "nginx.control")]
    NginxControl { action: String },
    /// 设置默认站点（IP / 未匹配域名兜底）形态：
    /// enable=false → 直接断开（444 / 443 拒握手，防串站）；
    /// enable=true → IP / 未匹配域名展示欢迎页
    #[serde(rename = "nginx.default_vhost")]
    NginxDefaultVhost { enable: bool },
    /// 查询 / 设置 Nginx 状态页 stub_status：
    /// enable=None → 仅查询是否启用并采集指标；
    /// enable=Some(true/false) → 开启 / 关闭状态页（托管于 127.0.0.1 本机端口）
    #[serde(rename = "nginx.stub_status")]
    NginxStubStatus { enable: Option<bool> },
    /// 通用服务配置·状态探测（服务配置页：php / mysql / mariadb / docker，未安装时 installed=false）
    #[serde(rename = "service_conf.status")]
    ServiceConfStatus { service: String },
    /// 通用服务配置·列出可编辑配置文件（主配置 + 配置目录白名单）
    #[serde(rename = "service_conf.list")]
    ServiceConfList { service: String },
    /// 通用服务配置·读取指定配置文件内容（Query: path）
    #[serde(rename = "service_conf.read")]
    ServiceConfRead { service: String, path: String },
    /// 通用服务配置·保存配置文件（备份 → 原子写入；可选校验由各服务定义决定）
    #[serde(rename = "service_conf.save")]
    ServiceConfSave {
        service: String,
        path: String,
        content: String,
    },
    /// 通用服务配置·读取关键项表单（字段定义 + 当前值，写入主配置）
    #[serde(rename = "service_conf.keys")]
    ServiceConfKeys { service: String },
    /// 通用服务配置·保存关键项（写入主配置文件托管区 / JSON 合并）
    #[serde(rename = "service_conf.keys_save")]
    ServiceConfKeysSave {
        service: String,
        keys: std::collections::BTreeMap<String, String>,
    },
    /// 通用服务配置·服务控制 start / stop / restart / reload
    #[serde(rename = "service_conf.control")]
    ServiceConfControl { service: String, action: String },
    /// 通用服务配置·列出服务已安装的版本实例（php74 / php81 …），目前 php 可用
    #[serde(rename = "service_conf.instances")]
    ServiceConfInstances { service: String },
    /// 通用服务配置·设置 / 取消某实例的「全局默认访问」（注册到 /usr/local/bin）
    #[serde(rename = "service_conf.default")]
    ServiceConfDefault { service: String, enable: bool },
    /// 读取已加密保存的服务凭据（由 `zapctl cred gen <服务> <用户>` 生成）。
    /// zapexec 以 root 读取 `/etc/zap/credentials/{service}_{user}.cred` 并用面板主密钥解密，
    /// 用于创建数据库 / 初始化服务时取回密码。仅回传明文，不做任何写操作。
    #[serde(rename = "cred.read")]
    CredRead { service: String, user: String },
    /// 面板用户的计划任务（crontab）：以指定 Linux 账号运行一条命令 / 脚本。
    ///
    /// - `linux_user`：实际执行身份（已由 zapd 校验；非 admin 恒为其自身账号）
    /// - `home_dir`：非空且存在时作为工作目录
    /// - `kind`：`script`（`command` 为脚本绝对路径）或 `command`（`command` 为 sh 命令体）
    /// - `log_path`：输出追加写入的日志文件（须位于 `{ZAP_PATH}/data/users/` 之下）
    ///
    /// 后台执行：立即返回，结束后在日志末尾追加 `__ZAP_DONE__ <exit_code>`。
    #[serde(rename = "cron.run")]
    CronRun {
        run_id: String,
        linux_user: String,
        home_dir: String,
        command: String,
        kind: String,
        log_path: String,
    },
    /// 写入 ACME HTTP-01 验证文件（root 特权，供 Let's Encrypt 自动验证）。
    ///
    /// 落盘到面板自管的验证根
    /// `{ZAP_PATH}/data/www/_zap/acme/.well-known/acme-challenge/{token}`；
    /// 站点 vhost 里固定渲染了
    /// `location ^~ /.well-known/acme-challenge/ { alias <验证根>/.well-known/acme-challenge/; }`，
    /// 因此无需重启 / 重载 nginx 即可生效。
    /// token 仅允许 `[A-Za-z0-9_-]`（防路径穿越），文件 0644、目录 0755。
    #[serde(rename = "acme.http_write")]
    AcmeHttpWrite { entries: Vec<AcmeChallengeEntry> },
    /// 清理 ACME HTTP-01 验证文件（幂等）。订单成功 / 失败 / 取消后都应收尾调用。
    #[serde(rename = "acme.http_clear")]
    AcmeHttpClear { tokens: Vec<String> },
    // ── Docker 容器管理（面板「容器」）──────────────────────
    /// 环境探测：docker 是否安装、守护进程是否可用、compose 插件是否存在。
    #[serde(rename = "docker.status")]
    DockerStatus,
    /// 容器列表。`all = true` 时含已停止容器（`docker container ls -a`）。
    #[serde(rename = "docker.containers")]
    DockerContainers { all: bool },
    /// 容器批量动作：start / stop / restart / pause / unpause / kill / remove。
    #[serde(rename = "docker.container_action")]
    DockerContainerAction { ids: Vec<String>, action: String },
    /// 容器快启：等价 `docker run -d`（创建 + 启动一步到位），返回新容器 ID。
    ///
    /// 供镜像列表的「启动」按钮使用 —— 只透传白名单字段，参数走 Engine API 而不拼命令。
    /// `name` 为空时由 daemon 自动起名；`ports` 支持 `[IP:]宿主机端口:容器端口[/udp]`。
    #[serde(rename = "docker.container_run")]
    DockerContainerRun {
        image: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        ports: Vec<String>,
        /// 重启策略：空 / no / always / unless-stopped / on-failure
        #[serde(default)]
        restart: String,
    },
    /// 容器详情：透传 `docker container inspect` 的原始 JSON 对象。
    #[serde(rename = "docker.container_inspect")]
    DockerContainerInspect { id: String },
    /// 容器日志尾部（一次性拉取；前端按需轮询实现跟随）。
    #[serde(rename = "docker.container_logs")]
    DockerContainerLogs {
        id: String,
        tail: Option<u32>,
        since: Option<String>,
        timestamps: bool,
    },
    /// 实时资源快照（`docker stats --no-stream`）：CPU / 内存 / 网络 / 磁盘 IO。
    #[serde(rename = "docker.stats")]
    DockerStats,
    /// 镜像列表（含悬空镜像）。
    #[serde(rename = "docker.images")]
    DockerImages,
    /// 镜像动作：pull（id 为仓库引用）/ remove（id 为镜像 ID 或引用）/ prune。
    #[serde(rename = "docker.image_action")]
    DockerImageAction { id: String, action: String },
    /// 镜像详情：`inspect` 原始 JSON + 构建历史（每层对应的构建指令）。
    #[serde(rename = "docker.image_inspect")]
    DockerImageInspect { id: String },
    /// 用 Containerfile / Dockerfile 构建镜像。
    ///
    /// 构建是长任务，因此和其它运行记录一样「先登记、后后台执行」：stdout / stderr
    /// 逐行追加写入 `log_path`，进程退出后追加 `__ZAP_DONE__ <code>`，
    /// 前端用 `/appstore/ws/{run_id}` 看实时日志。
    #[serde(rename = "docker.image_build")]
    DockerImageBuild {
        run_id: String,
        log_path: String,
        /// 构建上下文目录（绝对路径）
        context_dir: String,
        /// Containerfile 绝对路径，必须位于上下文目录之内
        containerfile: String,
        /// 目标镜像名（含 tag），可多个
        tags: Vec<String>,
        #[serde(default)]
        build_args: Vec<DockerBuildArg>,
        /// 目标平台（如 linux/amd64）；空 = 跟随宿主架构
        #[serde(default)]
        platform: String,
        #[serde(default)]
        no_cache: bool,
        #[serde(default)]
        pull: bool,
    },
    /// 数据卷列表。
    #[serde(rename = "docker.volumes")]
    DockerVolumes,
    /// 数据卷动作：create / remove / prune。
    ///
    /// `owner_home` / `owner_user` 非空时，create 建的是 bind mount 卷：
    /// 数据落在 `{owner_home}/volumes/{name}`，并归 `owner_user` 这个 Linux 账号所有 ——
    /// 多用户环境下卷数据要进用户自己的配额、也能跟着 home 一起备份，
    /// 而不是闷在 `/var/lib/docker/volumes` 里（那里只有 root 看得到）。
    /// 两者为空（旧客户端）或 remove / prune 时，退回 daemon 默认位置。
    #[serde(rename = "docker.volume_action")]
    DockerVolumeAction {
        name: String,
        action: String,
        #[serde(default)]
        owner_home: String,
        #[serde(default)]
        owner_user: String,
    },
    /// 网络列表。
    #[serde(rename = "docker.networks")]
    DockerNetworks,
    /// 网络动作：create（可指定 driver）/ remove / prune。
    #[serde(rename = "docker.network_action")]
    DockerNetworkAction {
        name: String,
        action: String,
        driver: Option<String>,
    },
    /// Compose 项目列表（`docker compose ls -a`）。
    #[serde(rename = "docker.compose_list")]
    DockerComposeList,
    /// Compose 动作：up / down / start / stop / restart / pull / update。
    ///
    /// 项目配置文件从 `compose ls` 结果中反查（受管项目再兜底到 stacks 目录），
    /// 避免前端直接传任意路径。
    #[serde(rename = "docker.compose_action")]
    DockerComposeAction { project: String, action: String },
    /// 读取 Compose 项目的配置文件内容（面板的 yaml 预览 / 编辑）。
    ///
    /// 只读 `compose ls` 反查到的路径，或受管目录
    /// `{ZAP_PATH}/data/stacks/<project>/compose.yaml`：不接受前端传路径，
    /// 否则就是一个「读任意文件」的口子。
    #[serde(rename = "docker.compose_file")]
    DockerComposeFile { project: String },
    /// 新建 / 覆盖受管项目的 compose.yaml（面板「+ Compose」与「导入」）。
    ///
    /// 文件固定落在 `{ZAP_PATH}/data/stacks/<project>/compose.yaml`，
    /// `project` 只允许 docker 的项目名字符集，写盘前自行挡路径穿越。
    #[serde(rename = "docker.compose_save")]
    DockerComposeSave { project: String, content: String },
    /// 删除项目：先 `down` 再清理受管目录；`compose ls` 里的外部项目只 down、
    /// 不动它的文件（那些文件不归面板管）。
    #[serde(rename = "docker.compose_remove")]
    DockerComposeRemove { project: String },
    /// 项目日志尾部（`docker compose logs --tail N`），一次性拉取。
    #[serde(rename = "docker.compose_logs")]
    DockerComposeLogs { project: String, tail: u32 },
    /// 容器内交互式终端（`docker exec -it` 的等价物）。
    ///
    /// 这是**长会话**：stdin 需要持续输入、stdout 需要增量回传，
    /// 因此不走普通 `Request` 的一问一答，而是由 `Message::StreamOpen` 承载。
    /// `cmd` 为空时退回 `sh`。
    #[serde(rename = "docker.container_exec")]
    DockerContainerExec {
        id: String,
        cmd: Vec<String>,
        /// 容器内执行用户（`user[:group]`），为空用镜像默认用户
        user: Option<String>,
        cols: u16,
        rows: u16,
    },
    /// 守护进程实时事件流（`docker events` 的等价物）。
    ///
    /// 同样是长会话：连接期间持续推送事件，断开即结束，因此也走 `Message::StreamOpen`。
    #[serde(rename = "docker.events")]
    DockerEvents,
}

/// `zapexec` -> `zapd` 的响应。
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    /// 0 = 成功，非 0 = 错误（沿用 `ZapError` 的 code 约定）
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok(message: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            code: 0,
            message: message.into(),
            data,
        }
    }

    pub fn err(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// 握手与数据阶段共用的消息封装。
///
/// `Request` / `Response` 载荷较大（含站点同步等大字段），装箱存放：
/// 避免 `Message` 的栈上尺寸被最大变体撑大（各变体尺寸差异 10 倍以上）。
/// serde 序列化对 `Box<T>` 透明，线上 JSON 格式与未装箱时完全一致。
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// server -> client：随机挑战（hex）
    Challenge { challenge: String },
    /// client -> server：HMAC-SHA256(secret, challenge) 的 hex
    Auth { mac: String },
    /// server -> client：握手成功
    Welcome,
    /// client -> server：请求
    Request(Box<Request>),
    /// server -> client：响应
    Response(Box<Response>),
    // ── 流式会话（容器 exec 终端）─────────────────────────
    //
    // 一问一答的 `Request` / `Response` 承载不了交互式终端：stdin 要持续输入、
    // stdout 要增量回传。这里复用**同一条已认证的 Unix 连接**跑会话，
    // 由 `id` 区分（理论上可并发多路，目前 exec 一路一连接）。
    /// client -> server：开启会话
    StreamOpen { id: String, req: Box<Request> },
    /// client -> server：会话输入（**base64** 编码的字节，JSON 无法直接承载二进制）
    StreamIn { id: String, data: String },
    /// client -> server：TTY 窗口尺寸变更
    StreamResize { id: String, cols: u16, rows: u16 },
    /// client -> server：关闭会话（关闭 stdin，等待命令退出）
    StreamClose { id: String },
    /// server -> client：会话就绪（可以开始收发数据）
    StreamReady { id: String },
    /// server -> client：会话输出（base64）
    StreamOut { id: String, data: String },
    /// server -> client：会话正常结束；`code` 为容器内命令退出码
    StreamEnd { id: String, code: i32 },
    /// server -> client：会话异常结束（容器不存在 / 未运行 / 无该 shell …）
    StreamError { id: String, message: String },
}

/// 站点/目录名安全规范化（zapd 与 zapexec 共用）：
/// 仅保留 ASCII 字母数字与 `_`/`-`，其余（含 `.`、空格、路径分隔符等）替换为 `-`，
/// 再去除首尾 `-`；空结果回退 `site`，最长 48 字符。
/// 与 zapexec `verbs/site.rs` 的历史实现保持一致，避免「面板侧记录的目录名」与
/// 「执行端实际创建的目录名」分叉。
pub fn sanitize_site_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    out = out.trim_matches('-').to_string();
    if out.is_empty() {
        out = "site".to_string();
    }
    if out.chars().count() > 48 {
        out = out.chars().take(48).collect();
    }
    out
}

/// 面板用户名 → Linux 系统账号名派生（与家目录末段一致，home_dir 统一
/// `/home/{linux_username}`）：
/// 1. 经 `sanitize_site_name` 清洗；2. 转小写；
/// 3. 首位必须是 ascii 字母或 `_`（数字/`-` 开头补 `z` 前缀，保证 useradd 合法）；
/// 4. 最长 24 字符（Linux 用户名上限 32，留裕量给面板其它后缀）。
pub fn linux_username(username: &str) -> String {
    let clean = sanitize_site_name(username);
    let mut base: String = clean
        .chars()
        .take(23)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let first_ok = base
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !first_ok {
        // base 以数字或 `-` 开头（sanitize 已去首尾 `-`，这里多为数字开头/空）
        if base.is_empty() {
            base = "zapuser".to_string();
        } else {
            base.insert(0, 'z');
        }
    }
    if base.len() > 24 {
        base.truncate(24);
    }
    base
}

/// docker 镜像引用白名单：`[命名空间/]名称[:tag]`（zapd 与 zapexec 共用）。
///
/// 只允许小写字母数字与 `. _ - /`（docker 本身也拒绝大写仓库名，提前拦一次能给出
/// 中文提示而不是 daemon 的英文长串），tag 部分额外允许大写。
pub fn valid_image_ref(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    let (name, tag) = match s.split_once(':') {
        Some((n, t)) => (n, Some(t)),
        None => (s, None),
    };
    if let Some(t) = tag
        && (t.is_empty()
            || t.len() > 128
            || !t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
    {
        return false;
    }
    if name.is_empty() || name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return false;
    }
    name.split('/').all(|seg| {
        !seg.is_empty()
            && !seg.starts_with('.')
            && seg.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            })
    })
}

/// 补全默认 tag：`nginx` → `nginx:latest`（docker 的默认行为，显式化便于展示 / 去重）。
pub fn normalize_image_tag(tag: &str) -> String {
    let tag = tag.trim();
    let last = tag.rsplit('/').next().unwrap_or(tag);
    if last.contains(':') {
        tag.to_string()
    } else {
        format!("{tag}:latest")
    }
}

/// 面板用户名 → docker 命名空间前缀（多用户隔离用）。
///
/// 直接复用 [`linux_username`] 的派生规则：它已经保证「小写 + 仅 `[a-z0-9_-]`」，
/// 且不含 `.`（首段带点会被 docker 当成 registry 主机名）。这样
/// 「面板账号 / 家目录 / 系统账号 / 镜像命名空间」四者是同一个名字，用户好记。
pub fn docker_namespace(username: &str) -> String {
    linux_username(username)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_uses_whitelist_verbs() {
        assert_eq!(
            serde_json::to_string(&Request::TimeSync).unwrap(),
            r#"{"verb":"time.sync"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::SshKeyInstallPub {
                username: "admin".into(),
                public_key: "ssh-ed25519 AAAA".into(),
            })
            .unwrap(),
            r#"{"verb":"ssh_key.install_pub","username":"admin","public_key":"ssh-ed25519 AAAA"}"#
        );
    }

    #[test]
    fn request_round_trip() {
        let req = Request::FileWrite {
            path: "/tmp/a.txt".into(),
            content: "hello".into(),
            as_user: None,
            skip_owner_check: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{req:?}"), format!("{back:?}"));
    }

    #[test]
    fn appstore_verbs_tagging() {
        assert_eq!(
            serde_json::to_string(&Request::AppstoreInstalled).unwrap(),
            r#"{"verb":"appstore.installed"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::AppstoreInstall {
                pkg_path: "database/mariadb".into(),
                source: "official".into(),
                repo_id: Some("zap-appstore".into()),
                version: "11.4.4".into(),
                action: None,
                options: None,
                instance: None,
                provision: None,
                user: None,
                run_mode: None,
                run_id: "r1".into(),
            })
            .unwrap(),
            r#"{"verb":"appstore.install","pkg_path":"database/mariadb","source":"official","repo_id":"zap-appstore","version":"11.4.4","run_id":"r1"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::AppstoreInstall {
                pkg_path: "application/php".into(),
                source: "official".into(),
                repo_id: Some("zap-appstore".into()),
                version: "8.3.3".into(),
                action: Some("build".into()),
                options: None,
                instance: None,
                provision: Some(
                    [("DB_NAME".to_string(), "u_wp1".to_string())]
                        .into_iter()
                        .collect()
                ),
                user: None,
                run_mode: None,
                run_id: "r2".into(),
            })
            .unwrap(),
            r#"{"verb":"appstore.install","pkg_path":"application/php","source":"official","repo_id":"zap-appstore","version":"8.3.3","action":"build","provision":{"DB_NAME":"u_wp1"},"run_id":"r2"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::AppstoreRepoAdd {
                name: "My Store".into(),
                url: "https://github.com/user/store.git".into(),
                run_id: "r9".into(),
            })
            .unwrap(),
            r#"{"verb":"appstore.repo_add","name":"My Store","url":"https://github.com/user/store.git","run_id":"r9"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::AppstoreRepoRemove {
                id: "my-store".into(),
            })
            .unwrap(),
            r#"{"verb":"appstore.repo_remove","id":"my-store"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::AppstoreRepoUpdate {
                id: "zap-appstore".into(),
                run_id: "r9".into(),
            })
            .unwrap(),
            r#"{"verb":"appstore.repo_update","id":"zap-appstore","run_id":"r9"}"#
        );
    }

    #[test]
    fn appstore_instance_action_tagging() {
        assert_eq!(
            serde_json::to_string(&Request::AppstoreInstanceAction {
                pkg_path: "application/php".into(),
                instance: None,
                action: "stop".into(),
            })
            .unwrap(),
            r#"{"verb":"appstore.instance_action","pkg_path":"application/php","action":"stop"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::SiteVhostSync {
                site_id: 1,
                name: "blog".into(),
                domains: vec!["a.com".into(), "b.com".into()],
                enabled: true,
                mode: None,
                php_socket: Some("unix:/var/run/php-fpm-8.3.sock".into()),
                web_root: None,
                log_root: None,
                owner_user: None,
                site_type: "php".into(),
                pseudo_static: "none".into(),
                pseudo_custom: String::new(),
                web_root_custom: false,
                upstreams: vec![],
                locations: vec![],
                ssl_fullchain: None,
                ssl_key: None,
                force_https: false,
                ssl_protocols: String::new(),
                ssl_ciphers: String::new(),
                ssl_prefer_server_ciphers: false,
                ssl_http2: false,
                listen_ipv4: String::new(),
                listen_ipv6: String::new(),
            })
            .unwrap(),
            r#"{"verb":"site.vhost_sync","site_id":1,"name":"blog","domains":["a.com","b.com"],"enabled":true,"php_socket":"unix:/var/run/php-fpm-8.3.sock","site_type":"php","pseudo_static":"none","pseudo_custom":"","web_root_custom":false,"force_https":false}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::SiteVhostRemove {
                site_id: 2,
                name: "x".into(),
            })
            .unwrap(),
            r#"{"verb":"site.vhost_remove","site_id":2,"name":"x"}"#
        );
    }

    #[test]
    fn env_verb_tagging() {
        assert_eq!(
            serde_json::to_string(&Request::EnvDetect).unwrap(),
            r#"{"verb":"env.detect"}"#
        );
        // env.detect 需能经 serde 反序列化回来（白名单内部路由用）
        let back: Request = serde_json::from_str(r#"{"verb":"env.detect"}"#).unwrap();
        assert!(matches!(back, Request::EnvDetect));
    }

    #[test]
    fn site_vhost_sync_with_dirs() {
        let req = Request::SiteVhostSync {
            site_id: 1,
            name: "blog".into(),
            domains: vec!["a.com".into()],
            enabled: true,
            mode: None,
            php_socket: None,
            web_root: Some("/home/zap/www/blog-1".into()),
            log_root: Some("/home/zap/logs/1-blog".into()),
            owner_user: Some("zap".into()),
            site_type: "php".into(),
            pseudo_static: "none".into(),
            pseudo_custom: String::new(),
            web_root_custom: false,
            upstreams: vec![],
            locations: vec![],
            ssl_fullchain: None,
            ssl_key: None,
            force_https: false,
            ssl_protocols: String::new(),
            ssl_ciphers: String::new(),
            ssl_prefer_server_ciphers: false,
            ssl_http2: false,
            listen_ipv4: String::new(),
            listen_ipv6: String::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(
            json,
            r#"{"verb":"site.vhost_sync","site_id":1,"name":"blog","domains":["a.com"],"enabled":true,"web_root":"/home/zap/www/blog-1","log_root":"/home/zap/logs/1-blog","owner_user":"zap","site_type":"php","pseudo_static":"none","pseudo_custom":"","web_root_custom":false,"force_https":false}"#
        );
        // 老版本 JSON（无 web_root/log_root）也能反序列化成功 → None
        let old: Request =
            serde_json::from_str(r#"{"verb":"site.vhost_sync","site_id":1,"name":"blog","domains":["a.com"],"enabled":true,"php_socket":"unix:/var/run/php-fpm-8.3.sock"}"#)
                .unwrap();
        match old {
            Request::SiteVhostSync {
                web_root, log_root, ..
            } => {
                assert!(web_root.is_none());
                assert!(log_root.is_none());
            }
            _ => panic!("应解析为 SiteVhostSync"),
        }
    }

    #[test]
    fn user_home_init_tagging() {
        assert_eq!(
            serde_json::to_string(&Request::UserHomeInit {
                home_dir: "/home/zap".into(),
                owner: "zap".into(),
            })
            .unwrap(),
            r#"{"verb":"user.home_init","home_dir":"/home/zap","owner":"zap"}"#
        );
        let back: Request = serde_json::from_str(
            r#"{"verb":"user.home_init","home_dir":"/home/zap","owner":"zap"}"#,
        )
        .unwrap();
        assert!(
            matches!(back, Request::UserHomeInit { home_dir, owner } if home_dir == "/home/zap" && owner == "zap")
        );
    }

    #[test]
    fn sanitize_site_name_helper() {
        assert_eq!(sanitize_site_name("我的 博客"), "site");
        assert_eq!(sanitize_site_name("my blog/x"), "my-blog-x");
        assert_eq!(sanitize_site_name(".."), "site");
        assert_eq!(sanitize_site_name("ABC_123"), "ABC_123");
        // 最长 48
        let long = "a".repeat(60);
        assert_eq!(sanitize_site_name(&long).chars().count(), 48);
    }

    #[test]
    fn linux_username_helper() {
        assert_eq!(linux_username("zap"), "zap");
        assert_eq!(linux_username("Zap_Admin"), "zap_admin");
        // 全非法 → sanitize 回退 site → 小写 site
        assert_eq!(linux_username("我的 博客"), "site");
        assert_eq!(linux_username("123abc"), "z123abc");
        // 前导 - 被 sanitize 修剪
        assert_eq!(linux_username("-x"), "x");
        let long = "A".repeat(40);
        let got = linux_username(&long);
        assert!(got.len() <= 24);
        assert!(got.chars().next().unwrap().is_ascii_alphabetic());
    }

    #[test]
    fn upgrade_verbs_tagging() {
        assert_eq!(
            serde_json::to_string(&Request::UpgradeInfo).unwrap(),
            r#"{"verb":"upgrade.info"}"#
        );
        let json = serde_json::to_string(&Request::UpgradeRun {
            run_id: "r1".into(),
            stage_dir: "/usr/local/zap/data/upgrade/stage/r1".into(),
            log_path: "/usr/local/zap/data/upgrade/logs/run-r1.log".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"verb":"upgrade.run","run_id":"r1","stage_dir":"/usr/local/zap/data/upgrade/stage/r1","log_path":"/usr/local/zap/data/upgrade/logs/run-r1.log"}"#
        );
        // 白名单之外必须被拒绝（回归）
        let err = serde_json::from_str::<Request>(r#"{"verb":"shell.exec","cmd":"id"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn php_pool_sync_tagging() {
        let req = Request::PhpPoolSync {
            php_instance: "php8.3".into(),
            linux_user: "zap".into(),
            home_dir: "/home/zap".into(),
            spec: "{\"pm\":\"dynamic\",\"max_children\":8}".into(),
        };
        let back: Request = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        assert_eq!(format!("{req:?}"), format!("{back:?}"));
    }

    #[test]
    fn appstore_verbs_round_trip() {
        let req = Request::AppstoreScriptWrite {
            path: "scripts/admin/backup.sh".into(),
            content: "#!/bin/bash\necho hi".into(),
            username: "admin".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{req:?}"), format!("{back:?}"));
    }

    #[test]
    fn unknown_verb_rejected() {
        // 白名单之外（例如任意 shell 执行）必须被拒绝
        let err = serde_json::from_str::<Request>(r#"{"verb":"shell.exec","cmd":"id"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn missing_field_rejected() {
        let err = serde_json::from_str::<Request>(r#"{"verb":"file.read"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn response_helpers() {
        let ok = Response::ok("done", Some(serde_json::json!({ "a": 1 })));
        assert_eq!(ok.code, 0);
        assert_eq!(ok.message, "done");
        assert!(ok.data.is_some());

        let err = Response::err(7, "boom");
        assert_eq!(err.code, 7);
        assert_eq!(err.message, "boom");
        assert!(err.data.is_none());
    }

    #[test]
    fn response_omits_none_data() {
        let err = Response::err(1, "x");
        let json = serde_json::to_string(&err).unwrap();
        assert!(
            !json.contains("data"),
            "data=None 时不应序列化该字段: {json}"
        );
    }

    #[test]
    fn message_round_trip_all_variants() {
        let msgs = vec![
            Message::Challenge {
                challenge: "abc".into(),
            },
            Message::Auth { mac: "def".into() },
            Message::Welcome,
            Message::Request(Box::new(Request::TimeSetTimezone {
                timezone: "Asia/Shanghai".into(),
            })),
            Message::Response(Box::new(Response::ok("ok", None))),
        ];
        for m in msgs {
            let json = serde_json::to_string(&m).unwrap();
            let back: Message = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{m:?}"), format!("{back:?}"), "json: {json}");
        }
    }

    #[test]
    fn message_type_tagging() {
        assert_eq!(
            serde_json::to_string(&Message::Welcome).unwrap(),
            r#"{"type":"welcome"}"#
        );
        let m: Message = serde_json::from_str(r#"{"type":"challenge","challenge":"x"}"#).unwrap();
        assert!(matches!(m, Message::Challenge { challenge } if challenge == "x"));
    }
}
