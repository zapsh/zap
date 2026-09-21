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

# ── 解析版本与架构 ─────────────────────────────────────────
VERSION="${1:-latest}"
# ── 平台探测：操作系统 + 服务管理器 ─────────────────────────
# 服务管理器决定后面怎么装/启服务：
#   Linux          → systemd（systemctl）
#   FreeBSD        → rc.d + service(8)，开机自启归 sysrc 管（它没有 rcctl）
#   OpenBSD/NetBSD → rc.d + rcctl
# RCD_DIR 是 rc.d 脚本的落点：FreeBSD 的第三方服务装在 /usr/local/etc/rc.d/，
# 系统自带的才在 /etc/rc.d/；OpenBSD/NetBSD 一律在 /etc/rc.d/。
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
    NetBSD)
        INIT=rcd
        OS_PKG=netbsd
        RCD_DIR=/etc/rc.d
        ;;
    *)
        die "不支持的操作系统: ${OS}（当前支持 Linux、FreeBSD、OpenBSD、NetBSD）"
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
    fetch_file "${DOWNLOAD_ZAP_URL}/${ZAP_FILENAME}" || die "下载失败，请检查网络或版本号"
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

# ── 服务安装（systemd / rc.d）────────────────────────────────
info "安装 ${INIT} 服务..."

# 用法：install_service <服务名>
# 动作语义一致（enable / 启动），差别在单元文件放哪、以及每个平台用什么命令：
#   systemd → systemctl；FreeBSD → sysrc + service；OpenBSD/NetBSD → rcctl
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
echo "  Default Username:  admin"
echo "  Default Password:  123456"
printf "\n"
printf "  后续升级:  zapupgrade upgrade --to latest（回滚: zapupgrade rollback --list）\n"
printf "\n"
printf "${YELLOW}  ⚠ 首次登录后请立即修改默认密码！${NC}\n"
