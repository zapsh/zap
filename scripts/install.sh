#!/bin/bash
# ZAP 服务器/VPS 管理系统 一键安装脚本
set -euo pipefail

# ── 终端颜色 ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
# 用 printf 而非 `echo -e`：dash/sh 的 echo 不解析 -e，会原样输出 "-e [✗] ..."
info() { printf "${BLUE}[*]${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*"; }
die()  { printf "${RED}[✗]${NC} %s\n" "$*" >&2; exit 1; }

usage() {
    cat <<'EOF'
用法: sudo bash install.sh [VERSION] [选项]

  VERSION                要安装的版本号（默认 latest）

初始管理员凭据（仅首次安装、全新数据库时生效）：
  --admin-user <name>    管理员用户名（默认 admin）
                         同时作为 Linux 账号名，家目录为 /home/<name>
  --admin-pass <pass>    管理员密码；不指定则自动生成随机密码并打印
                         可用字符：字母、数字、. _ -（避免破坏 env 文件解析）
  环境变量                ZAP_ADMIN_USER / ZAP_ADMIN_PASSWORD（命令行参数优先）

说明：安装末尾会执行 `zapd --init-admin` 建库并写入管理员（凭据只经命令行传递，
不落任何文件）；已安装过的机器重新执行本脚本不会改动现有管理员密码。

  -h, --help             显示本帮助

示例:
  sudo bash install.sh latest --admin-user zapops --admin-pass 'S3cret-Pass'
  sudo ZAP_ADMIN_PASSWORD='S3cret-Pass' bash install.sh
EOF
}

# 16 位随机密码：优先 base64（可读性好），没有则退回十六进制
gen_password() {
    local raw
    raw=$(head -c 12 /dev/urandom 2>/dev/null | base64 2>/dev/null | tr -d '\n=+/') || raw=""
    [ -n "$raw" ] || raw=$(od -An -N12 -tx1 /dev/urandom 2>/dev/null | tr -d ' \n') || raw=""
    [ -n "$raw" ] || raw="zap$(date +%s)"
    printf '%s' "${raw:0:16}"
}

# ── 解释器检查（脚本用到 bash 数组等特性，sh/dash 下行为异常）──
if [ -z "${BASH_VERSION:-}" ]; then
    printf "${RED}[✗]${NC} %s\n" "请使用 bash 执行：sudo bash $0" >&2
    exit 1
fi

# ── 权限检查 ────────────────────────────────────────────────
[ "$(id -u)" -eq 0 ] || die "请以 root 身份运行：sudo bash $0"

printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   ZAP 服务器/VPS 管理系统 · 安装程序${NC}\n"
printf "${GREEN}========================================${NC}\n"

# ── 解析参数：版本号（位置参数）+ 初始管理员凭据 ───────────
VERSION="latest"
ADMIN_USER=""; ADMIN_PASS=""; PASS_GENERATED=0; ADMIN_UNCHANGED=0
while [ $# -gt 0 ]; do
    case "$1" in
        --admin-user)
            ADMIN_USER="${2:-}"
            [ -n "$ADMIN_USER" ] || die "--admin-user 缺少用户名"
            shift 2 ;;
        --admin-user=*)
            ADMIN_USER="${1#*=}"
            [ -n "$ADMIN_USER" ] || die "--admin-user 缺少用户名"
            shift ;;
        --admin-pass|--admin-password)
            ADMIN_PASS="${2:-}"
            [ -n "$ADMIN_PASS" ] || die "--admin-pass 缺少密码"
            shift 2 ;;
        --admin-pass=*|--admin-password=*)
            ADMIN_PASS="${1#*=}"
            [ -n "$ADMIN_PASS" ] || die "--admin-pass 缺少密码"
            shift ;;
        -h|--help)
            usage; exit 0 ;;
        -*)
            die "未知参数: $1（--help 查看用法）" ;;
        *)
            VERSION="$1"; shift ;;
    esac
done

# 命令行未指定时回落到环境变量，再回落到默认值
[ -n "$ADMIN_USER" ] || ADMIN_USER="${ZAP_ADMIN_USER:-admin}"
[ -n "$ADMIN_PASS" ] || ADMIN_PASS="${ZAP_ADMIN_PASSWORD:-}"

# 用户名即 Linux 账号名（家目录 /home/<name>），必须满足 useradd 约束
ADMIN_USER=$(printf '%s' "$ADMIN_USER" | tr 'A-Z' 'a-z')
[ -n "$ADMIN_USER" ] || die "管理员用户名不能为空"
[ "${#ADMIN_USER}" -le 32 ] || die "管理员用户名过长（≤32 字符）: ${ADMIN_USER}"
case "$ADMIN_USER" in
    [a-z_]*) ;;
    *) die "管理员用户名需以小写字母或 _ 开头: ${ADMIN_USER}" ;;
esac
case "$ADMIN_USER" in
    *[!a-z0-9_-]*) die "管理员用户名只能包含 a-z 0-9 _ -: ${ADMIN_USER}" ;;
esac

if [ -z "$ADMIN_PASS" ]; then
    ADMIN_PASS=$(gen_password)
    PASS_GENERATED=1
fi
# 密码会写进 systemd 的 env 文件，含空白 / 引号 / $ 会破坏解析
case "$ADMIN_PASS" in
    *[!A-Za-z0-9._-]*) die "管理员密码只能包含字母、数字以及 . _ -（避免破坏 env 文件解析）" ;;
esac
[ "${#ADMIN_PASS}" -ge 8 ] || warn "管理员密码不足 8 位，建议登录后修改"
# ── 平台探测：操作系统 + 服务管理器 ─────────────────────────
# 服务管理器决定后面怎么装/启服务：
#   Linux          → systemd（systemctl）
#   FreeBSD        → rc.d + service(8)，开机自启归 sysrc 管（它没有 rcctl）
#   OpenBSD        → rc.d + rcctl
# RCD_DIR 是 rc.d 脚本的落点：FreeBSD 的第三方服务装在 /usr/local/etc/rc.d/，
# 系统自带的才在 /etc/rc.d/；OpenBSD 一律在 /etc/rc.d/。
OS=$(uname -s)
case "$OS" in
    Linux)
        command -v systemctl >/dev/null 2>&1 \
            || die "未检测到 systemctl：Linux 上目前仅支持 systemd 作为服务管理器"
        INIT=systemd
        OS_PKG=linux
        ;;
    FreeBSD)
        INIT=rcd
        OS_PKG=freebsd
        RCD_DIR=/usr/local/etc/rc.d
        ;;
    OpenBSD)
        INIT=rcd
        OS_PKG=openbsd
        RCD_DIR=/etc/rc.d
        ;;
    *)
        die "不支持的操作系统: ${OS}（当前支持 Linux、FreeBSD、OpenBSD）"
        ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)              ARCH="amd64" ;;
    aarch64|arm64|arm*)  ARCH="arm64" ;;
    ppc64le)             ;;
    s390x)               ;;
    *) die "不支持的架构: $ARCH" ;;
esac
info "目标版本: ${VERSION}   系统: ${OS} (${INIT})   架构: ${ARCH}"

# ── 下载工具 ────────────────────────────────────────────────
# OpenBSD 默认没有 wget，自带的是 ftp(1)
if command -v wget >/dev/null 2>&1; then
    fetch_url()  { wget -q -O - "$1"; }
    fetch_file() { wget -O "$2" "$1"; }
elif command -v ftp >/dev/null 2>&1; then
    fetch_url()  { ftp -o - "$1"; }
    fetch_file() { ftp -o "$2" "$1"; }
elif command -v fetch >/dev/null 2>&1; then
    # FreeBSD 的 fetch(1)
    fetch_url()  { fetch -o - "$1"; }
    fetch_file() { fetch -o "$2" "$1"; }
else
    die "未找到 wget 或 ftp，无法下载安装包"
fi

# ── 解析 latest 版本号 ──────────────────────────────────────
DOWNLOAD_ZAP_URL="https://mirrors.zap.cn/zap/releases"
if [ "$VERSION" = "latest" ]; then
    info "查询最新版本..."
    if LATEST=$(fetch_url "${DOWNLOAD_ZAP_URL}/latest.txt?t=$(date +%s)") && [ -n "$LATEST" ]; then
        VERSION="$LATEST"
        info "最新版本: ${VERSION}"
    else
        warn "无法查询最新版本，使用 latest 标签"
    fi
fi

ZAP_FILENAME="zap-v${VERSION}-${OS_PKG}-${ARCH}.tar.gz"

# ── 下载 ────────────────────────────────────────────────────
if [ -f "$ZAP_FILENAME" ]; then
    info "使用已存在的安装包 ${ZAP_FILENAME}"
else
    info "下载 ${ZAP_FILENAME} ..."
    # fetch_file 需要两个参数：URL + 落盘路径
    # 先写 .part 再改名：中途失败不会留下半截包，被下次运行当成完整包解压
    rm -f "${ZAP_FILENAME}.part"
    fetch_file "${DOWNLOAD_ZAP_URL}/${ZAP_FILENAME}" "${ZAP_FILENAME}.part" \
        || die "下载失败，请检查网络或版本号"
    [ -s "${ZAP_FILENAME}.part" ] || die "下载内容为空: ${DOWNLOAD_ZAP_URL}/${ZAP_FILENAME}"
    mv -f "${ZAP_FILENAME}.part" "${ZAP_FILENAME}"
fi

# ── 创建运行用户 ───────────────────────────────────────────
# 幂等创建：用户已存在则跳过；同名组已存在时改为加入现有组，
# 避免 adduser 报 "The group `www' already exists" 中断安装。
#
# 工具选择：优先 useradd（shadow-utils，Debian / Ubuntu / RHEL / CentOS / Rocky
# 等发行版均自带，参数一致）；只有 Debian 系才有的 adduser 作为回退。
# OpenBSD 的 useradd 是 BSD 版，参数不兼容，单独走一个分支。
# 用法：create_user <用户名> <1=系统用户|0=普通用户>

# 组是否存在：getent 是 glibc 工具，OpenBSD 上没有，退化到读 /etc/group
has_group() {
    if command -v getent >/dev/null 2>&1; then
        getent group "$1" >/dev/null 2>&1
    else
        grep -q "^$1:" /etc/group
    fi
}

create_user() {
    local user="$1" is_system="$2"
    if id "$user" >/dev/null 2>&1; then
        ok "用户 ${user} 已存在"
        return 0
    fi

    info "创建用户 ${user}"
    local group_exists=0
    if has_group "$user"; then
        group_exists=1
    fi

    # FreeBSD：没有 shadow-utils 的 useradd，只有 pw(8)。pw useradd 会自动建
    # 同名主组，所以不需要 -g；nologin 在 /usr/sbin 下，与 OpenBSD 的 /sbin 不同。
    if [ "$OS" = "FreeBSD" ]; then
        pw useradd "$user" -s /usr/sbin/nologin -d /nonexistent \
            || die "创建 ${user} 用户失败"
        ok "用户 ${user} 创建完成"
        return 0
    fi

    # OpenBSD：useradd 没有 -r / -M，默认也不建家目录；nologin 在 /sbin 下
    if [ "$OS" = "OpenBSD" ]; then
        local -a bopts=(-s /sbin/nologin)
        [ "$group_exists" = "1" ] && bopts+=(-g "$user")
        useradd "${bopts[@]}" "$user" || die "创建 ${user} 用户失败"
        ok "用户 ${user} 创建完成"
        return 0
    fi

    if command -v useradd >/dev/null 2>&1; then
        local -a opts=(-s /bin/false -M)
        [ "$is_system" = "1" ] && opts+=(-r)
        if [ "$group_exists" = "1" ]; then
            opts+=(-g "$user")
        elif command -v groupadd >/dev/null 2>&1; then
            # 先建同名主组（系统用户对应系统组），个别环境不支持 useradd -U 时也能用
            local -a gopts=()
            [ "$is_system" = "1" ] && gopts+=(-r)
            groupadd "${gopts[@]}" "$user" 2>/dev/null || true
            if has_group "$user"; then
                opts+=(-g "$user")
            else
                opts+=(-U)
            fi
        else
            opts+=(-U)
        fi
        useradd "${opts[@]}" "$user" || die "创建 ${user} 用户失败"
    elif command -v adduser >/dev/null 2>&1; then
        local -a opts=(--shell /bin/false --no-create-home --disabled-password --disabled-login)
        [ "$is_system" = "1" ] && opts+=(--system)
        if [ "$group_exists" = "1" ]; then
            opts+=(--ingroup "$user")
        else
            opts+=(--group)
        fi
        adduser "${opts[@]}" "$user" || die "创建 ${user} 用户失败"
    else
        die "未找到 useradd / adduser，无法创建 ${user} 用户"
    fi
    ok "用户 ${user} 创建完成"
}

# ── 初始管理员：Linux 账号 + 家目录骨架 ────────────────────
# 结构对齐 zapexec 的 user.home_init（家目录 711、www 755、logs 770 归 www、
# tmp 700），区别只是这里由安装脚本以 root 直接建好，不必等面板首次启动。
#
# 非 Linux 平台跳过：BSD 的 useradd / pw 参数差异较大，交给 zapd 启动后经
# zapexec 补齐（zapd::zap::admin_bootstrap），功能不缺。
# 用法：create_admin_account <用户名> <家目录>
create_admin_account() {
    local user="$1" home="$2"
    if [ "$OS" != "Linux" ]; then
        warn "非 Linux 平台，跳过预建 ${user} 账号与家目录（面板首次启动时自动补齐）"
        return 0
    fi
    local gname="$user"
    # 同名组已被系统占用（如发行版预置的 admin 组）时改用专属组，避免继承额外权限
    if has_group "$user"; then
        gname="zap_${user}"
        if ! has_group "$gname" && command -v groupadd >/dev/null 2>&1; then
            groupadd "$gname" 2>/dev/null || warn "创建组 ${gname} 失败（继续）"
        fi
    fi
    if id "$user" >/dev/null 2>&1; then
        ok "Linux 账号 ${user} 已存在"
    else
        local shell
        shell=$(command -v nologin 2>/dev/null || true)
        [ -n "$shell" ] || shell=/usr/sbin/nologin
        # -m：连家目录一起建（面板账号需要真实家目录承载 www/logs/tmp）
        if has_group "$gname"; then
            useradd -m -d "$home" -s "$shell" -g "$gname" "$user" || die "创建 ${user} 用户失败"
        else
            useradd -m -d "$home" -s "$shell" -U "$user" || die "创建 ${user} 用户失败"
        fi
        ok "Linux 账号 ${user} 已创建（${home}，nologin）"
    fi
    # 家目录骨架（幂等：已存在时只校正归属与权限）
    mkdir -p "$home/www" "$home/logs" "$home/tmp" || die "创建家目录骨架失败: ${home}"
    chown -R "${user}:${gname}" "$home/www" 2>/dev/null || true
    chmod 755 "$home/www"
    chown -R www:www "$home/logs" 2>/dev/null || true
    chmod 770 "$home/logs"
    chown -R "${user}:${gname}" "$home/tmp" 2>/dev/null || true
    chmod 700 "$home/tmp"
    chown "${user}:${gname}" "$home" 2>/dev/null || true
    chmod 711 "$home"
    ok "家目录已就绪: ${home}（www/logs/tmp）"
}

# www：站点运行用户（普通用户）；zapadm：面板运维用户（系统用户）
create_user www 0
create_user zapadm 1

# ── 解压（解压到临时目录，避免污染当前目录）──────────────────
info "解压安装包..."
WORK_DIR=$(mktemp -d /tmp/zap-install.XXXXXX) || die "无法创建临时目录"
trap 'rm -rf "$WORK_DIR"' EXIT
tar zxf "$ZAP_FILENAME" -C "$WORK_DIR" || die "解压失败，安装包可能已损坏"

# 发行包布局（唯一，由 build.sh 保证）：整包内容都在 zap/ 下，
# 二进制与 scripts / data 同级，因此二进制与资源共用同一个内容根。
SRC="$WORK_DIR/zap"
[ -d "$SRC" ] || die "安装包格式不正确：缺少 zap/ 目录"
info "安装包内容目录: ${SRC}"

# ── AppStore 官方仓库地址 ──────────────────────────────────
# 官方包脚本存放于独立 git 仓库，便于单独升级；面板中可添加/更换其他源
APPSTORE_REPO_URL="${APPSTORE_REPO_URL:-https://github.com/zapj/zap-appstore.git}"

# ── AppStore 目录部署（多 Git 源，幂等：不覆盖 repos/.git 与 custom/）──
deploy_appstore() {
    local DEST="$ZAP_DIR/data/appstore"
    local BUILTIN="$DEST/repos/zap-appstore"
    mkdir -p "$DEST"/{repos,custom,cache,tmp,logs}
    mkdir -p "$ZAP_DIR/data/apps"

    # 仅在缺失时复制模板/配置文件，避免覆盖用户修改
    [ -f "$DEST/repos.yaml" ]       || cp -f "$SRC/data/appstore/repos.yaml" "$DEST/repos.yaml" 2>/dev/null || true
    [ -f "$DEST/custom/README.md" ] || cp -f "$SRC/data/appstore/custom/README.md" "$DEST/custom/README.md" 2>/dev/null || true

    # 种子官方包（内置源）：复制发行包内置包（构建时从独立 git 仓库同步）作为离线兜底；
    # 发行包无内置包时留空，交由下方 git clone 拉取（离线则面板中可重试更新）
    if [ ! -d "$BUILTIN/.git" ] && [ ! -d "$BUILTIN/database" ]; then
        mkdir -p "$BUILTIN"
        for c in infra application webapps database library; do
            [ -d "$SRC/data/appstore/repos/zap-appstore/$c" ] && cp -Rf "$SRC/data/appstore/repos/zap-appstore/$c" "$BUILTIN/" 2>/dev/null || true
        done
    fi

    # 首次初始化官方 git 仓库（离线时保留种子包，面板中可重试更新）
    if [ ! -d "$BUILTIN/.git" ] && command -v git >/dev/null 2>&1; then
        info "初始化 AppStore 官方仓库..."
        if git clone -q --depth 1 "$APPSTORE_REPO_URL" "$DEST/repos/.tmp-zap-appstore" 2>/dev/null; then
            local has_seed
            # 不用 find -maxdepth（GNU 扩展，OpenBSD 的 find 没有）：只看有没有条目
            has_seed=$(ls -A "$BUILTIN" 2>/dev/null | head -1)
            [ -n "$has_seed" ] && mv "$BUILTIN" "$DEST/repos/.seed-zap-appstore" 2>/dev/null || true
            mv "$DEST/repos/.tmp-zap-appstore" "$BUILTIN"
            rm -rf "$DEST/repos/.seed-zap-appstore" 2>/dev/null || true
            ok "AppStore 官方仓库同步完成"
        else
            rm -rf "$DEST/repos/.tmp-zap-appstore" 2>/dev/null || true
            warn "无法克隆 AppStore 仓库（网络不可达？），已保留内置种子包，可在面板中重试更新"
        fi
    fi
    chmod -R 755 "$DEST" 2>/dev/null || true
}

# ── 部署程序 ────────────────────────────────────────────────
TARGET="/usr/local"
ZAP_DIR="$TARGET/zap"

# 安装目录必须先显式创建：不能依赖 cp 隐式创建（包内布局变化或 /usr/local 缺失时
# 会导致 /usr/local/zap 根本没建出来），同时避免"目录在但内容不全"被误判为升级。
mkdir -p "$ZAP_DIR" "$ZAP_DIR/data" || die "无法创建安装目录 ${ZAP_DIR}"

# 升级判定看二进制是否存在，而不是目录是否存在
if [ -x "$ZAP_DIR/zapd" ]; then
    info "检测到已安装版本，执行升级..."
else
    info "部署程序到 ${ZAP_DIR} ..."
fi

# 安装 / 升级共用同一段逻辑（幂等）：二进制 + 脚本 + 权限 + /usr/local/bin 软链
for bin in zapd zapctl zapexec zapupgrade; do
    [ -f "$SRC/$bin" ] || die "安装包缺少 ${bin}（查找目录: ${SRC}）"
    cp -f "$SRC/$bin" "$ZAP_DIR/$bin" || die "部署 ${bin} 失败"
    chmod 0755 "$ZAP_DIR/$bin"
    ln -sf "$ZAP_DIR/$bin" "/usr/local/bin/$bin"
done
# scripts 是必需资源（systemd 服务文件等）：缺失要显式报错，不能静默继续
if [ -d "$SRC/scripts" ]; then
    cp -Rf "$SRC/scripts" "$ZAP_DIR/" || die "部署 scripts 失败"
else
    warn "安装包未包含 scripts 目录（查找目录: ${SRC}），systemd 服务文件将缺失"
fi
# 部署 AppStore（升级不覆盖 git/.git 与 custom/）
deploy_appstore
ok "程序部署完成"

# ── 配置与凭据目录（/etc/zap）───────────────────────────────
info "准备配置目录 /etc/zap ..."
mkdir -p /etc/zap
# zapd 以 zapadm 身份运行（见 zapd.service 的 User=），这里把 /etc/zap 交给 zapadm：
# 首次启动要在此生成自签证书（zap.crt / zap.key）与面板自身的 secret.key。
# 面板用户的 SSH 密钥存于各自家目录 ~/.ssh（zap_ 前缀），由 zapexec(root) 读写。
chown zapadm:zapadm /etc/zap
chmod 0750 /etc/zap

if [ ! -f /etc/zap/zap.yaml ]; then
    info "生成默认配置 /etc/zap/zap.yaml"
    cat > /etc/zap/zap.yaml <<'EOF'
server:
  address: 0.0.0.0
  port: 2600
  cert_file: /etc/zap/zap.crt
  key_file: /etc/zap/zap.key
  url_prefix: ""
jwt:
  jwt_secure: secure-key-zap-default
  jwt_expire: 3600
exec:
  socket_path: /run/zap/exec.sock
  secret_path: /etc/zap/exec.key
db:
  path: /usr/local/zap/data/zap.db
EOF
fi
chown root:zapadm /etc/zap/zap.yaml
chmod 0660 /etc/zap/zap.yaml

# ── 站点配置目录（由 zapexec/root 写入，zapd 只读）──────────
# 与 webserver 安装位置（/usr/local/apps/...）解耦：
#   sites-available 存放实际配置，sites-enabled 用软链启用/停用站点。
#   nginx.conf / httpd.conf 首次同步时由 zapexec 幂等注入 include。
mkdir -p /etc/zap/webservers/nginx/sites-available /etc/zap/webservers/nginx/sites-enabled \
         /etc/zap/webservers/apache/sites-available /etc/zap/webservers/apache/sites-enabled
chown root:zapadm /etc/zap/webservers \
    /etc/zap/webservers/nginx /etc/zap/webservers/nginx/sites-available \
    /etc/zap/webservers/nginx/sites-enabled \
    /etc/zap/webservers/apache /etc/zap/webservers/apache/sites-available \
    /etc/zap/webservers/apache/sites-enabled
chmod 0750 /etc/zap/webservers /etc/zap/webservers/nginx /etc/zap/webservers/apache \
    /etc/zap/webservers/nginx/sites-available /etc/zap/webservers/nginx/sites-enabled \
    /etc/zap/webservers/apache/sites-available /etc/zap/webservers/apache/sites-enabled
ok "站点配置目录已就绪（/etc/zap/webservers）"

# ── 运行时目录权限（zapd 以 zapadm 运行）────────────────────
# 面板数据区：zap.db（sqlite 还会写 -wal/-shm）、AppStore、升级包目录都必须可写；
# 证书改为 zapd 首次启动自行生成，故安装脚本只负责把目录/文件归属准备好。
info "设置运行目录权限（zapadm）..."
mkdir -p "$ZAP_DIR/data/appstore" "$ZAP_DIR/data/apps" "$ZAP_DIR/data/upgrade" "$ZAP_DIR/data/users"
chown zapadm:zapadm "$ZAP_DIR/data" \
    "$ZAP_DIR/data/appstore" "$ZAP_DIR/data/apps" "$ZAP_DIR/data/upgrade" "$ZAP_DIR/data/users"
# 用户私有目录（crontab.yaml / cloud / scripts）由面板进程直接读写：
# 只放开 `users/<user>` 这一层，站点应用数据（webapps/<name>/<site_id>）仍归站点账号
for d in "$ZAP_DIR"/data/users/*/; do
    [ -d "$d" ] && chown zapadm:zapadm "$d" 2>/dev/null || true
done
# 老版本以 root 跑过的话，库文件与 WAL 也需要一并改属，否则 sqlite 无法写入
for f in zap.db zap.db-wal zap.db-shm; do
    [ -e "$ZAP_DIR/data/$f" ] && chown zapadm:zapadm "$ZAP_DIR/data/$f" || true
done
# AppStore 仓库/缓存由面板拉取与写入
[ -d "$ZAP_DIR/data/appstore" ] && chown -R zapadm:zapadm "$ZAP_DIR/data/appstore" 2>/dev/null || true
ok "配置准备完成"

# ── 初始管理员的 Linux 账号 / 家目录 ────────────────────────
# 账号与家目录先建好（Linux）；建库 + 写入管理员交给下面的 `zapd --init-admin`，
# 凭据只经命令行参数传递，不落任何文件（/etc/zap 下也不会留明文密码）。
create_admin_account "$ADMIN_USER" "/home/${ADMIN_USER}"

# ── 服务安装（systemd / rc.d）────────────────────────────────
info "安装 ${INIT} 服务..."

# 用法：install_service <服务名>
# 动作语义一致（enable / 启动），差别在单元文件放哪、以及每个平台用什么命令：
#   systemd → systemctl；FreeBSD → sysrc + service；OpenBSD → rcctl
install_service() {
    local name="$1"
    case "$INIT" in
        systemd)
            cp -Rf "$SRC/scripts/systemd/${name}.service" /etc/systemd/system/ \
                || die "安装 ${name}.service 失败"
            systemctl daemon-reload
            systemctl enable "${name}.service" >/dev/null 2>&1 || warn "服务 ${name} enable 失败"
            systemctl restart "${name}.service" || warn "${name} 启动失败"
            ;;
        rcd)
            cp -Rf "$SRC/scripts/rc.d/${OS_PKG}/${name}" "${RCD_DIR}/${name}" \
                || die "安装 rc.d/${name} 失败"
            chmod 0755 "${RCD_DIR}/${name}"
            # 开机自启：FreeBSD 要写 rc.conf 里的 <n>_enable，不归 service 管
            case "$OS" in
                FreeBSD)
                    sysrc "${name}_enable=YES" || warn "服务 ${name} enable 失败"
                    # 必须带 one 前缀：rc.conf 没开启时 service 会直接拒绝执行
                    service "$name" onestart || warn "${name} 启动失败"
                    ;;
                *)
                    rcctl enable "$name" || warn "服务 ${name} enable 失败"
                    rcctl restart "$name" || warn "${name} 启动失败"
                    ;;
            esac
            ;;
    esac
}

# 顺序有意义：全新机器上 exec.key 由 zapexec 首启生成，zapd 随后才能读到
# （systemd 侧对应 zapd.service 的 After=zapexec.service）
install_service zapexec

# 建库 + 写入初始管理员：必须在 zapd 首次启动前做，否则 zapd 自己建库时会用
# 内置的 admin / 123456。`--init-admin` 只走命令行参数，不写任何凭据文件。
# 注意：密码会在进程命令行上短暂可见（仅安装瞬间，且安装本身已是 root 操作）；
# 想避免可改用 zapd 自动生成（不带 --admin-password，密码打印在输出里）。
init_admin_account() {
    info "初始化管理员 ${ADMIN_USER} ..."
    local out
    if ! out=$(ZAP_CONFIG=/etc/zap/zap.yaml "$ZAP_DIR/zapd" --init-admin "$ADMIN_USER" \
            --admin-password "$ADMIN_PASS" 2>&1); then
        warn "初始化管理员失败（可稍后手动执行 zapd --init-admin 重试）：${out}"
        return 1
    fi
    echo "$out"
    # init-admin 以 root 运行，库文件要交还给 zapadm（zapd 服务以此身份读写）
    for f in zap.db zap.db-wal zap.db-shm; do
        [ -e "$ZAP_DIR/data/$f" ] && chown zapadm:zapadm "$ZAP_DIR/data/$f" || true
    done
    # 库里已有管理员时 init-admin 不会改其密码，完成页据此调整提示
    case "$out" in
        *"未做修改"*) ADMIN_UNCHANGED=1 ;;
    esac
    return 0
}
init_admin_account || true

install_service zapd
ok "${INIT} 服务已启用"

# ── Cleanup ────────────────────────────────────────────
rm -f "$ZAP_FILENAME"
case "$INIT" in
    systemd) systemctl status zapd.service --no-pager || true ;;
    rcd)
        case "$OS" in
            FreeBSD) service zapd onestatus || true ;;
            *)       rcctl check zapd || true ;;
        esac
        ;;
esac
# ── 完成总结 ────────────────────────────────────────────────
printf "\n"
printf "${GREEN}========================================${NC}\n"
printf "${GREEN}           ZAP Installation Complete${NC}\n"
printf "${GREEN}========================================${NC}\n"
echo "  Version:      ${VERSION}"
echo "  Program Directory:  /usr/local/zap"
echo "  Configuration Directory:  /etc/zap"
echo "  Access URL:  https://<Server IP>:2600"
if [ "$ADMIN_UNCHANGED" = "1" ]; then
    echo "  Admin User:      ${ADMIN_USER}（已存在，密码未改动）"
    echo "  Home Directory:  /home/${ADMIN_USER}"
    printf "\n"
    printf "${YELLOW}  ⚠ 检测到库里已有管理员：本次未改动其密码，请用原密码登录${NC}\n"
    printf "\n"
else
    echo "  Admin User:      ${ADMIN_USER}"
    echo "  Admin Password:  ${ADMIN_PASS}"
    echo "  Home Directory:  /home/${ADMIN_USER}"
    printf "\n"
    if [ "$PASS_GENERATED" = "1" ]; then
        printf "${YELLOW}  ⚠ 以上密码为随机生成，请立即保存（不会再次显示）${NC}\n"
        printf "\n"
    fi
    printf "${YELLOW}  ⚠ 首次登录后请立即修改密码！${NC}\n"
    printf "\n"
fi
printf "  后续升级:  zapupgrade upgrade --to latest（回滚: zapupgrade rollback --list）\n"
