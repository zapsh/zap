#!/usr/bin/env bash
# ZAP 发布打包脚本：构建 zapd / zapctl / zapexec / zapupgrade，打包并上传至 zap mirror
set -euo pipefail

# ── 终端颜色 ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { echo -e "${BLUE}[*]${NC} $*"; }
ok()   { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
die()  { echo -e "${RED}[✗]${NC} $*" >&2; exit 1; }

CUR_DIR=$(pwd)

# ── 架构与 Rust target 映射 ─────────────────────────────────
# 只支持「本机构建」：三个 BSD 都是 tier-3 target，没有预编译 std，交叉编译得
# 自己 build-std——CI 里不现实，BSD 的包要在对应的 BSD 机器上跑本脚本产出。
OS_NAME=$(uname -s | tr '[:upper:]' '[:lower:]')
MACHINE=$(uname -m)
case "$MACHINE" in
    x86_64)        ARCH="amd64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    *) die "不支持的架构: $MACHINE" ;;
esac
case "$OS_NAME" in
    linux)
        case "$MACHINE" in
            x86_64)        TARGET="x86_64-unknown-linux-gnu" ;;
            aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
        esac
        ;;
    openbsd)
        case "$MACHINE" in
            x86_64)        TARGET="x86_64-unknown-openbsd" ;;
            aarch64|arm64) TARGET="aarch64-unknown-openbsd" ;;
        esac
        ;;
    freebsd)
        case "$MACHINE" in
            x86_64)        TARGET="x86_64-unknown-freebsd" ;;
            aarch64|arm64) TARGET="aarch64-unknown-freebsd" ;;
        esac
        ;;
    *) die "不支持的操作系统: ${OS_NAME}（当前支持 linux / freebsd / openbsd 的本机构建）" ;;
esac
info "OS: ${OS_NAME}   架构: ${ARCH} (${TARGET})"

# ── 依赖检查 ────────────────────────────────────────────────
command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
# OpenBSD 自带 ftp，wget 要额外装包
if ! command -v wget >/dev/null 2>&1 && ! command -v ftp >/dev/null 2>&1; then
    die "未找到 wget 或 ftp，请先安装其中一个"
fi

if ! command -v zapfile >/dev/null 2>&1; then
    if [ "$OS_NAME" = "linux" ]; then
        info "未找到 zapfile，正在安装..."
        wget -qO- https://mirrors.zap.cn/zapfile/zapfile-linux-amd64 -O /usr/bin/zapfile \
            || die "zapfile 下载失败"
        chmod +x /usr/bin/zapfile
        ok "zapfile 安装完成"
    else
        # 只有 linux-amd64 的预编译包；其它平台上跳过上传，安装包照样打得出来
        warn "未找到 zapfile，且 ${OS_NAME} 无预编译包：跳过上传步骤"
    fi
fi

# ── 上传凭据 ────────────────────────────────────────────────
[ -n "${COS_ID:-}" ]  || die "环境变量 COS_ID 未设置"
[ -n "${COS_KEY:-}" ] || die "环境变量 COS_KEY 未设置"

# ── 版本号（从根 Cargo.toml 的 [workspace.package] 读取，各 crate 统一继承）─
VERSION=$(awk -F'"' '/^\[workspace\.package\]/{f=1} f&&/^version/{print $2; exit}' Cargo.toml)
[ -n "$VERSION" ] || die "无法从 Cargo.toml 解析 workspace 版本号"
info "版本: ${VERSION}"

# ── 前端产物（zapd 通过 rust-embed 内嵌 ../web/dist）────────────
# 必须在 cargo build **之前**构建：否则二进制内嵌的是上一次的前端产物，
# 页脚 / 系统更新页展示的 Web 版本就会落后于本次发布版本。
WEB_DIR="$CUR_DIR/web"
if [ -f "$WEB_DIR/package.json" ] && command -v npm >/dev/null 2>&1; then
    info "构建前端产物（web/dist）..."
    # 版本唯一来源：上面从根 Cargo.toml 解析出的 $VERSION，显式传给前端构建
    # （web/vite.config.ts 读取 ZAP_VERSION 注入；缺失时回退自行解析 Cargo.toml）
    (cd "$WEB_DIR" && ZAP_VERSION="$VERSION" npm run build:prod) || die "前端构建失败"
    ok "前端产物构建完成（v${VERSION}）"
else
    warn "跳过前端构建（缺少 web/package.json 或 npm）：二进制将内嵌现有 web/dist，页面展示的 Web 版本可能落后于本次发布"
fi

# ── 构建 ────────────────────────────────────────────────────
info "构建 release 二进制（${TARGET}）..."
cargo build --release --target "$TARGET" || die "构建失败"

# ── 打包 ────────────────────────────────────────────────────
DIST_DIR="$CUR_DIR/dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

BIN_DIR="$CUR_DIR/target/$TARGET/release"
# 发行包统一为「单层 zap/ 目录」布局：
#   zap/{zapd, zapctl, zapexec, zapupgrade} + zap/scripts + zap/data
# 二进制与资源同级，install.sh 直接以 zap/ 作为唯一内容根；
# zapupgrade 的 normalize_stage 同样从 zap/ 里取二进制，无需额外适配。
DIST_ZAP="$DIST_DIR/zap"
mkdir -p "$DIST_ZAP"
for bin in zapd zapctl zapexec zapupgrade; do
    cp -f "$BIN_DIR/$bin" "$DIST_ZAP/" || die "复制 $bin 失败"
done
ok "二进制复制完成: zap/{zapd, zapctl, zapexec, zapupgrade}"

# ── 内置 AppStore 源（git 仓库位于 data/appstore/repos/zap-appstore）──────
# 源内容由独立 git 仓库管理并随构建机维护在此目录；打包前若有 .git 则 pull 到最新，
# 无 .git（如 CI 检出仅含 gitlink 快照）则直接使用现有内容，缺失时发行包不含内置包。
APPSTORE_BUILTIN="$CUR_DIR/data/appstore/repos/zap-appstore"
if [ -d "$APPSTORE_BUILTIN" ]; then
    if [ -d "$APPSTORE_BUILTIN/.git" ]; then
        info "更新内置 AppStore 源（$APPSTORE_BUILTIN）..."
        # submodule / CI 检出默认 detached HEAD，先切回 main 分支再快进拉取
        git -C "$APPSTORE_BUILTIN" checkout -q main 2>/dev/null \
            || git -C "$APPSTORE_BUILTIN" checkout -q -B main origin/main 2>/dev/null \
            || true
        git -C "$APPSTORE_BUILTIN" pull -q --ff-only 2>/dev/null \
            || warn "内置源 git pull 失败，使用本地现有内容"
    else
        warn "内置源非 git 仓库（$APPSTORE_BUILTIN），跳过 pull，直接打包现有内容"
    fi
    [ -d "$APPSTORE_BUILTIN/database" ] \
        && ok "内置 AppStore 源就绪" \
        || warn "内置源目录为空（$APPSTORE_BUILTIN），发行包将不含内置包，可在面板中添加源"
else
    warn "未找到内置 AppStore 源（$APPSTORE_BUILTIN），发行包将不含内置包，可在面板中添加源"
fi

# 脚本、数据模板与配置（data/ 仅打包发行需要的内容，剔除运行时产物）
# 与二进制同级，统一放进 zap/ 目录
cp -Rf "$CUR_DIR/scripts" "$DIST_ZAP/"

# data/ 打包白名单：
#   appstore/repos/zap-appstore/          内置 AppStore 种子源
#   appstore/repos.yaml、custom/README.md 安装脚本(install.sh)依赖的模板
#   apps/README.md                        APPS_DIR 占位说明（apps 下其它为运行时安装实例，不打包）
#   www/                                 站点骨架模板(data/www/skel) + IP 默认页 / 维护页(data/www/_zap)
# 说明：systemd 服务模板、运维脚本、zap 共享工具与 conf 模板统一由 scripts/ 提供，
#       不重复打进 data/；安装后 data/ 是运行时数据区（zap.db、apps、appstore、run/ 等）
# 不打包：zap.db、run/、tmp/、apps/library、appstore 的 cache/logs/runs/tmp/custom/scripts
DIST_DATA="$DIST_ZAP/data"
mkdir -p "$DIST_DATA/apps"
mkdir -p "$DIST_DATA/appstore/repos"
cp -Rf "$CUR_DIR/data/appstore/repos/zap-appstore" "$DIST_DATA/appstore/repos/" 2>/dev/null || true
cp -f "$CUR_DIR/data/appstore/repos.yaml" "$DIST_DATA/appstore/" 2>/dev/null || true
cp -f "$CUR_DIR/data/apps/README.md" "$DIST_DATA/apps/" 2>/dev/null || true
# www/：站点骨架模板 skel/index.html 与 IP 默认页 / 维护页 _zap/*.html（运维可直接编辑）
cp -Rf "$CUR_DIR/data/www" "$DIST_DATA/" 2>/dev/null || true
mkdir -p "$DIST_DATA/www/html"
# 「文档」菜单的源 md（位于仓库根，供 build.sh 拷贝到 data/www/html/）。
# 默认文件名必须与 zapd/src/routers/docs.rs 里的 DOCS 白名单一一对应，
# 否则前端 GET /api/docs/<id> 会 404。
# 多语言后缀：<FILE>_<locale>.md（例 USER_MANUAL_zh-CN.md），由后端按 ?lang= 自动回退。
cp -Rf "$CUR_DIR/CHANGELOG.md"         "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/USER_MANUAL.md"       "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/FAQ.md"               "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/UPGRADE.md"           "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/CHANGELOG_zh-CN.md"   "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/USER_MANUAL_zh-CN.md" "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/FAQ_zh-CN.md"         "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/UPGRADE_zh-CN.md"     "$DIST_DATA/www/html/" 2>/dev/null || true

ok "资源复制完成"

cd "$DIST_DIR" || die "无法进入 dist 目录"
ZAP_FILE_NAME="zap-v${VERSION}-${OS_NAME}-${ARCH}.tar.gz"
info "打包 ${ZAP_FILE_NAME} ..."
tar -czf "$ZAP_FILE_NAME" * || die "打包失败"
ok "打包完成"

# 上传 install.sh 和 uninstall.sh（zapd 升级下载时使用）
if command -v zapfile >/dev/null 2>&1; then
    info "上传 install.sh ..."
    zapfile upload zap/ "$CUR_DIR/scripts/install.sh" || die "上传 install.sh 失败"
    info "上传 uninstall.sh ..."
    zapfile upload zap/ "$CUR_DIR/scripts/uninstall.sh" || die "上传 uninstall.sh 失败"
else
    warn "未安装 zapfile：跳过 install.sh / uninstall.sh 上传（面板在线升级需要它们）"
fi

# ── 上传 ────────────────────────────────────────────────────
info "上传 ${ZAP_FILE_NAME} ..."
zapfile upload zap/releases/ "$ZAP_FILE_NAME" || die "上传失败"
# sha256 校验文件（zapd 升级下载时比对；不带换行避免残留）
printf '%s' "$(sha256sum "$ZAP_FILE_NAME" | awk '{print $1}')" > "$ZAP_FILE_NAME.sha256"
info "上传校验文件 ${ZAP_FILE_NAME}.sha256 ..."
zapfile upload zap/releases/ "$ZAP_FILE_NAME.sha256" || die "上传校验文件失败"
zapfile put "zap/releases/latest.txt" $VERSION || die "更新版本文件失败"
ok "上传完成"

# ── 完成总结 ────────────────────────────────────────────────
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   ZAP v${VERSION} (${ARCH}) 发布完成${NC}"
echo -e "${GREEN}========================================${NC}"
echo "  包名: ${ZAP_FILE_NAME}"
