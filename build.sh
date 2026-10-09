#!/usr/bin/env bash
# ZAP release packaging script: builds zapd / zapctl / zapexec / zapupgrade, packages them and uploads to the zap mirror
if [ -z "${BASH_VERSION:-}" ]; then
    exec bash "$0" "$@"
fi
set -euo pipefail

# ── Terminal colors ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { echo -e "${BLUE}[*]${NC} $*"; }
ok()   { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
die()  { echo -e "${RED}[✗]${NC} $*" >&2; exit 1; }

CUR_DIR=$(pwd)

usage() {
    cat <<'EOF'
usage:build.sh [OPTIONS]
  -h, --help   Show this help
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help)  usage; exit 0 ;;
        *)          echo -e "${RED}[✗]${NC} Unknown argument: $1" >&2; usage >&2; exit 1 ;;
    esac
done

# ── Architecture and Rust target mapping ─────────────────────────────────
# Linux native builds only: packaged artifacts are named by OS / architecture.
OS_NAME=$(uname -s | tr '[:upper:]' '[:lower:]')
MACHINE=$(uname -m)
case "$MACHINE" in
    x86_64)        ARCH="amd64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    *) die "Unsupported architecture: $MACHINE" ;;
esac
case "$OS_NAME" in
    linux)
        case "$MACHINE" in
            x86_64)        TARGET="x86_64-unknown-linux-gnu" ;;
            aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
        esac
        ;;
    *) die "Unsupported OS: ${OS_NAME} (only linux native builds are supported)" ;;
esac
info "OS: ${OS_NAME}   Arch: ${ARCH} (${TARGET})"

# ── Dependency checks ────────────────────────────────────────────────
command -v cargo >/dev/null 2>&1 || die "cargo not found, please install Rust first"
command -v wget >/dev/null 2>&1 || die "wget not found, please install it"

if ! command -v zapfile >/dev/null 2>&1; then
    info "zapfile not found, installing..."
    wget -qO- https://mirrors.zap.cn/zapfile/zapfile-linux-amd64 -O /usr/bin/zapfile \
        || die "zapfile download failed"
    chmod +x /usr/bin/zapfile
    ok "zapfile installed"
fi

# ── Upload credentials ────────────────────────────────────────────────
[ -n "${COS_ID:-}" ]  || die "env var COS_ID is not set"
[ -n "${COS_KEY:-}" ] || die "env var COS_KEY is not set"

# ── Version (read from [workspace.package] in root Cargo.toml; all crates inherit it) ─
VERSION=$(awk -F'"' '/^\[workspace\.package\]/{f=1} f&&/^version/{print $2; exit}' Cargo.toml)
[ -n "$VERSION" ] || die "failed to parse workspace version from Cargo.toml"
info "Version: ${VERSION}"

# ── Frontend assets (zapd embeds ../web/dist via rust-embed) ────
# Must be built BEFORE cargo build: otherwise the binary embeds the previous frontend
# assets, and the Web version shown in the footer / system-update page would lag behind this release.
WEB_DIR="$CUR_DIR/web"
if [ -f "$WEB_DIR/package.json" ] && command -v npm >/dev/null 2>&1; then
    WEB_BUILD=1
else
    WEB_BUILD=0
    warn "Skipping frontend build (missing web/package.json or npm): the binary will embed the existing web/dist; the Web version shown may lag behind this release"
fi

# Usage: build_web
build_web() {
    if [[ "$WEB_BUILD" -ne 1 ]]; then
        warn "Skipping frontend build: reusing existing web/dist"
        return 0
    fi
    info "Building frontend assets (web/dist)..."
    # Single source of truth for version: $VERSION parsed above from root Cargo.toml,
    # passed explicitly to the frontend build (web/vite.config.ts reads ZAP_VERSION to
    # inject it; falls back to parsing Cargo.toml if missing)
    (cd "$WEB_DIR" && ZAP_VERSION="$VERSION" npm run build:prod) || die "frontend build failed"
    ok "Frontend assets built (v${VERSION})"
}

# ── Packaging directory ────────────────────────────────────────────────
DIST_DIR="$CUR_DIR/dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

BIN_DIR="$CUR_DIR/target/$TARGET/release"
# After building, move the four binaries into a staging dir for final packaging (with resources, into zap/).
BIN_STAGE="$DIST_DIR/.bins"
PACKAGES=()

# Usage: build_variant <community|pro>
# Community edition has no commercial features (the --features zapd/commercial build is split into zappro/build_pro.sh).
build_variant() {
    local edition="$1"
    info "Building ${edition} release binaries (${TARGET})..."
    cargo build --release --target "$TARGET" \
        || die "build failed (${edition})"
    mkdir -p "$BIN_STAGE/$edition"
    for bin in zapd zapctl zapexec zapupgrade; do
        cp -f "$BIN_DIR/$bin" "$BIN_STAGE/$edition/$bin" \
            || die "failed to copy $bin (${edition})"
    done
    ok "${edition} binaries ready"
}

# Release packages use a single-level "zap/" directory layout:
#   zap/{zapd, zapctl, zapexec, zapupgrade} + zap/scripts + zap/data
# Binaries and resources are at the same level; install.sh uses zap/ as the sole content root;
# zapupgrade's normalize_stage also takes binaries from zap/, requiring no extra adaptation.
DIST_ZAP="$DIST_DIR/zap"
mkdir -p "$DIST_ZAP"

# Usage: package_variant <community|pro> <package name suffix>
package_variant() {
    local edition="$1" suffix="$2"
    for bin in zapd zapctl zapexec zapupgrade; do
        cp -f "$BIN_STAGE/$edition/$bin" "$DIST_ZAP/$bin" \
            || die "failed to copy $bin (${edition})"
    done
    local name="zap-v${VERSION}${suffix}-${OS_NAME}-${ARCH}.tar.gz"
    info "Packaging ${name} ..."
    (cd "$DIST_DIR" && tar -czf "$name" zap) || die "packaging failed (${name})"
    PACKAGES+=("$name")
    ok "Packaged: ${name}"
}

# ── Bundled AppStore source (standalone git repo: data/appstore/repos/appstore) ──────
# This dir is excluded by .gitignore and maintained as a git project independent of the
# zap main repo (no longer a submodule).
# Key: the release package must carry a "self-contained" .git (a real directory). When build.sh packages it:
#   has a real .git dir -> switch back to main in place and fast-forward;
#   otherwise (dir missing / snapshot only) -> shallow-clone a real .git using the same HTTPS URL as repos.yaml.
# This way the target machine's first "update" does a fetch instead of a full clone
# (slow and often times out at 180s in China due to GitHub being unreachable).
# Note: HTTPS must be used to avoid the old .gitmodules git@ SSH address (git@github.com:)
# causing the target machine's fetch to fail for lack of an SSH key.
APPSTORE_BUILTIN="$CUR_DIR/data/appstore/repos/appstore"
APPSTORE_URL="https://github.com/zapsh/appstore.git"   # keep in sync with data/appstore/repos.yaml
if [ -d "$APPSTORE_BUILTIN" ]; then
    if [ -d "$APPSTORE_BUILTIN/.git" ]; then
        info "Updating bundled AppStore source ($APPSTORE_BUILTIN)..."
        # submodule / CI checkouts default to detached HEAD; switch back to main then fast-forward pull
        git -C "$APPSTORE_BUILTIN" checkout -q main 2>/dev/null \
            || git -C "$APPSTORE_BUILTIN" checkout -q -B main origin/main 2>/dev/null \
            || true
        git -C "$APPSTORE_BUILTIN" pull -q --ff-only 2>/dev/null \
            || warn "Bundled source git pull failed, using local existing content"
    else
        # No real .git (gitlink pointer or pure snapshot): shallow-clone via HTTPS to produce a
        # self-contained repo; updates do fetch rather than clone
        APPSTORE_TMP="${APPSTORE_BUILTIN}.tmp"
        rm -rf "$APPSTORE_TMP"
        if git clone --depth 1 --branch main "$APPSTORE_URL" "$APPSTORE_TMP" 2>/dev/null; then
            rm -rf "$APPSTORE_BUILTIN"
            mv "$APPSTORE_TMP" "$APPSTORE_BUILTIN"
            ok "Bundled AppStore source shallow-cloned (ships its own .git, updates via fetch)"
        else
            warn "Bundled source clone failed, using existing content (package lacks .git; update requires a networked clone)"
        fi
    fi
    [ -d "$APPSTORE_BUILTIN/database" ] \
        && ok "Bundled AppStore source ready" \
        || warn "Bundled source dir is empty ($APPSTORE_BUILTIN); package will ship no built-in apps, add a source from the panel"
else
    warn "Bundled AppStore source not found ($APPSTORE_BUILTIN); package will ship no built-in apps, add a source from the panel"
fi

# Scripts, data templates and config (data/ packages only what's needed for release, dropping runtime artifacts)
# Placed alongside the binaries, all under the zap/ dir
cp -Rf "$CUR_DIR/scripts" "$DIST_ZAP/"

# data/ packaging allowlist:
#   appstore/repos/appstore/          bundled AppStore seed source
#   appstore/repos.yaml, custom/README.md  templates the install script (install.sh) depends on
#   apps/README.md                    APPS_DIR placeholder note (other files under apps are runtime install instances, not packaged)
#   plugins/_lib/*.lua                 plugin common function library (loaded by zapexec automatically before main.lua)
#   www/                              site skeleton template (data/www/skel) + IP default page / maintenance page (data/www/_zap)
# Note: systemd service templates, ops scripts, zap shared tools and conf templates are all provided by scripts/,
#       and are not duplicated into data/; after install, data/ is the runtime data area (zap.db, apps, appstore, run/, etc.)
# Not packaged: zap.db, run/, tmp/, apps/library, and appstore's cache/logs/runs/tmp/custom/scripts
DIST_DATA="$DIST_ZAP/data"
mkdir -p "$DIST_DATA/apps"
mkdir -p "$DIST_DATA/appstore/repos"
cp -Rf "$CUR_DIR/data/appstore/repos/appstore" "$DIST_DATA/appstore/repos/" 2>/dev/null || true
cp -f "$CUR_DIR/data/appstore/repos.yaml" "$DIST_DATA/appstore/" 2>/dev/null || true
cp -f "$CUR_DIR/data/apps/README.md" "$DIST_DATA/apps/" 2>/dev/null || true

# mlua plugin common library (loaded by zapexec)
mkdir -p "$DIST_DATA/plugins/_lib"
cp -f "$CUR_DIR/data/plugins/_lib/"*.lua "$DIST_DATA/plugins/_lib/" 2>/dev/null || true
cp -f "$CUR_DIR/data/plugins/_lib/"*.css "$DIST_DATA/plugins/_lib/" 2>/dev/null || true
cp -f "$CUR_DIR/data/plugins/_lib/"*.js  "$DIST_DATA/plugins/_lib/" 2>/dev/null || true

# www/: site skeleton template skel/index.html, plus IP default page / maintenance page _zap/*.html (editable by ops)
cp -Rf "$CUR_DIR/data/www" "$DIST_DATA/" 2>/dev/null || true
mkdir -p "$DIST_DATA/www/html"

# Markdown docs (used by zapd for the online docs page)
cp -Rf "$CUR_DIR/CHANGELOG.md"         "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/USER_MANUAL.md"       "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/FAQ.md"               "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/UPGRADE.md"           "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/CHANGELOG_zh-CN.md"   "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/USER_MANUAL_zh-CN.md" "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/FAQ_zh-CN.md"         "$DIST_DATA/www/html/" 2>/dev/null || true
cp -Rf "$CUR_DIR/UPGRADE_zh-CN.md"     "$DIST_DATA/www/html/" 2>/dev/null || true

ok "Resources copied (scripts / data shipped with package)"

# Note: always use absolute paths below; do not `cd dist` -- cargo must stay under the repo-root target/.
# ── Build + package ───────────────────────────────────────────────
build_web
build_variant community
package_variant community ""

# Upload install.sh and uninstall.sh (used by zapd for upgrade downloads)
if command -v zapfile >/dev/null 2>&1; then
    info "Uploading install.sh ..."
    zapfile upload zap/ "$CUR_DIR/scripts/install.sh" || die "failed to upload install.sh"
    info "Uploading uninstall.sh ..."
    zapfile upload zap/ "$CUR_DIR/scripts/uninstall.sh" || die "failed to upload uninstall.sh"
else
    warn "zapfile not installed: skipping install.sh / uninstall.sh upload (the panel's online upgrade needs them)"
fi

# ── Upload ────────────────────────────────────────────────────
for pkg in "${PACKAGES[@]}"; do
    info "Uploading ${pkg} ..."
    zapfile upload zap/releases/ "$DIST_DIR/$pkg" || die "upload failed: ${pkg}"
    # sha256 checksum file (compared by zapd on upgrade download; no trailing newline to avoid residue)
    printf '%s' "$(sha256sum "$DIST_DIR/$pkg" | awk '{print $1}')" > "$DIST_DIR/$pkg.sha256"
    info "Uploading checksum ${pkg}.sha256 ..."
    zapfile upload zap/releases/ "$DIST_DIR/$pkg.sha256" || die "failed to upload checksum: ${pkg}"
done

zapfile put "zap/releases/latest.txt" $VERSION || die "failed to update version file"
ok "Upload complete"

# ── Completion summary ────────────────────────────────────────────────
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   ZAP v${VERSION} (${ARCH}) release complete${NC}"
echo -e "${GREEN}========================================${NC}"
echo "  Version: ${VERSION}"
for pkg in "${PACKAGES[@]}"; do
    echo "  Package: ${pkg}"
done
echo "  Install: bash install.sh ${VERSION}"
