#!/usr/bin/env bash
# ZAP 离线包制作脚本 —— 在有外网的机器上跑，产出一个可以直接搬到内网的包。
#
# 产物 zap-offline-v<版本>[-pro]-linux-<arch>.tar.gz 解开后是：
#   zap-v<版本>[-pro]-linux-<arch>.tar.gz   发布包本体（与在线安装用的是同一个）
#   install.sh / uninstall.sh               与实际安装一致的那份脚本
#   install-offline.sh                      内网机上的安装入口（自动挑包 + 校验）
#   SHA256SUMS                              传递过程有没有损坏，装之前先验一遍
#   README-offline.txt                      内网安装步骤
#
# 为什么连 install.sh 一起打进去：内网机器上不一定有仓库里的脚本，
# 而「装进去的脚本」与「装上去的二进制」必须同版本，就地取用最省事。
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { printf "${BLUE}[*]${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*"; }
die()  { printf "${RED}[✗]${NC} %s\n" "$*" >&2; exit 1; }

usage() {
    cat <<'EOF'
用法: bash scripts/offline-pack.sh [选项]

在有外网的机器上制作离线安装包，之后拷到内网机器即可安装。

选项:
  --version <版本>   要打包的版本（默认 latest：联网查询最新版本号）
  --pro              打包商业版 Zap Pro（包名带 -pro）
  --arch <arch>      目标架构（默认当前机器；可给 amd64 / arm64，用于交叉投递）
  --pkg <路径>       用本地已有的发布包，不再下载
  --out <目录>       产物目录（默认 ./dist）
  -h, --help         显示本帮助

示例:
  bash scripts/offline-pack.sh                          # 最新社区版 + 本机架构
  bash scripts/offline-pack.sh --pro --version 1.2.3    # 指定版本的商业版
  bash scripts/offline-pack.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz
EOF
}

MIRROR="https://mirrors.zap.cn/zap/releases"
VERSION="latest"; PRO=0; LOCAL_PKG=""; OUT_DIR=""; ARCH=""

while [ $# -gt 0 ]; do
    case "$1" in
        --version)     VERSION="${2:-}"; [ -n "$VERSION" ] || die "--version 缺少版本号"; shift 2 ;;
        --version=*)   VERSION="${1#*=}"; shift ;;
        --pro)         PRO=1; shift ;;
        --arch)        ARCH="${2:-}"; [ -n "$ARCH" ] || die "--arch 缺少架构"; shift 2 ;;
        --arch=*)      ARCH="${1#*=}"; shift ;;
        --pkg)         LOCAL_PKG="${2:-}"; [ -n "$LOCAL_PKG" ] || die "--pkg 缺少路径"; shift 2 ;;
        --pkg=*)       LOCAL_PKG="${1#*=}"; shift ;;
        --out)         OUT_DIR="${2:-}"; [ -n "$OUT_DIR" ] || die "--out 缺少目录"; shift 2 ;;
        --out=*)       OUT_DIR="${1#*=}"; shift ;;
        -h|--help)     usage; exit 0 ;;
        *) die "未知参数: $1（--help 查看用法）" ;;
    esac
done

# 仓库根（本脚本位于 scripts/ 下）
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
[ -f "$HERE/install.sh" ] || die "未找到 scripts/install.sh（请在本仓库内运行）"
[ -f "$HERE/install-offline.sh" ] || die "未找到 scripts/install-offline.sh（请在本仓库内运行）"

PRO_SUFFIX=""; EDITION="Zap Community"
[ "$PRO" = "1" ] && { PRO_SUFFIX="-pro"; EDITION="Zap Pro"; }

if [ -z "$ARCH" ]; then
    case "$(uname -m)" in
        x86_64)             ARCH="amd64" ;;
        aarch64|arm64|arm*) ARCH="arm64" ;;
        *) die "无法识别当前架构（$(uname -m)），请用 --arch 指定" ;;
    esac
fi

[ -n "$OUT_DIR" ] || OUT_DIR="$ROOT/dist"
mkdir -p "$OUT_DIR"
OUT_DIR=$(cd "$OUT_DIR" && pwd)

# ── 取发布包：本地指定 > 镜像下载 ───────────────────────────
WORK=$(mktemp -d /tmp/zap-offline-pack.XXXXXX) || die "无法创建临时目录"
trap 'rm -rf "$WORK"' EXIT

if [ -n "$LOCAL_PKG" ]; then
    [ -f "$LOCAL_PKG" ] || die "指定的发布包不存在: ${LOCAL_PKG}"
    cp -f "$LOCAL_PKG" "$WORK/"
    PKG_NAME=$(basename "$LOCAL_PKG")
    # 从文件名反推版本 / 架构，产物名才不会张冠李戴
    if [[ "$PKG_NAME" =~ ^zap-v(.+)(-pro)?-linux-([a-z0-9_]+)\.tar\.gz$ ]]; then
        VERSION="${BASH_REMATCH[1]}"
        ARCH="${BASH_REMATCH[3]}"
        [ -n "${BASH_REMATCH[2]}" ] && { PRO=1; PRO_SUFFIX="-pro"; EDITION="Zap Pro"; }
    fi
else
    if [ "$VERSION" = "latest" ]; then
        command -v wget >/dev/null 2>&1 || command -v curl >/dev/null 2>&1 \
            || die "latest 需要联网查询版本号，但未找到 wget/curl（可显式给 --version）"
        info "查询最新版本..."
        if command -v wget >/dev/null 2>&1; then
            LATEST=$(wget -q -O - "${MIRROR}/latest.txt?t=$(date +%s)")
        else
            LATEST=$(curl -fsSL "${MIRROR}/latest.txt?t=$(date +%s)")
        fi
        [ -n "$LATEST" ] || die "查询最新版本失败（网络不可达？）"
        VERSION="$LATEST"
        ok "最新版本: ${VERSION}"
    fi
    PKG_NAME="zap-v${VERSION}${PRO_SUFFIX}-linux-${ARCH}.tar.gz"
    info "下载发布包 ${PKG_NAME} ..."
    if command -v wget >/dev/null 2>&1; then
        wget -O "$WORK/${PKG_NAME}" "${MIRROR}/${PKG_NAME}" || die "下载失败：${MIRROR}/${PKG_NAME}"
    elif command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$WORK/${PKG_NAME}" "${MIRROR}/${PKG_NAME}" || die "下载失败：${MIRROR}/${PKG_NAME}"
    else
        die "未找到 wget/curl，无法下载发布包（可先用 --pkg 指定本地包）"
    fi
    [ -s "$WORK/${PKG_NAME}" ] || die "下载内容为空：${MIRROR}/${PKG_NAME}"
    ok "发布包已就绪（${PKG_NAME}）"
fi

# ── 组装离线包内容 ─────────────────────────────────────────
STAGE="$WORK/zap-offline"
mkdir -p "$STAGE"
mv -f "$WORK/${PKG_NAME}" "$STAGE/"
cp -f "$HERE/install.sh" "$STAGE/install.sh"
[ -f "$HERE/uninstall.sh" ] && cp -f "$HERE/uninstall.sh" "$STAGE/uninstall.sh"
cp -f "$HERE/install-offline.sh" "$STAGE/install-offline.sh"
chmod 0755 "$STAGE"/*.sh

cat > "$STAGE/README-offline.txt" <<EOF
ZAP ${VERSION}（${EDITION}）离线安装包 · linux-${ARCH}

本包用于**内网 / 无外网**的机器，安装全程不访问网络。

在目标机上（root）：

    tar zxf zap-offline-v${VERSION}${PRO_SUFFIX}-linux-${ARCH}.tar.gz
    cd zap-offline
    sudo bash install-offline.sh

常用参数（原样透传给 install.sh）：

    --admin-user <name>   管理员用户名（默认 admin）
    --admin-pass <pass>   管理员密码（不指定则随机生成并打印）

Zap Pro 集群接入（可选）：

    sudo ZAP_JOIN_TOKEN='zec_…' bash install-offline.sh \\
        --join-url https://ctrl.example.com:2600/zap --join-insecure

说明：
  · install-offline.sh 会自动挑本目录下版本号最大的发布包，也可用 --pkg 指定
  · 安装前会按 SHA256SUMS 校验发布包；确认无误但仍报不一致时用 --no-verify
  · AppStore 使用发行包内置的种子包，不克隆远端仓库（面板里可随时重试更新）
  · 后续升级：把新版本的离线包拷进来重跑一次 install-offline.sh 即可

面板地址：https://<服务器 IP>:2600
EOF

(cd "$STAGE" && sha256sum * > SHA256SUMS) || warn "生成 SHA256SUMS 失败（目标机上将跳过校验）"
ok "内容已组装"

# ── 打包 ──────────────────────────────────────────────────
OUT_NAME="zap-offline-v${VERSION}${PRO_SUFFIX}-linux-${ARCH}.tar.gz"
tar czf "$OUT_DIR/${OUT_NAME}" -C "$WORK" zap-offline \
    || die "打包失败：${OUT_DIR}/${OUT_NAME}"
SIZE=$(du -h "$OUT_DIR/${OUT_NAME}" | awk '{print $1}')

printf "\n"
printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   离线安装包制作完成${NC}\n"
printf "${GREEN}========================================${NC}\n"
echo "  版本:   ${VERSION}（${EDITION}）"
echo "  架构:   linux-${ARCH}"
echo "  产物:   ${OUT_DIR}/${OUT_NAME}（${SIZE}）"
printf "\n"
printf "  拷到内网后：tar zxf ${OUT_NAME} && cd zap-offline && sudo bash install-offline.sh\n"
