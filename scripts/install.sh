#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Zap One-Click Installer
if [ -z "${BASH_VERSION:-}" ]; then
    exec bash "$0" "$@"
fi
set -euo pipefail

# ── 终端颜色 ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'

info() { printf "${BLUE}[*]${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*"; }
die()  { printf "${RED}[✗]${NC} %s\n" "$*" >&2; exit 1; }

detect_lang() {
    local loc="${LANG_MODE:-${LC_ALL:-${LANG:-}}}"

    case "$loc" in
        zh_CN*|zh_TW*|zh_HK*|zh_SG*|zh) echo "zh" ;;
        en_US*|en_GB*|en)               echo "en" ;;
        C|C.UTF-8|POSIX|"")             echo "zh" ;;
        *)                              echo "en" ;; 
    esac
}

LANG_MODE="$(detect_lang)"

if [ "$LANG_MODE" = "zh" ]; then
    L_MUST_ROOT="请以 root 身份运行: sudo bash $0"
    L_FIND_BASH_FAIL="未找到 bash：请先安装后重试"
    L_EDITION="发行版: %s"
    L_TARGET="目标版本: %s   发行版: %s   系统: %s (systemd)   架构: %s"
    L_NO_BASH="未找到 bash：AppStore 安装脚本与计划任务将无法执行，请先安装 bash"
    L_QUERY_LATEST="查询最新版本..."
    L_LATEST_VER="最新版本: %s"
    L_QUERY_LATEST_FAIL="无法查询最新版本，使用 latest 标签"
    L_OFFLINE_NO_QUERY="离线模式：不查询最新版本，请显式指定版本号"
    L_OFFLINE_USE="离线安装：使用本地安装包 %s（版本 %s）"
    L_USE_EXISTING="使用已存在的安装包 %s"
    L_DOWNLOAD="下载 %s ..."
    L_OFFLINE_NO_PKG="离线模式且当前目录没有安装包 %s：请先用 --pkg 指定本地包"
    L_NO_WGET="未找到 wget，且当前目录没有安装包 %s：请先用 --pkg 指定本地包"
    L_DOWNLOAD_FAIL="下载失败，请检查网络或版本号"
    L_PKG_NOT_EXIST="指定的安装包不存在: %s"
    L_PKG_EMPTY="指定的安装包为空: %s"
    L_PKG_REQUIRED="--pkg 缺少安装包路径"
    L_ADMIN_USER_REQUIRED="--admin-user 缺少用户名"
    L_ADMIN_PASS_REQUIRED="--admin-pass 缺少密码"
    L_PKG_NAME_BAD="包名不是 zap-v<版本>[-pro]-%s-%s.tar.gz，按 %s 继续（仅影响完成页显示）"
    L_DECOMP="解压安装包..."
    L_TMP_FAIL="无法创建临时目录"
    L_EXTRACT_FAIL="解压失败，安装包可能已损坏"
    L_PKG_FORMAT="安装包格式不正确：缺少 zap/ 目录"
    L_PKG_CONTENT="安装包内容目录: %s"
    L_OFFLINE_SKIP="离线模式：跳过 AppStore 仓库克隆，沿用发行包内置种子包"
    L_INIT_APPSTORE="初始化 AppStore 官方仓库..."
    L_APPSTORE_DONE="AppStore 官方仓库同步完成"
    L_APPSTORE_FAIL="无法克隆 AppStore 仓库（网络不可达？），已保留内置种子包，可在面板中重试更新"
    L_WWW_MISSING="安装包未包含 data/www，面板「文档」菜单将不可用"
    L_DOC_DEPLOYED="文档已部署（%d 个 md → %s）"
    L_WWW_DOC_MISSING="安装包未包含文档 md，面板「文档」菜单将不可用"
    L_DEPLOY_PROGRAM="部署程序到 %s ..."
    L_UPGRADE_DETECTED="检测到已安装版本，执行升级..."
    L_PKG_MISSING_BIN="安装包缺少 %s（查找目录: %s）"
    L_DEPLOY_BIN_FAIL="部署 %s 失败"
    L_SCRIPTS_MISSING="安装包未包含 scripts 目录（查找目录: %s），systemd 服务文件将缺失"
    L_DEPLOY_DONE="程序部署完成"
    L_CFG_DIR="准备配置目录 /etc/zap ..."
    L_GEN_CFG="生成默认配置 /etc/zap/zap.yaml"
    L_WEBSERVER_DIR="站点配置目录已就绪（/etc/zap/webservers）"
    L_RUNTIME_PERM="设置运行目录权限（zapadm）..."
    L_CFG_DONE="配置准备完成"
    L_INSTALL_SERVICE="安装 systemd 服务..."
    L_SERVICE_INSTALL_FAIL="安装 %s.service 失败"
    L_SERVICE_ENABLE_FAIL="%s 服务 enable 失败"
    L_SERVICE_START_FAIL="%s 启动失败"
    L_INIT_ADMIN="初始化管理员 %s ..."
    L_INIT_ADMIN_FAIL="初始化管理员失败（可稍后手动执行 zapd --init-admin 重试）：%s"
    L_SERVICE_ENABLED="systemd 服务已启用"
    L_USER_EXISTS="用户 %s 已存在"
    L_CREATE_USER="创建用户 %s"
    L_CREATE_USER_FAIL="创建 %s 用户失败"
    L_NO_USERADD="未找到 useradd / adduser，无法创建 %s 用户"
    L_LINUX_ACCT_EXISTS="Linux 账号 %s 已存在"
    L_LINUX_ACCT_CREATED="Linux 账号 %s 已创建（%s，nologin）"
    L_HOME_READY="家目录已就绪: %s（www/logs/tmp）"
    L_GROUP_FAIL="创建组 %s 失败（继续）"
    L_USER_CREATED="用户 %s 创建完成"
    L_HOME_SKEL_FAIL="创建家目录骨架失败: %s"
    L_MKDIR_FAIL="无法创建安装目录 %s"
    L_DEPLOY_SCRIPTS_FAIL="部署 scripts 失败"
    L_WARN_EXISTING_ADMIN="⚠ 检测到库里已有管理员：本次未改动其密码，请用原密码登录"
    L_WARN_RANDOM_PASS="⚠ 以上密码为随机生成，请立即保存（不会再次显示）"
    L_WARN_CHANGE_PASS="⚠ 首次登录后请立即修改密码！"
    L_WARN_OFFLINE="⚠ 离线安装：AppStore 用内置种子包，升级请用离线升级入口"
    L_OFFLINE_UPGRADE_NOTE="（与 install-offline.sh 同目录；会自动备份，失败可 zapupgrade rollback）"
    L_UNKNOWN_ARG="未知参数: %s（--help 查看用法）"
    L_ADMIN_EMPTY="管理员用户名不能为空"
    L_ADMIN_TOO_LONG="管理员用户名过长（≤32 字符）: %s"
    L_ADMIN_START="管理员用户名需以小写字母或 _ 开头: %s"
    L_ADMIN_CHARS="管理员用户名只能包含 a-z 0-9 _ -: %s"
    L_PASS_CHARS="管理员密码只能包含字母、数字以及 . _ -（避免破坏 env 文件解析）"
    L_PASS_SHORT="管理员密码不足 8 位，建议登录后修改"
    L_NO_SYSTEMCTL="未检测到 systemctl：目前仅支持 systemd 作为服务管理器"
    L_OS_UNSUPPORTED="不支持的操作系统: %s（当前仅支持 Linux）"
    L_ARCH_UNSUPPORTED="不支持的架构: %s"
    L_DOWNLOAD_EMPTY="下载内容为空: %s"
else
    L_MUST_ROOT="Please run as root: sudo bash $0"
    L_FIND_BASH_FAIL="bash not found: please install it and retry"
    L_EDITION="Edition: %s"
    L_TARGET="Target version: %s   Edition: %s   OS: %s (systemd)   Arch: %s"
    L_NO_BASH="bash not found: AppStore install scripts and cron jobs will fail; please install bash"
    L_QUERY_LATEST="Querying latest version..."
    L_LATEST_VER="Latest version: %s"
    L_QUERY_LATEST_FAIL="Cannot query latest version, using 'latest' tag"
    L_OFFLINE_NO_QUERY="Offline mode: not querying latest version, please specify a version explicitly"
    L_OFFLINE_USE="Offline install: using local package %s (version %s)"
    L_USE_EXISTING="Using existing package %s"
    L_DOWNLOAD="Downloading %s ..."
    L_OFFLINE_NO_PKG="Offline mode and no package in current dir %s: please specify a local package with --pkg"
    L_NO_WGET="wget not found and no package in current dir %s: please specify a local package with --pkg"
    L_DOWNLOAD_FAIL="Download failed, please check network or version"
    L_PKG_NOT_EXIST="Specified package does not exist: %s"
    L_PKG_EMPTY="Specified package is empty: %s"
    L_PKG_REQUIRED="--pkg requires package path"
    L_ADMIN_USER_REQUIRED="--admin-user requires username"
    L_ADMIN_PASS_REQUIRED="--admin-pass requires password"
    L_PKG_NAME_BAD="Package name is not zap-v<version>[-pro]-%s-%s.tar.gz, continuing with %s (affects completion page only)"
    L_DECOMP="Extracting package..."
    L_TMP_FAIL="Cannot create temp directory"
    L_EXTRACT_FAIL="Extraction failed, the package may be corrupted"
    L_PKG_FORMAT="Invalid package layout: missing zap/ directory"
    L_PKG_CONTENT="Package content dir: %s"
    L_OFFLINE_SKIP="Offline mode: skipping AppStore repo clone, using bundled seed package"
    L_INIT_APPSTORE="Initializing AppStore official repo..."
    L_APPSTORE_DONE="AppStore official repo synced"
    L_APPSTORE_FAIL="Cannot clone AppStore repo (network unreachable?); kept bundled seed package, retry from panel later"
    L_WWW_MISSING="Package does not contain data/www; the panel 'Docs' menu will be unavailable"
    L_DOC_DEPLOYED="Documents deployed (%d md → %s)"
    L_WWW_DOC_MISSING="Package does not contain doc md; the panel 'Docs' menu will be unavailable"
    L_DEPLOY_PROGRAM="Deploying program to %s ..."
    L_UPGRADE_DETECTED="Existing installation detected, upgrading..."
    L_PKG_MISSING_BIN="Package missing %s (lookup dir: %s)"
    L_DEPLOY_BIN_FAIL="Failed to deploy %s"
    L_SCRIPTS_MISSING="Package does not contain scripts dir (lookup dir: %s); systemd unit files will be missing"
    L_DEPLOY_DONE="Program deployed"
    L_CFG_DIR="Preparing config dir /etc/zap ..."
    L_GEN_CFG="Generating default config /etc/zap/zap.yaml"
    L_WEBSERVER_DIR="Web server config dir ready (/etc/zap/webservers)"
    L_RUNTIME_PERM="Setting runtime dir permissions (zapadm)..."
    L_CFG_DONE="Config prepared"
    L_INSTALL_SERVICE="Installing systemd service..."
    L_SERVICE_INSTALL_FAIL="Failed to install %s.service"
    L_SERVICE_ENABLE_FAIL="%s service enable failed"
    L_SERVICE_START_FAIL="%s start failed"
    L_INIT_ADMIN="Initializing admin %s ..."
    L_INIT_ADMIN_FAIL="Failed to initialize admin (retry later with: zapd --init-admin): %s"
    L_SERVICE_ENABLED="systemd services enabled"
    L_USER_EXISTS="User %s already exists"
    L_CREATE_USER="Creating user %s"
    L_CREATE_USER_FAIL="Failed to create user %s"
    L_NO_USERADD="useradd / adduser not found, cannot create user %s"
    L_LINUX_ACCT_EXISTS="Linux account %s already exists"
    L_LINUX_ACCT_CREATED="Linux account %s created (%s, nologin)"
    L_HOME_READY="Home dir ready: %s (www/logs/tmp)"
    L_GROUP_FAIL="Failed to create group %s (continuing)"
    L_USER_CREATED="User %s created"
    L_HOME_SKEL_FAIL="Failed to create home skeleton: %s"
    L_MKDIR_FAIL="Cannot create install dir %s"
    L_DEPLOY_SCRIPTS_FAIL="Failed to deploy scripts"
    L_WARN_EXISTING_ADMIN="⚠ An admin already exists in the database: password was NOT changed this time, please log in with the original password"
    L_WARN_RANDOM_PASS="⚠ The password above was randomly generated, save it now (it will not be shown again)"
    L_WARN_CHANGE_PASS="⚠ Change your password immediately after first login!"
    L_WARN_OFFLINE="⚠ Offline install: AppStore uses the bundled seed package; upgrade via the offline upgrade entry"
    L_OFFLINE_UPGRADE_NOTE="(same dir as install-offline.sh; auto-backup, rollback with zapupgrade rollback on failure)"
    L_UNKNOWN_ARG="Unknown argument: %s (use --help)"
    L_ADMIN_EMPTY="Admin username cannot be empty"
    L_ADMIN_TOO_LONG="Admin username too long (≤32 chars): %s"
    L_ADMIN_START="Admin username must start with a-z or _: %s"
    L_ADMIN_CHARS="Admin username may only contain a-z 0-9 _ -: %s"
    L_PASS_CHARS="Admin password may only contain letters, digits and . _ - (to avoid breaking env-file parsing)"
    L_PASS_SHORT="Admin password is shorter than 8 chars; change it after first login"
    L_NO_SYSTEMCTL="systemctl not found: only systemd is supported as service manager"
    L_OS_UNSUPPORTED="Unsupported OS: %s (only Linux supported)"
    L_ARCH_UNSUPPORTED="Unsupported architecture: %s"
    L_DOWNLOAD_EMPTY="Downloaded content is empty: %s"
fi

usage() {
    if [ "$LANG_MODE" = "zh" ]; then
        cat <<'EOF'
用法: sudo bash install.sh [VERSION] [OPTIONS]

  VERSION                要安装的版本号（默认 latest）

离线安装（内网 / 无外网机器）：
  --pkg <路径>           使用**本地已有的**安装包，不联网下载（版本号从文件名解析）
  --offline              全程不访问外网：不查 latest、不下载、不克隆 AppStore 仓库
                         （AppStore 用发行包内置的种子包，面板里可随时重试更新）

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
  sudo bash install.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz --offline   # 内网离线安装
EOF
    else
        cat <<'EOF'
Usage: sudo bash install.sh [VERSION] [OPTIONS]

  VERSION                Version to install (default: latest)

Offline install (intranet / no internet):
  --pkg <path>           Use an existing local package, no download (version parsed from filename)
  --offline              Fully offline: no latest check, no download, no AppStore clone
                         (AppStore uses the bundled seed package; retry update from panel later)

Initial admin credentials (only on first install with a fresh DB):
  --admin-user <name>    Admin username (default: admin); also the Linux account, home at /home/<name>
  --admin-pass <pass>    Admin password; if omitted a random one is generated and printed
                         Allowed chars: letters, digits, . _ - (avoid breaking env-file parsing)
  Env vars               ZAP_ADMIN_USER / ZAP_ADMIN_PASSWORD (CLI args take precedence)

Notes: at the end `zapd --init-admin` creates the DB and writes the admin (credentials passed only via CLI,
never written to disk); re-running on an installed machine won't change the existing admin password.

  -h, --help             Show this help

Examples:
  sudo bash install.sh latest --admin-user zapops --admin-pass 'S3cret-Pass'
  sudo ZAP_ADMIN_PASSWORD='S3cret-Pass' bash install.sh
  sudo bash install.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz --offline   # intranet offline install
EOF
    fi
}

# rand 16 chars from /dev/urandom, base64, strip = + / and newlines
gen_password() {
    local raw
    raw=$(head -c 12 /dev/urandom 2>/dev/null | base64 2>/dev/null | tr -d '\n=+/') || raw=""
    [ -n "$raw" ] || raw=$(od -An -N12 -tx1 /dev/urandom 2>/dev/null | tr -d ' \n') || raw=""
    [ -n "$raw" ] || raw="zap$(date +%s)"
    printf '%s' "${raw:0:16}"
}

zap_install_main() {
[ "$(id -u)" -eq 0 ] || die "$L_MUST_ROOT"

printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   Zap One-Click Installer ${NC}\n"
printf "${GREEN}========================================${NC}\n"

VERSION="latest"
ADMIN_USER=""; ADMIN_PASS=""; PASS_GENERATED=0; ADMIN_UNCHANGED=0
LOCAL_PKG=""; OFFLINE=0
while [ $# -gt 0 ]; do
    case "$1" in
        --pkg)
            LOCAL_PKG="${2:-}"
            [ -n "$LOCAL_PKG" ] || die "$L_PKG_REQUIRED"
            shift 2 ;;
        --pkg=*)
            LOCAL_PKG="${1#*=}"
            [ -n "$LOCAL_PKG" ] || die "$L_PKG_REQUIRED"
            shift ;;
        --offline)
            OFFLINE=1; shift ;;
        --admin-user)
            ADMIN_USER="${2:-}"
            [ -n "$ADMIN_USER" ] || die "$L_ADMIN_USER_REQUIRED"
            shift 2 ;;
        --admin-user=*)
            ADMIN_USER="${1#*=}"
            [ -n "$ADMIN_USER" ] || die "$L_ADMIN_USER_REQUIRED"
            shift ;;
        --admin-pass|--admin-password)
            ADMIN_PASS="${2:-}"
            [ -n "$ADMIN_PASS" ] || die "$L_ADMIN_PASS_REQUIRED"
            shift 2 ;;
        --admin-pass=*|--admin-password=*)
            ADMIN_PASS="${1#*=}"
            [ -n "$ADMIN_PASS" ] || die "$L_ADMIN_PASS_REQUIRED"
            shift ;;
        -h|--help)
            usage; exit 0 ;;
        -*)
            die "$(printf "$L_UNKNOWN_ARG" "$1")" ;;
        *)
            VERSION="$1"; shift ;;
    esac
done

[ -n "$ADMIN_USER" ] || ADMIN_USER="${ZAP_ADMIN_USER:-admin}"
[ -n "$ADMIN_PASS" ] || ADMIN_PASS="${ZAP_ADMIN_PASSWORD:-}"

ADMIN_USER=$(printf '%s' "$ADMIN_USER" | tr 'A-Z' 'a-z')
[ -n "$ADMIN_USER" ] || die "$L_ADMIN_EMPTY"
[ "${#ADMIN_USER}" -le 32 ] || die "$(printf "$L_ADMIN_TOO_LONG" "$ADMIN_USER")"
case "$ADMIN_USER" in
    [a-z_]*) ;;
    *) die "$(printf "$L_ADMIN_START" "$ADMIN_USER")" ;;
esac
case "$ADMIN_USER" in
    *[!a-z0-9_-]*) die "$(printf "$L_ADMIN_CHARS" "$ADMIN_USER")" ;;
esac

if [ -z "$ADMIN_PASS" ]; then
    ADMIN_PASS=$(gen_password)
    PASS_GENERATED=1
fi
case "$ADMIN_PASS" in
    *[!A-Za-z0-9._-]*) die "$L_PASS_CHARS" ;;
esac
[ "${#ADMIN_PASS}" -ge 8 ] || warn "$L_PASS_SHORT"

EDITION="${EDITION:-Zap Community}"
EDITION_ID="${EDITION_ID:-community}"
PRO_SUFFIX="${PRO_SUFFIX:-}"
info "$(printf "$L_EDITION" "$EDITION")"

OS=$(uname -s)
case "$OS" in
    Linux)
        command -v systemctl >/dev/null 2>&1 \
            || die "$L_NO_SYSTEMCTL"
        OS_PKG=linux
        ;;
    *)
        die "$(printf "$L_OS_UNSUPPORTED" "$OS")"
        ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)              ARCH="amd64" ;;
    aarch64|arm64|arm*)  ARCH="arm64" ;;
    ppc64le)             ;;
    s390x)               ;;
    *) die "$(printf "$L_ARCH_UNSUPPORTED" "$ARCH")" ;;
esac
info "$(printf "$L_TARGET" "$VERSION" "$EDITION" "$OS" "$ARCH")"

HAVE_FETCH=0
if command -v wget >/dev/null 2>&1; then
    fetch_url()  { wget -q -O - "$1"; }
    fetch_file() { wget -O "$2" "$1"; }
    HAVE_FETCH=1
fi

DOWNLOAD_ZAP_URL="https://mirrors.zap.cn/zap/releases"
NEED_DOWNLOAD=1
if [ -n "$LOCAL_PKG" ]; then
    NEED_DOWNLOAD=0
fi
if [ "$VERSION" = "latest" ] && [ "$NEED_DOWNLOAD" = "1" ]; then
    if [ "$HAVE_FETCH" = "1" ] && [ "$OFFLINE" != "1" ]; then
        info "$L_QUERY_LATEST"
        if LATEST=$(fetch_url "${DOWNLOAD_ZAP_URL}/latest.txt?t=$(date +%s)") && [ -n "$LATEST" ]; then
            VERSION="$LATEST"
            info "$(printf "$L_LATEST_VER" "$VERSION")"
        else
            warn "$L_QUERY_LATEST_FAIL"
        fi
    else
        warn "$L_OFFLINE_NO_QUERY"
    fi
fi

ZAP_FILENAME="zap-v${VERSION}${PRO_SUFFIX}-${OS_PKG}-${ARCH}.tar.gz"

# ── 取包：本地指定 > 当前目录已存在 > 下载 ────────────────────
DOWNLOADED=0
if [ -n "$LOCAL_PKG" ]; then
    [ -f "$LOCAL_PKG" ] || die "$(printf "$L_PKG_NOT_EXIST" "$LOCAL_PKG")"
    [ -s "$LOCAL_PKG" ] || die "$(printf "$L_PKG_EMPTY" "$LOCAL_PKG")"
    # 版本号从文件名解析（zap-v<版本>[-pro]-<os>-<arch>.tar.gz），装完的总结页要用
    local_base=$(basename "$LOCAL_PKG")
    if [[ "$local_base" =~ ^zap-v(.+)(-pro)?-${OS_PKG}-${ARCH}\.tar\.gz$ ]]; then
        VERSION="${BASH_REMATCH[1]}"
    else
        warn "$(printf "$L_PKG_NAME_BAD" "$OS_PKG" "$ARCH" "$VERSION")"
    fi
    ZAP_FILENAME="$LOCAL_PKG"
    info "$(printf "$L_OFFLINE_USE" "$LOCAL_PKG" "$VERSION")"
elif [ -f "$ZAP_FILENAME" ]; then
    info "$(printf "$L_USE_EXISTING" "$ZAP_FILENAME")"
else
    # 只有真要下载这一步，才要求机器上有下载工具且允许连外网
    [ "$OFFLINE" != "1" ] || die "$(printf "$L_OFFLINE_NO_PKG" "$ZAP_FILENAME")"
    [ "$HAVE_FETCH" = "1" ] || die "$(printf "$L_NO_WGET" "$ZAP_FILENAME")"
    info "$(printf "$L_DOWNLOAD" "$ZAP_FILENAME")"
    # fetch_file 需要两个参数：URL + 落盘路径
    # 先写 .part 再改名：中途失败不会留下半截包，被下次运行当成完整包解压
    rm -f "${ZAP_FILENAME}.part"
    fetch_file "${DOWNLOAD_ZAP_URL}/${ZAP_FILENAME}" "${ZAP_FILENAME}.part" \
        || die "$L_DOWNLOAD_FAIL"
    [ -s "${ZAP_FILENAME}.part" ] || die "$(printf "$L_DOWNLOAD_EMPTY" "${DOWNLOAD_ZAP_URL}/${ZAP_FILENAME}")"
    mv -f "${ZAP_FILENAME}.part" "${ZAP_FILENAME}"
    DOWNLOADED=1
fi

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
        ok "$(printf "$L_USER_EXISTS" "$user")"
        return 0
    fi

    info "$(printf "$L_CREATE_USER" "$user")"
    local group_exists=0
    if has_group "$user"; then
        group_exists=1
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
        useradd "${opts[@]}" "$user" || die "$(printf "$L_CREATE_USER_FAIL" "$user")"
    elif command -v adduser >/dev/null 2>&1; then
        local -a opts=(--shell /bin/false --no-create-home --disabled-password --disabled-login)
        [ "$is_system" = "1" ] && opts+=(--system)
        if [ "$group_exists" = "1" ]; then
            opts+=(--ingroup "$user")
        else
            opts+=(--group)
        fi
        adduser "${opts[@]}" "$user" || die "$(printf "$L_CREATE_USER_FAIL" "$user")"
    else
        die "$(printf "$L_NO_USERADD" "$user")"
    fi
    ok "$(printf "$L_USER_CREATED" "$user")"
}


# zapd::zap::admin_bootstrap
# create_admin_account <username> <homedir>
create_admin_account() {
    local user="$1" home="$2"
    local gname="$user"
    if has_group "$user"; then
        gname="zap_${user}"
        if ! has_group "$gname" && command -v groupadd >/dev/null 2>&1; then
            groupadd "$gname" 2>/dev/null || warn "$(printf "$L_GROUP_FAIL" "$gname")"
        fi
    fi
    if id "$user" >/dev/null 2>&1; then
        ok "$(printf "$L_LINUX_ACCT_EXISTS" "$user")"
    else
        local shell
        shell=$(command -v nologin 2>/dev/null || true)
        [ -n "$shell" ] || shell=/usr/sbin/nologin
        if has_group "$gname"; then
            useradd -m -d "$home" -s "$shell" -g "$gname" "$user" || die "$(printf "$L_CREATE_USER_FAIL" "$user")"
        else
            useradd -m -d "$home" -s "$shell" -U "$user" || die "$(printf "$L_CREATE_USER_FAIL" "$user")"
        fi
        ok "$(printf "$L_LINUX_ACCT_CREATED" "$user" "$home")"
    fi

    mkdir -p "$home/www" "$home/logs" "$home/tmp" || die "$(printf "$L_HOME_SKEL_FAIL" "$home")"
    chown -R "${user}:${gname}" "$home/www" 2>/dev/null || true
    chmod 755 "$home/www"
    chown -R www:www "$home/logs" 2>/dev/null || true
    chmod 770 "$home/logs"
    chown -R "${user}:${gname}" "$home/tmp" 2>/dev/null || true
    chmod 700 "$home/tmp"
    chown "${user}:${gname}" "$home" 2>/dev/null || true
    chmod 711 "$home"
    ok "$(printf "$L_HOME_READY" "$home")"
}

create_user www 0
create_user zapadm 1

info "$L_DECOMP"
WORK_DIR=$(mktemp -d /tmp/zap-install.XXXXXX) || die "$L_TMP_FAIL"
trap 'rm -rf "$WORK_DIR"' EXIT
tar zxf "$ZAP_FILENAME" -C "$WORK_DIR" || die "$L_EXTRACT_FAIL"

SRC="$WORK_DIR/zap"
[ -d "$SRC" ] || die "$L_PKG_FORMAT"
info "$(printf "$L_PKG_CONTENT" "$SRC")"

APPSTORE_REPO_URL="${APPSTORE_REPO_URL:-https://github.com/zapsh/appstore.git}"

deploy_appstore() {
    local DEST="$ZAP_DIR/data/appstore"
    local BUILTIN="$DEST/repos/appstore"
    mkdir -p "$DEST"/{repos,custom,cache,tmp,logs}
    mkdir -p "$ZAP_DIR/data/apps"

    [ -f "$DEST/repos.yaml" ]       || cp -f "$SRC/data/appstore/repos.yaml" "$DEST/repos.yaml" 2>/dev/null || true
    [ -f "$DEST/custom/README.md" ] || cp -f "$SRC/data/appstore/custom/README.md" "$DEST/custom/README.md" 2>/dev/null || true

    if [ ! -d "$BUILTIN/.git" ] && [ ! -d "$BUILTIN/database" ]; then
        mkdir -p "$BUILTIN"
        for c in infra application webapps database library; do
            [ -d "$SRC/data/appstore/repos/appstore/$c" ] && cp -Rf "$SRC/data/appstore/repos/appstore/$c" "$BUILTIN/" 2>/dev/null || true
        done
    fi

    if [ "$OFFLINE" = "1" ]; then
        info "$L_OFFLINE_SKIP"
    elif [ ! -d "$BUILTIN/.git" ] && command -v git >/dev/null 2>&1; then
        info "$L_INIT_APPSTORE"
        if git clone -q --depth 1 "$APPSTORE_REPO_URL" "$DEST/repos/.tmp-appstore" 2>/dev/null; then
            local has_seed
            # 只看有没有条目（不用 find，避免依赖 GNU 扩展）
            has_seed=$(ls -A "$BUILTIN" 2>/dev/null | head -1)
            [ -n "$has_seed" ] && mv "$BUILTIN" "$DEST/repos/.seed-appstore" 2>/dev/null || true
            mv "$DEST/repos/.tmp-appstore" "$BUILTIN"
            rm -rf "$DEST/repos/.seed-appstore" 2>/dev/null || true
            ok "$L_APPSTORE_DONE"
        else
            rm -rf "$DEST/repos/.tmp-appstore" 2>/dev/null || true
            warn "$L_APPSTORE_FAIL"
        fi
    fi
    chmod -R 755 "$DEST" 2>/dev/null || true
}

deploy_www() {
    if [ ! -d "$SRC/data/www" ]; then
        warn "$(printf '%s (lookup dir: %s)' "$L_WWW_MISSING" "$SRC")"
        return 0
    fi
    local DEST="$ZAP_DIR/data/www"
    mkdir -p "$DEST/html"

    local f n=0
    for f in "$SRC"/data/www/html/*.md; do
        [ -f "$f" ] || continue
        cp -f "$f" "$DEST/html/" 2>/dev/null && n=$((n + 1))
    done
    chmod 0644 "$DEST"/html/*.md 2>/dev/null || true
    if [ "$n" -gt 0 ]; then
        ok "$(printf "$L_DOC_DEPLOYED" "$n" "$DEST/html")"
    else
        warn "$(printf '%s (lookup dir: %s)' "$L_WWW_DOC_MISSING" "$SRC/data/www/html")"
    fi

    [ -d "$DEST/skel" ] || cp -Rf "$SRC/data/www/skel" "$DEST/" 2>/dev/null || true
    [ -d "$DEST/_zap" ] || cp -Rf "$SRC/data/www/_zap" "$DEST/" 2>/dev/null || true
    chmod 0755 "$DEST" "$DEST/html" 2>/dev/null || true
}

# Install to 
TARGET="/usr/local"
ZAP_DIR="$TARGET/zap"


mkdir -p "$ZAP_DIR" "$ZAP_DIR/data" || die "$(printf "$L_MKDIR_FAIL" "$ZAP_DIR")"

if [ -x "$ZAP_DIR/zapd" ]; then
    info "$L_UPGRADE_DETECTED"
else
    info "$(printf "$L_DEPLOY_PROGRAM" "$ZAP_DIR")"
fi

for bin in zapd zapctl zapexec zapupgrade; do
    [ -f "$SRC/$bin" ] || die "$(printf "$L_PKG_MISSING_BIN" "$bin" "$SRC")"
    cp -f "$SRC/$bin" "$ZAP_DIR/$bin" || die "$(printf "$L_DEPLOY_BIN_FAIL" "$bin")"
    chmod 0755 "$ZAP_DIR/$bin"
done
ln -sf "$ZAP_DIR/zapctl" "/usr/local/bin/zapctl"
ln -sf "$ZAP_DIR/zapupgrade" "/usr/local/bin/zapupgrade"

if [ -d "$SRC/scripts" ]; then
    cp -Rf "$SRC/scripts" "$ZAP_DIR/" || die "$L_DEPLOY_SCRIPTS_FAIL"
else
    warn "$(printf "$L_SCRIPTS_MISSING" "$SRC")"
fi

deploy_appstore
deploy_www
ok "$L_DEPLOY_DONE"

# configuration dir /etc/zap 
info "$L_CFG_DIR"
mkdir -p /etc/zap

printf '%s\n' "$EDITION_ID" > /etc/zap/edition
chmod 0644 /etc/zap/edition
chown zapadm:zapadm /etc/zap
chmod 0750 /etc/zap

if [ ! -f /etc/zap/zap.yaml ]; then
    info "$L_GEN_CFG"
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

mkdir -p /etc/zap/webservers/nginx/sites-available /etc/zap/webservers/nginx/sites-enabled

chown root:zapadm /etc/zap/webservers \
    /etc/zap/webservers/nginx /etc/zap/webservers/nginx/sites-available \
    /etc/zap/webservers/nginx/sites-enabled

chmod 0750 /etc/zap/webservers /etc/zap/webservers/nginx \
    /etc/zap/webservers/nginx/sites-available /etc/zap/webservers/nginx/sites-enabled

ok "$L_WEBSERVER_DIR"

info "$L_RUNTIME_PERM"
mkdir -p "$ZAP_DIR/data/appstore" "$ZAP_DIR/data/apps" "$ZAP_DIR/data/users" \
    "$ZAP_DIR/data/upgrade/stage" "$ZAP_DIR/data/upgrade/logs" "$ZAP_DIR/data/upgrade/backup"
chown zapadm:zapadm "$ZAP_DIR/data" \
    "$ZAP_DIR/data/appstore" "$ZAP_DIR/data/apps" "$ZAP_DIR/data/users"

chown -R zapadm:zapadm "$ZAP_DIR/data/upgrade" 2>/dev/null || true
chmod 0755 "$ZAP_DIR/data/upgrade" 2>/dev/null || true

for d in "$ZAP_DIR"/data/users/*/; do
    [ -d "$d" ] && chown zapadm:zapadm "$d" 2>/dev/null || true
done
for f in zap.db zap.db-wal zap.db-shm; do
    [ -e "$ZAP_DIR/data/$f" ] && chown zapadm:zapadm "$ZAP_DIR/data/$f" || true
done
[ -d "$ZAP_DIR/data/appstore" ] && chown -R zapadm:zapadm "$ZAP_DIR/data/appstore" 2>/dev/null || true
ok "$L_CFG_DONE"

create_admin_account "$ADMIN_USER" "/home/${ADMIN_USER}"

info "$L_INSTALL_SERVICE"

# install_service <service name>
install_service() {
    local name="$1"
    cp -Rf "$SRC/scripts/systemd/${name}.service" /etc/systemd/system/ \
        || die "$(printf "$L_SERVICE_INSTALL_FAIL" "$name")"
    systemctl daemon-reload
    systemctl enable "${name}.service" >/dev/null 2>&1 || warn "$(printf "$L_SERVICE_ENABLE_FAIL" "$name")"
    systemctl restart "${name}.service" || warn "$(printf "$L_SERVICE_START_FAIL" "$name")"
}

install_service zapexec

init_admin_account() {
    info "$(printf "$L_INIT_ADMIN" "$ADMIN_USER")"
    local out
    if ! out=$(ZAP_CONFIG=/etc/zap/zap.yaml "$ZAP_DIR/zapd" --init-admin "$ADMIN_USER" \
            --admin-password "$ADMIN_PASS" 2>&1); then
        warn "$(printf "$L_INIT_ADMIN_FAIL" "$out")"
        return 1
    fi
    echo "$out"
    for f in zap.db zap.db-wal zap.db-shm; do
        [ -e "$ZAP_DIR/data/$f" ] && chown zapadm:zapadm "$ZAP_DIR/data/$f" || true
    done
    case "$out" in
        *"未做修改"*) ADMIN_UNCHANGED=1 ;;
    esac
    return 0
}
init_admin_account || true

install_service zapd
ok "$L_SERVICE_ENABLED"

if [ "$DOWNLOADED" = "1" ]; then
    rm -f "$ZAP_FILENAME"
fi
systemctl status zapd.service --no-pager || true

printf "\n"
printf "${GREEN}========================================${NC}\n"
printf "${GREEN}           ZAP Installation Complete${NC}\n"
printf "${GREEN}========================================${NC}\n"
echo "  Version:      ${VERSION}"
echo "  Edition:      ${EDITION}"
echo "  Program Directory:  /usr/local/zap"
echo "  Configuration Directory:  /etc/zap"
echo "  Access URL:  https://<Server IP>:2600"
if [ "$ADMIN_UNCHANGED" = "1" ]; then
    echo "  Admin User:      ${ADMIN_USER}(Existing, Password Unchanged)"
    echo "  Home Directory:  /home/${ADMIN_USER}"
    printf "\n"
    printf "${YELLOW}  ${L_WARN_EXISTING_ADMIN}${NC}\n"
    printf "\n"
else
    echo "  Admin User:      ${ADMIN_USER}"
    echo "  Admin Password:  ${ADMIN_PASS}"
    echo "  Home Directory:  /home/${ADMIN_USER}"
    printf "\n"
    if [ "$PASS_GENERATED" = "1" ]; then
        printf "${YELLOW}  ${L_WARN_RANDOM_PASS}${NC}\n"
        printf "\n"
    fi
    printf "${YELLOW}  ${L_WARN_CHANGE_PASS}${NC}\n"
    printf "\n"
fi
if [ "$OFFLINE" = "1" ]; then
    printf "${YELLOW}  ${L_WARN_OFFLINE}${NC}\n"
    printf "     sudo bash upgrade-offline.sh --pkg ./zap-v<版本>${PRO_SUFFIX}-linux-${ARCH}.tar.gz\n"
    printf "     ${L_OFFLINE_UPGRADE_NOTE}\n"
    printf "\n"
else
    UPGRADE_HINT="zapupgrade upgrade --to latest"
    [ "$PRO" = "1" ] && UPGRADE_HINT="${UPGRADE_HINT} --pro"
    printf "  后续升级:  ${UPGRADE_HINT}（回滚: zapupgrade rollback --list）\n"
fi
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    zap_install_main "$@"
fi
