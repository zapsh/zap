#!/usr/bin/env bash
# ZAP 离线升级入口 —— 给内网 / 无外网访问的机器。
#
# 与 install-offline.sh 同一套思路：它自己不替换任何二进制，只做
#   1. 找到本地发布包（--pkg 指定，或在本脚本同目录里挑版本号最大的那个）
#   2. 有 SHA256SUMS 就先校验一遍（传递过程有没有损坏）
#   3. 把剩下的活交给 zapupgrade upgrade --pkg <包>
#
# 于是「离线升级」与「在线升级」走的是**同一个 zapupgrade 替换流程**
# （备份 → 替换 → 重启 → 失败自动回滚 → 可 rollback），离线只是把
# 「下载」换成「读本地文件」。离线的升级质量因此与在线一致 —— 这一点
# 比省事重要：升级一旦半途出错，回滚是唯一能指望的东西。
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { printf "${BLUE}[*]${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*"; }
die()  { printf "${RED}[✗]${NC} %s\n" "$*" >&2; exit 1; }

usage() {
    cat <<'EOF'
用法: sudo bash upgrade-offline.sh [选项]

离线升级（内网 / 无外网）：用本地发布包升级，全程不访问网络。

选项:
  --pkg <路径>      本地发布包（zap-v<版本>[-pro]-linux-<arch>.tar.gz）
                    不指定时，在本脚本所在目录自动挑选版本号最大的那个
  --dir <路径>      ZAP 安装根目录（默认 /usr/local/zap）
  --force           目标版本不高于当前版本时仍然升级（重装 / 回退用）
  --pro / --community  换发行线时用它显式确认（见下）
  --no-verify       跳过 sha256 校验（包被改动过 / 没有校验值时用）
  -h, --help        显示本帮助

说明:
  · 升级前会自动备份当前二进制到 <dir>/data/upgrade/backup，失败可回滚：
      zapupgrade rollback --list
  · 社区版 ⇄ Pro 属于换发行线，必须显式加 --pro 或 --community，
    否则拒绝执行（避免拷错包把 Pro 覆盖成社区版）
  · 包是 Linux 哪个架构由文件名决定，拷错架构会被拒绝

示例:
  sudo bash upgrade-offline.sh                                  # 用同目录下的发布包
  sudo bash upgrade-offline.sh --pkg ./zap-v1.2.3-linux-amd64.tar.gz
  sudo bash upgrade-offline.sh --pkg ./zap-v1.2.3-pro-linux-amd64.tar.gz --pro
EOF
}

# ── 解释器检查 ──────────────────────────────────────────────
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

PKG=""; NO_VERIFY=0; ZAP_DIR="/usr/local/zap"; EXTRA=()
while [ $# -gt 0 ]; do
    case "$1" in
        --pkg)       PKG="${2:-}"; [ -n "$PKG" ] || die "--pkg 缺少路径"; shift 2 ;;
        --pkg=*)     PKG="${1#*=}"; [ -n "$PKG" ] || die "--pkg 缺少路径"; shift ;;
        --dir)       ZAP_DIR="${2:-}"; [ -n "$ZAP_DIR" ] || die "--dir 缺少路径"; shift 2 ;;
        --dir=*)     ZAP_DIR="${1#*=}"; shift ;;
        --no-verify) NO_VERIFY=1; EXTRA+=(--no-verify); shift ;;
        --force)     EXTRA+=(--force); shift ;;
        --pro)       EXTRA+=(--pro); shift ;;
        --community) EXTRA+=(--community); shift ;;
        -h|--help)   usage; exit 0 ;;
        *) die "未知参数: $1（--help 查看用法）" ;;
    esac
done

HERE=$(cd "$(dirname "$0")" && pwd)

# ── 升级前必须先有已安装实例 ─────────────────────────────────
# 升级不管建库 / systemd，全靠现有安装；没装就用 install-offline.sh
[ -x "$ZAP_DIR/zapd" ] || die "在 ${ZAP_DIR} 下没找到已安装的 zapd。
  本机还没装 ZAP：请用 install-offline.sh 安装，而不是升级。
  装在了别处时用 --dir 指定安装根目录。"

UPGRADER="$ZAP_DIR/zapupgrade"
[ -x "$UPGRADER" ] || die "未找到升级器 ${UPGRADER}（用 --dir 指定安装根目录）"

CUR=$("$ZAP_DIR/zapd" --version 2>/dev/null | awk '{print $NF}') || CUR=""
ok "当前版本: ${CUR:-未知}"

# ── 定位发布包 ─────────────────────────────────────────────
if [ -z "$PKG" ]; then
    # 只认发布包（zap-v…）：外层离线包（zap-offline-…）会被 zapupgrade 拒，
    # 但在这里就筛掉，省得用户看两条不同措辞的报错
    candidates=$(cd "$HERE" && ls -1 2>/dev/null | grep -E '^zap-v.*\.tar\.gz$' || true)
    [ -n "$candidates" ] || die "在 ${HERE} 下没找到发布包（zap-v<版本>-*.tar.gz）。
  离线包请用 scripts/offline-pack.sh 制作，解开后里面那个 zap-v*.tar.gz 才是发布包。"
    count=$(printf '%s\n' "$candidates" | wc -l | tr -d ' ')
    if [ "$count" -gt 1 ]; then
        PKG=$(printf '%s\n' "$candidates" | (sort -V 2>/dev/null || sort) | tail -1)
        warn "目录下有 ${count} 个发布包，已选择最新的 ${PKG}（要指定其它包请用 --pkg）"
    else
        PKG="$candidates"
    fi
    PKG="$HERE/$PKG"
fi
[ -f "$PKG" ] || die "发布包不存在: ${PKG}"
ok "发布包: ${PKG}"

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

printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   ZAP 离线升级（无外网环境）${NC}\n"
printf "${GREEN}========================================${NC}\n"
info "升级日志: ${ZAP_DIR}/data/upgrade/logs/"
exec "$UPGRADER" upgrade --pkg "$PKG" --dir "$ZAP_DIR" "${EXTRA[@]}"
