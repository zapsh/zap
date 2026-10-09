#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# ZAP 服务器/VPS 管理系统 卸载脚本
set -euo pipefail

# ── 终端颜色 ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { echo -e "${BLUE}[*]${NC} $*"; }
ok()   { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
die()  { echo -e "${RED}[✗]${NC} $*" >&2; exit 1; }

detect_lang() {
    local loc="${LANG_MODE:-${LC_ALL:-${LANG:-}}}"
    [[ -z "$loc" ]] && { echo "zh"; return; }
    case "$loc" in
        zh_CN*|zh_TW*|zh_HK*|zh_SG*|zh) echo "zh" ;;
        en_US*|en_GB*|en)               echo "en" ;;
        C|C.UTF-8|POSIX|"")             echo "zh" ;;  # 默认中文
        *)                              echo "en" ;;
    esac
}
LANG_MODE="$(detect_lang)"

if [ "$LANG_MODE" = "zh" ]; then
    L_MUST_ROOT="请以 root 身份运行: sudo bash $0"
    L_OS_UNSUPPORTED="不支持的操作系统: %s（当前仅支持 Linux）"
    L_UNINSTALL_TITLE="           ZAP 卸载程序"
    L_WILL_REMOVE="将删除以下内容："
    L_ITEM_SVC="systemd 服务"
    L_ITEM_LINKS="命令链接"
    L_ITEM_PROG="程序目录（二进制与脚本）"
    L_ITEM_CFG="配置目录（配置、证书、密钥）"
    L_ITEM_DATA="数据目录"
    L_DATA_PURGE="（含数据库，将彻底删除）"
    L_DATA_KEEP="（含数据库，将保留）"
    L_CONFIRM="确认卸载 ZAP 吗？(yes/no): "
    L_CANCELLED="已取消，未做任何改动"
    L_STOP_SVC="停止服务..."
    L_REMOVE_UNITS="删除服务单元与命令链接..."
    L_REMOVE_CFG="删除配置目录 /etc/zap ..."
    L_REMOVE_PROG_DATA="删除程序与数据目录 /usr/local/zap ..."
    L_REMOVE_PROG="删除程序目录（保留 data）..."
    L_DONE="ZAP 卸载完成"
    L_DATA_KEPT="数据目录 /usr/local/zap/data 已保留。"
    L_PURGE_HINT="如需彻底清除（含数据库），请执行: sudo bash %s --purge"
else
    L_MUST_ROOT="Please run as root: sudo bash $0"
    L_OS_UNSUPPORTED="Unsupported OS: %s (only Linux supported)"
    L_UNINSTALL_TITLE="           ZAP Uninstall"
    L_WILL_REMOVE="The following will be removed:"
    L_ITEM_SVC="systemd services"
    L_ITEM_LINKS="command links"
    L_ITEM_PROG="program directory (binaries and scripts)"
    L_ITEM_CFG="config directory (config, certs, keys)"
    L_ITEM_DATA="data directory"
    L_DATA_PURGE="(with database, will be permanently deleted)"
    L_DATA_KEEP="(with database, will be kept)"
    L_CONFIRM="Confirm uninstall ZAP? (yes/no): "
    L_CANCELLED="Cancelled, no changes made"
    L_STOP_SVC="Stopping services..."
    L_REMOVE_UNITS="Removing service units and command links..."
    L_REMOVE_CFG="Removing config dir /etc/zap ..."
    L_REMOVE_PROG_DATA="Removing program and data dir /usr/local/zap ..."
    L_REMOVE_PROG="Removing program dir (keeping data)..."
    L_DONE="ZAP uninstalled"
    L_DATA_KEPT="Data directory /usr/local/zap/data kept."
    L_PURGE_HINT="To fully remove (including database), run: sudo bash %s --purge"
fi

# ── 权限检查 ────────────────────────────────────────────────
[ "$(id -u)" -eq 0 ] || die "$L_MUST_ROOT"

# ── 平台探测 ────────────────────────────────────────────────
# 与 install.sh 保持一致：只支持 Linux + systemd。
OS=$(uname -s)
case "$OS" in
    Linux) ;;
    *) die "$(printf "$L_OS_UNSUPPORTED" "$OS")" ;;
esac

# ── 参数解析 ────────────────────────────────────────────────
PURGE=0
if [ "${1:-}" = "--purge" ]; then
    PURGE=1
fi

echo -e "${RED}========================================${NC}"
echo -e "${RED}${L_UNINSTALL_TITLE}${NC}"
echo -e "${RED}========================================${NC}"
echo ""

# ── 列出将删除的内容 ────────────────────────────────────────
echo "$L_WILL_REMOVE"
echo "  · ${L_ITEM_SVC} : zapd / zapexec"
echo "  · ${L_ITEM_LINKS}     : /usr/local/bin/{zapd,zapctl,zapexec}"
echo "  · ${L_ITEM_PROG} : /usr/local/zap"
echo "  · ${L_ITEM_CFG} : /etc/zap"
if [ "$PURGE" = "1" ]; then
    echo -e "  · ${RED}${L_ITEM_DATA}     : /usr/local/zap/data${L_DATA_PURGE}${NC}"
else
    echo -e "  · ${GREEN}${L_ITEM_DATA}     : /usr/local/zap/data${L_DATA_KEEP}${NC}"
fi
echo ""

# ── 确认 ────────────────────────────────────────────────────
read -r -p "$L_CONFIRM" ans
if [ "$ans" != "yes" ]; then
    info "$L_CANCELLED"
    exit 0
fi
echo ""

# ── 停止服务 ────────────────────────────────────────────────
info "$L_STOP_SVC"
systemctl stop zapd.service zapexec.service 2>/dev/null || true
systemctl disable zapd.service zapexec.service 2>/dev/null || true
systemctl daemon-reload

# ── 删除服务单元与命令链接 ──────────────────────────────────
info "$L_REMOVE_UNITS"
rm -f /etc/systemd/system/zapd.service /etc/systemd/system/zapexec.service
rm -f /usr/local/bin/zapd /usr/local/bin/zapctl /usr/local/bin/zapexec

# ── 删除配置目录 ────────────────────────────────────────────
info "$L_REMOVE_CFG"
rm -rf /etc/zap

# ── 删除程序目录（按需保留数据）─────────────────────────────
if [ "$PURGE" = "1" ]; then
    info "$L_REMOVE_PROG_DATA"
    rm -rf /usr/local/zap
else
    info "$L_REMOVE_PROG"
    if [ -d /usr/local/zap/data ]; then
        # 安装目录下只有本脚本部署的条目，没有点开头的文件，glob 足够
        # （不用 find，避免依赖 GNU 扩展）。
        for entry in /usr/local/zap/*; do
            [ -e "$entry" ] || continue
            [ "${entry##*/}" = "data" ] && continue
            rm -rf "$entry"
        done
    else
        rm -rf /usr/local/zap
    fi
fi

# ── 完成 ────────────────────────────────────────────────────
ok "$L_DONE"
echo ""
if [ "$PURGE" != "1" ]; then
    echo -e "${YELLOW}${L_DATA_KEPT}${NC}"
    echo -e "${YELLOW}$(printf "$L_PURGE_HINT" "$0")${NC}"
fi
