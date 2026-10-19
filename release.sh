#!/usr/bin/env bash
# ZAP release tool
#
# Orchestrates a release:
#   1. bump [workspace.package].version in Cargo.toml
#   2. git commit + tag v<version> + push
#   3. publish the version to the mirrors via `zapfile put zap/releases/latest.txt`
#
# Actual binary build & packaging is done by GitHub Actions on the pushed tag
# (.github/workflows/release.yml), which also syncs the packages to the mirrors.
#
# Usage:
#   ./release.sh <version> [--no-tag] [--no-latest]
#   <version>   semver, e.g. 1.5.0  (no leading v; the pushed tag is v<version>)
#   --no-tag    only set Cargo version + latest.txt; do NOT git commit/tag/push
#   --no-latest set Cargo version + tag, but skip `zapfile put latest.txt`
#
# Env:
#   RELEASE_BRANCH  branch to release from (default: main)
#   COS_ID/COS_KEY  zapfile credentials (needed for `zapfile put latest.txt`)
if [ -z "${BASH_VERSION:-}" ]; then
    exec bash "$0" "$@"
fi
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { printf "${BLUE}[*]${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*"; }
die()  { printf "${RED}[✗]${NC} %s\n" >&2; exit 1; }

RELEASE_BRANCH="${RELEASE_BRANCH:-main}"
NO_TAG=0
NO_LATEST=0
VERSION=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-tag)    NO_TAG=1; shift ;;
        --no-latest) NO_LATEST=1; shift ;;
        -h|--help)
            grep -E '^#' "$0" | sed 's/^#\{1,2\} //' | sed '/^Usage:/,$!d'
            exit 0 ;;
        -*) die "unknown option: $1" ;;
        *)
            [ -z "$VERSION" ] || die "version already given: $VERSION"
            VERSION="$1"; shift ;;
    esac
done

[ -n "$VERSION" ] || die "version required, e.g. ./release.sh 1.5.0"

# semver X.Y.Z
case "$VERSION" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) die "invalid version: $VERSION (expect X.Y.Z, e.g. 1.5.0)" ;;
esac

# ── git sanity ────────────────────────────────────────────────
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "not in a git repo"
CUR_BRANCH=$(git rev-parse --abbrev-ref HEAD)
[ "$CUR_BRANCH" = "$RELEASE_BRANCH" ] \
    || die "must be on branch '$RELEASE_BRANCH' (current: $CUR_BRANCH); set RELEASE_BRANCH=... to override"
git diff HEAD --quiet \
    || die "working tree has tracked changes; commit or stash first"
if git rev-parse "v$VERSION" >/dev/null 2>&1; then
    die "tag v$VERSION already exists"
fi
git fetch -q origin "$RELEASE_BRANCH" 2>/dev/null || true
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse "origin/$RELEASE_BRANCH" 2>/dev/null || true)
[ "$LOCAL" = "$REMOTE" ] \
    || die "local $RELEASE_BRANCH is not in sync with origin/$RELEASE_BRANCH; pull first"

# ── zapfile (for latest.txt) ──────────────────────────────────
if ! command -v zapfile >/dev/null 2>&1; then
    info "zapfile not found, installing from mirror..."
    if command -v wget >/dev/null 2>&1; then
        wget -qO- https://mirrors.zap.cn/zapfile/zapfile-linux-amd64 -O /usr/bin/zapfile \
            || wget -qO- https://mirrors.zap.sh/zapfile/zapfile-linux-amd64 -O /usr/bin/zapfile \
            || die "zapfile install failed"
    elif command -v curl >/dev/null 2>&1; then
        curl -fsSL https://mirrors.zap.cn/zapfile/zapfile-linux-amd64 -o /usr/bin/zapfile \
            || curl -fsSL https://mirrors.zap.sh/zapfile/zapfile-linux-amd64 -o /usr/bin/zapfile \
            || die "zapfile install failed"
    else
        die "zapfile not found and neither wget nor curl available"
    fi
    chmod +x /usr/bin/zapfile
    ok "zapfile installed"
fi
if [ "$NO_LATEST" -ne 1 ]; then
    [ -n "${COS_ID:-}" ]  || die "COS_ID not set (needed for: zapfile put latest.txt)"
    [ -n "${COS_KEY:-}" ] || die "COS_KEY not set (needed for: zapfile put latest.txt)"
fi

# ── 1. set Cargo workspace version ────────────────────────────
set_cargo_version() {
    local ver="$1" tmp
    tmp=$(mktemp)
    awk -v v="$ver" '
        /^\[workspace\.package\]/ { in_wp=1 }
        in_wp && /^version[[:space:]]*=/ { print "version = \"" v "\""; in_wp=0; next }
        { print }
    ' Cargo.toml > "$tmp" && mv "$tmp" Cargo.toml
}
set_cargo_version "$VERSION"
info "Cargo.toml [workspace.package].version -> $VERSION"
grep -q "^version = \"$VERSION\"\$" Cargo.toml \
    || die "failed to set Cargo.toml version (check [workspace.package])"

# ── 2. commit + tag + push ───────────────────────────────────
if [ "$NO_TAG" -ne 1 ]; then
    git add Cargo.toml
    git commit -q -m "chore(release): v$VERSION"
    git tag "v$VERSION"
    git push origin "$RELEASE_BRANCH" >/dev/null
    git push origin "v$VERSION" >/dev/null
    ok "tagged & pushed v$VERSION (GitHub Actions will build & publish the packages)"
else
    warn "skipped git commit/tag/push (--no-tag); Cargo.toml updated locally only"
fi

# ── 3. publish latest.txt to mirrors ─────────────────────────
if [ "$NO_LATEST" -ne 1 ]; then
    info "publishing latest.txt -> $VERSION on mirrors (zap/releases/latest.txt)"
    zapfile put "zap/releases/latest.txt" "$VERSION" \
        || die "zapfile put latest.txt failed"
    ok "mirrors latest.txt = $VERSION"
else
    warn "skipped zapfile put latest.txt (--no-latest)"
fi

echo
printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   ZAP release v${VERSION} prepared${NC}\n"
printf "${GREEN}========================================${NC}\n"
echo "  Version:   $VERSION"
echo "  Tag:       v$VERSION"
[ "$NO_TAG" -eq 1 ] && echo "  Git:       (skipped)"
echo "  Install:   bash install.sh $VERSION   (or: install.sh latest)"
