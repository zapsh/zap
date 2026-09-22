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

# ── 参数 ────────────────────────────────────────────────────
# 默认只发社区版；--with-pro 一次发两份（社区版 + 商业版），两条发行线同版本。
usage() {
    cat <<'EOF'
用法：build.sh [选项]

  （无参数）   只构建发布社区版（开源，包名 zap-v<版本>-linux-amd64.tar.gz）
  --with-pro   连同商业版 Zap Pro 一起发布：**编两份、发两个包**
               商业版 = cargo --features zapd/commercial，包名加 -pro
               （zap-v<版本>-pro-linux-amd64.tar.gz）
               要求：仓库根目录下已有 zappro/（独立私有仓库，见 zappro/README.md）
  --pro-only   只构建发布商业版（社区版已在别处发布完时用）
  -h, --help   显示本帮助

两条发行线的包名不同，在线升级各自认自己的那条（社区版装不出 Pro，
Pro 装完升级也不会退化成社区版）；版本号共用 latest.txt，同版本一起发。
EOF
}
WITH_PRO=0
PRO_ONLY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --with-pro) WITH_PRO=1; shift ;;
        --pro-only) PRO_ONLY=1; shift ;;
        -h|--help)  usage; exit 0 ;;
        *)          echo -e "${RED}[✗]${NC} 未知参数: $1" >&2; usage >&2; exit 1 ;;
    esac
done

# 默认发社区版；--with-pro 追加商业版；--pro-only 只发商业版
BUILD_COMMUNITY=1
BUILD_PRO=0
[[ "$WITH_PRO" -eq 1 ]] && BUILD_PRO=1
[[ "$PRO_ONLY" -eq 1 ]] && { BUILD_PRO=1; BUILD_COMMUNITY=0; }

if [[ "$BUILD_PRO" -eq 1 ]]; then
    # 模块源码不在本仓库，没 clone 就直接失败：开着开关编出「假 Pro 版」更危险
    [ -f "$CUR_DIR/zappro/src/mod.rs" ] \
        || die "要构建商业版但未找到 $CUR_DIR/zappro（先 clone 商业模块仓库）"
    info "商业版 Zap Pro：--features zapd/commercial"
fi
# Pro 页面的「有无」在各自的构建阶段处理（见 prepare_web），这里只做模块源码的提前校验。

# ── 架构与 Rust target 映射 ─────────────────────────────────
# 只支持 Linux 本机构建：打包产物按 OS / 架构命名。
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
    *) die "不支持的操作系统: ${OS_NAME}（当前仅支持 linux 本机构建）" ;;
esac
info "OS: ${OS_NAME}   架构: ${ARCH} (${TARGET})"

# ── 依赖检查 ────────────────────────────────────────────────
command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
command -v wget >/dev/null 2>&1 || die "未找到 wget，请先安装"

if ! command -v zapfile >/dev/null 2>&1; then
    info "未找到 zapfile，正在安装..."
    wget -qO- https://mirrors.zap.cn/zapfile/zapfile-linux-amd64 -O /usr/bin/zapfile \
        || die "zapfile 下载失败"
    chmod +x /usr/bin/zapfile
    ok "zapfile 安装完成"
fi

# ── 上传凭据 ────────────────────────────────────────────────
[ -n "${COS_ID:-}" ]  || die "环境变量 COS_ID 未设置"
[ -n "${COS_KEY:-}" ] || die "环境变量 COS_KEY 未设置"

# ── 版本号（从根 Cargo.toml 的 [workspace.package] 读取，各 crate 统一继承）─
VERSION=$(awk -F'"' '/^\[workspace\.package\]/{f=1} f&&/^version/{print $2; exit}' Cargo.toml)
[ -n "$VERSION" ] || die "无法从 Cargo.toml 解析 workspace 版本号"
info "版本: ${VERSION}"

# ── 前端产物（zapd 通过 rust-embed 内嵌 ../web/dist）────────────
# 两个版本内嵌的不是同一份前端：vite 用 import.meta.glob('../views/**/*.vue')
# 扫描整个 views 目录，web/src/views/pro 存在时 Pro 页面会一起进包。
# 所以每份构建都要「先摆好 Pro 页面的有无 → 再构建前端 → 再 cargo build」，
# 否则 --with-pro 会把 Pro 页面带进社区版二进制（路由不显示，代码已在里面）。
# 同理必须在 cargo build **之前**构建：否则二进制内嵌的是上一次的前端产物，
# 页脚 / 系统更新页展示的 Web 版本就会落后于本次发布版本。
WEB_DIR="$CUR_DIR/web"
if [ -f "$WEB_DIR/package.json" ] && command -v npm >/dev/null 2>&1; then
    WEB_BUILD=1
else
    WEB_BUILD=0
    warn "跳过前端构建（缺少 web/package.json 或 npm）：二进制将内嵌现有 web/dist，页面展示的 Web 版本可能落后于本次发布"
fi

# 用法：prepare_web <community|pro>
# 按版本决定 web/src/views/pro 的存在与否（必须在构建前端之前调用）。
prepare_web() {
    local edition="$1"
    if [[ "$edition" == "pro" ]]; then
        # 商业版：同步一次，保证打进去的是模块仓库里的最新页面（幂等，覆盖旧副本）
        if [ -d "$CUR_DIR/zappro/web/views" ]; then
            info "同步 Pro 页面 → web/src/views/pro ..."
            bash "$CUR_DIR/zappro/pro.sh" setup || die "同步 Pro 页面失败（zappro/pro.sh setup）"
            ok "Pro 页面已同步"
        elif [ -d "$CUR_DIR/web/src/views/pro" ]; then
            warn "zappro/web/views 不存在，沿用已同步的 web/src/views/pro（可能不是最新）"
        else
            die "未找到 Pro 页面：$CUR_DIR/zappro/web/views 与 $CUR_DIR/web/src/views/pro 都不存在（模块仓库是否完整 clone？）"
        fi
    elif [ -d "$CUR_DIR/web/src/views/pro" ]; then
        # 社区版：清掉，别让 Pro 页面混进社区版前端
        info "清理 web/src/views/pro（社区版不含商业版页面）..."
        if [ -f "$CUR_DIR/zappro/pro.sh" ]; then
            bash "$CUR_DIR/zappro/pro.sh" clean || die "清理 Pro 页面失败（zappro/pro.sh clean）"
        else
            rm -rf "$CUR_DIR/web/src/views/pro" || die "清理 Pro 页面失败"
        fi
        ok "已移除 web/src/views/pro（发商业版时会重新同步）"
    fi
}

# 用法：build_web
build_web() {
    if [[ "$WEB_BUILD" -ne 1 ]]; then
        warn "跳过前端构建：沿用现有 web/dist"
        return 0
    fi
    info "构建前端产物（web/dist）..."
    # 版本唯一来源：上面从根 Cargo.toml 解析出的 $VERSION，显式传给前端构建
    # （web/vite.config.ts 读取 ZAP_VERSION 注入；缺失时回退自行解析 Cargo.toml）
    (cd "$WEB_DIR" && ZAP_VERSION="$VERSION" npm run build:prod) || die "前端构建失败"
    ok "前端产物构建完成（v${VERSION}）"
}

# ── 打包目录 ────────────────────────────────────────────────
DIST_DIR="$CUR_DIR/dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

BIN_DIR="$CUR_DIR/target/$TARGET/release"
# 两份构建共用同一个 target 目录：后一次 build 会覆盖前一次的 zapd
# （社区版 / 商业版的差别就在 zapd 上），所以每编完一份立刻把四个二进制
# 挪到各自的暂存目录，最后分别打包。
BIN_STAGE="$DIST_DIR/.bins"
PACKAGES=()

# 用法：build_variant <community|pro>
build_variant() {
    local edition="$1" suffix=""
    local -a flags=()
    if [[ "$edition" == "pro" ]]; then
        suffix="-pro"
        flags=(--features zapd/commercial)
    fi
    info "构建 ${edition} 版 release 二进制（${TARGET}）..."
    cargo build --release --target "$TARGET" ${flags[@]+"${flags[@]}"} \
        || die "构建失败（${edition}）"
    mkdir -p "$BIN_STAGE/$edition"
    for bin in zapd zapctl zapexec zapupgrade; do
        cp -f "$BIN_DIR/$bin" "$BIN_STAGE/$edition/$bin" \
            || die "复制 $bin 失败（${edition}）"
    done
    ok "${edition} 二进制就位"
}

# 发行包统一为「单层 zap/ 目录」布局：
#   zap/{zapd, zapctl, zapexec, zapupgrade} + zap/scripts + zap/data
# 二进制与资源同级，install.sh 直接以 zap/ 作为唯一内容根；
# zapupgrade 的 normalize_stage 同样从 zap/ 里取二进制，无需额外适配。
DIST_ZAP="$DIST_DIR/zap"
mkdir -p "$DIST_ZAP"

# 用法：package_variant <community|pro> <包名后缀>
package_variant() {
    local edition="$1" suffix="$2"
    for bin in zapd zapctl zapexec zapupgrade; do
        cp -f "$BIN_STAGE/$edition/$bin" "$DIST_ZAP/$bin" \
            || die "复制 $bin 失败（${edition}）"
    done
    local name="zap-v${VERSION}${suffix}-${OS_NAME}-${ARCH}.tar.gz"
    info "打包 ${name} ..."
    (cd "$DIST_DIR" && tar -czf "$name" zap) || die "打包失败（${name}）"
    PACKAGES+=("$name")
    ok "打包完成：${name}"
}

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
# 剔除 Windows 资源管理器在挂载盘/NTFS 上留下的 ADS 残留（形如 `FOO.md:Zone.Identifier`），
# 否则它们会跟着发行包一起装到线上（无意义文件，还容易让文档目录看着一堆脏东西）
find "$DIST_DATA/www/html" -type f -name '*:Zone.Identifier' -delete 2>/dev/null || true

ok "资源复制完成（两份包共用同一套 scripts / data）"

# 注意：后面一律用绝对路径，不要再 `cd dist` —— cargo 必须落在仓库根的 target/。
# ── 构建 + 打包（顺序有意义：编完一份就打包一份，别让后一次 build 覆盖前一次的二进制）──
if [[ "$BUILD_COMMUNITY" -eq 1 ]]; then
    prepare_web community
    build_web
    build_variant community
    package_variant community ""
fi
if [[ "$BUILD_PRO" -eq 1 ]]; then
    prepare_web pro
    build_web
    build_variant pro
    package_variant pro "-pro"
fi
[ "${#PACKAGES[@]}" -gt 0 ] || die "没有要发布的包（检查 --with-pro / --pro-only 参数）"

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
for pkg in "${PACKAGES[@]}"; do
    info "上传 ${pkg} ..."
    zapfile upload zap/releases/ "$DIST_DIR/$pkg" || die "上传失败：${pkg}"
    # sha256 校验文件（zapd 升级下载时比对；不带换行避免残留）
    printf '%s' "$(sha256sum "$DIST_DIR/$pkg" | awk '{print $1}')" > "$DIST_DIR/$pkg.sha256"
    info "上传校验文件 ${pkg}.sha256 ..."
    zapfile upload zap/releases/ "$DIST_DIR/$pkg.sha256" || die "上传校验文件失败：${pkg}"
done
# 两条线同版本：共用一个 latest.txt（商业版装的就是同版本的 -pro 包）
zapfile put "zap/releases/latest.txt" $VERSION || die "更新版本文件失败"
ok "上传完成"

# ── 完成总结 ────────────────────────────────────────────────
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   ZAP v${VERSION} (${ARCH}) 发布完成${NC}"
echo -e "${GREEN}========================================${NC}"
echo "  版本: ${VERSION}"
for pkg in "${PACKAGES[@]}"; do
    echo "  包名: ${pkg}"
done
echo "  安装: bash install.sh ${VERSION}$([[ "$BUILD_PRO" -eq 1 ]] && echo ' --pro')"
