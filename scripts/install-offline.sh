#!/usr/bin/env bash
# ZAP 离线安装入口 —— 给内网 / 无外网访问的机器。
#
# 它本身不装任何东西，只做三件事：
#   1. 找到本地安装包（--pkg 指定，或在本脚本同目录里挑版本号最大的那个）
#   2. 有 SHA256SUMS 就先校验一遍（离线包由 offline-pack.sh 生成时带的）
#   3. 把剩下的活原样交给 install.sh --pkg <包> --offline
#
# 这样离线与在线走的是同一条安装路径（同一份部署 / 建库 / systemd 逻辑），
# 离线只是把「取包」和「联网的几步」换成本地 / 跳过，不会出现两套行为不一致。
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { printf "${BLUE}[*]${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*"; }
die()  { printf "${RED}[✗]${NC} %s\n" "$*" >&2; exit 1; }

usage() {
    cat <<'EOF'
用法: sudo bash install-offline.sh [选项] [-- 传给 install.sh 的参数]

离线安装（内网 / 无外网）：全程不访问网络，安装包取自本地。

选项:
  --pkg <路径>      指定本地安装包（zap-v<版本>[-pro]-<os>-<arch>.tar.gz）
                    不指定时，在本脚本所在目录自动挑选版本号最大的那个
  --no-verify       跳过 SHA256SUMS 校验（包被改动过 / 没有校验文件时用）
  -h, --help        显示本帮助

其余参数（--admin-user / --admin-pass / --pro / --join-url ...）原样透传给 install.sh。

示例:
  sudo bash install-offline.sh                                   # 用同目录下的发布包
  sudo bash install-offline.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz
  sudo bash install-offline.sh --pro --admin-pass 'S3cret-Pass'  # 离线装商业版
EOF
}

# ── 解释器检查：本脚本用了 bash 数组与 pipefail ──────────────
if [ -z "${BASH_VERSION:-}" ]; then
    for b in /bin/bash /usr/bin/bash; do
        if [ -x "$b" ]; then exec "$b" "$0" "$@"; fi
    done
    die "未找到 bash：请先安装后重试"
fi

# --help 要在 root 检查之前：非 root 想看用法时不该被「请先 sudo」挡回来
for a in "$@"; do
    [ "$a" = "-h" ] || [ "$a" = "--help" ] || continue
    usage
    exit 0
done

[ "$(id -u)" -eq 0 ] || die "请以 root 身份运行：sudo bash $0"

PKG=""; NO_VERIFY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --pkg)       PKG="${2:-}"; [ -n "$PKG" ] || die "--pkg 缺少路径"; shift 2 ;;
        --pkg=*)     PKG="${1#*=}"; [ -n "$PKG" ] || die "--pkg 缺少路径"; shift ;;
        --no-verify) NO_VERIFY=1; shift ;;
        -h|--help)   usage; exit 0 ;;
        *) break ;;   # 其余参数留给 install.sh
    esac
done

# 本脚本所在目录（离线包解开后，包与 install.sh 都跟它同级）
HERE=$(cd "$(dirname "$0")" && pwd)

# ── 定位安装包 ─────────────────────────────────────────────
if [ -z "$PKG" ]; then
    # 只认发布包（zap-v…），不认外层离线包（zap-offline-…）
    candidates=$(cd "$HERE" && ls -1 2>/dev/null | grep -E '^zap-v.*\.tar\.gz$' || true)
    [ -n "$candidates" ] || die "在 ${HERE} 下没找到发布包（zap-v<版本>-*.tar.gz）。
  离线包请用 scripts/offline-pack.sh 制作，或用 --pkg 直接指定本地包路径。"
    count=$(printf '%s\n' "$candidates" | wc -l | tr -d ' ')
    if [ "$count" -gt 1 ]; then
        # 取版本号最大的那个：sort -V 认得 v1.10 > v1.9；不支持时退回字典序
        PKG=$(printf '%s\n' "$candidates" | (sort -V 2>/dev/null || sort) | tail -1)
        warn "目录下有 ${count} 个发布包，已选择最新的 ${PKG}（要指定其它包请用 --pkg）"
    else
        PKG="$candidates"
    fi
    PKG="$HERE/$PKG"
fi
[ -f "$PKG" ] || die "安装包不存在: ${PKG}"
ok "安装包: ${PKG}"

# ── 校验（SHA256SUMS 由 offline-pack.sh 生成；没有就跳过）──
SUMS="$HERE/SHA256SUMS"
if [ "$NO_VERIFY" = "1" ]; then
    warn "已按 --no-verify 跳过校验"
elif [ -f "$SUMS" ] && command -v sha256sum >/dev/null 2>&1; then
    name=$(basename "$PKG")
    if grep -q "[[:space:]]${name}$" "$SUMS"; then
        info "校验 ${name} ..."
        (cd "$HERE" && grep "[[:space:]]${name}$" "$SUMS" | sha256sum -c -) \
            || die "校验失败：${name} 与 SHA256SUMS 不一致（包在传递中损坏？）"
        ok "校验通过"
    else
        warn "SHA256SUMS 里没有 ${name}，跳过校验"
    fi
else
    [ -f "$SUMS" ] || warn "未找到 SHA256SUMS，跳过校验（建议用 offline-pack.sh 生成离线包）"
fi

# ── 交给 install.sh：离线只是它的一种取包方式 ────────────────
INSTALL_SH="$HERE/install.sh"
[ -f "$INSTALL_SH" ] || die "未找到 install.sh（查找位置: ${HERE}）。
  离线包里自带 install.sh；手工拷贝时请把它一起带上（仓库 scripts/install.sh）。"
chmod +x "$INSTALL_SH" 2>/dev/null || true

printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   ZAP 离线安装（无外网环境）${NC}\n"
printf "${GREEN}========================================${NC}\n"
info "后续升级: sudo bash ${HERE}/upgrade-offline.sh --pkg <新版本发布包>"
exec bash "$INSTALL_SH" --pkg "$PKG" --offline "$@"
