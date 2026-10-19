#!/usr/bin/env bash
# ZAP release tool
#
# Orchestrates a release:
#   1. bump [workspace.package].version in Cargo.toml
#   2. commit + push (default on a version-bump branch; --no-branch on current branch)
#   3. (--tag only) tag v<version> + push tag + publish latest.txt + install scripts to mirrors
#   4. (--sync only) download GitHub Release assets for <version>, mirror them to COS + Cloudflare
#
# Actual binary build & packaging is done by GitHub Actions on the pushed tag
# (.github/workflows/release.yml) — it only builds & creates the GitHub Release.
# Mirroring packages to COS/Cloudflare is done here (--tag for scripts+latest.txt,
# --sync for the compiled packages once Actions finishes building).
#
# Usage:
#   ./release.sh <version> [--no-branch] [--tag] [--no-latest] [--sync]
#   <version>   semver, e.g. 1.5.1  (no leading v; the pushed tag is v<version>)
#
#   Default (contributor PR flow):
#     bump version -> create branch bump-version-v<ver> -> commit -> push branch
#     (no tag, no latest.txt; open a PR to merge, then run --tag on main)
#   --no-branch (repo owner):
#     bump version -> commit -> push current branch (no new branch, no tag)
#   --tag (repo owner, release):
#     bump version -> commit -> push current branch -> tag v<ver> -> push tag
#     -> zapfile put zap/releases/latest.txt <ver>  (COS + Cloudflare via --aws)
#     -> zapfile upload zap/ scripts/install.sh + uninstall.sh  (COS + Cloudflare via --aws)
#   --sync (standalone, after Actions finishes building):
#     download GitHub Release assets for v<ver> -> upload to COS (mirrors.zap.cn)
#     and Cloudflare (mirrors.zap.sh, --aws) under zap/releases/
#   --no-latest  with --tag, skip the zapfile latest.txt upload
#
# Env:
#   RELEASE_BRANCH  base branch to bump from / tag on (default: main)
#   BUMP_BRANCH     branch name for the default PR flow (default: bump-version-v<ver>)
#   GITHUB_REPO     owner/repo for --sync download (default: derived from git remote, else zapsh/zap)
#   COS_ID/COS_KEY  zapfile credentials (needed for --tag / --sync mirror upload)
# __END_USAGE__
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
NO_BRANCH=0
WITH_TAG=0
NO_LATEST=0
SYNC=0
VERSION=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-branch) NO_BRANCH=1; shift ;;
        --tag)       WITH_TAG=1; NO_BRANCH=1; shift ;;
        --no-latest) NO_LATEST=1; shift ;;
        --sync)      SYNC=1; shift ;;
        -h|--help)
            sed -n '/^# Usage:/,/^# __END_USAGE__/p' "$0" | sed 's/^#* *//' | sed '/^__END_USAGE__/d'
            exit 0 ;;
        -*) die "unknown option: $1" ;;
        *)
            [ -z "$VERSION" ] || die "version already given: $VERSION"
            VERSION="$1"; shift ;;
    esac
done

[ -n "$VERSION" ] || die "version required, e.g. ./release.sh 1.5.0"

# --sync is standalone: it only mirrors GitHub Release assets, no bump/tag/commit
if [ "$SYNC" -eq 1 ] && [ "$WITH_TAG" -eq 1 ]; then
    die "--sync is standalone and cannot be combined with --tag"
fi

# semver X.Y.Z
case "$VERSION" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) die "invalid version: $VERSION (expect X.Y.Z, e.g. 1.5.0)" ;;
esac

# ── git sanity (skipped in --sync mode) ──────────────────────
if [ "$SYNC" -ne 1 ]; then
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "not in a git repo"
CUR_BRANCH=$(git rev-parse --abbrev-ref HEAD)
[ "$CUR_BRANCH" = "$RELEASE_BRANCH" ] \
    || die "must be on branch '$RELEASE_BRANCH' (current: $CUR_BRANCH); set RELEASE_BRANCH=... to override"
git diff HEAD --quiet \
    || die "working tree has tracked changes; commit or stash first"
if [ "$WITH_TAG" -eq 1 ] && git rev-parse "v$VERSION" >/dev/null 2>&1; then
    die "tag v$VERSION already exists"
fi
git fetch -q origin "$RELEASE_BRANCH" 2>/dev/null || true
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse "origin/$RELEASE_BRANCH" 2>/dev/null || true)
[ "$LOCAL" = "$REMOTE" ] \
    || die "local $RELEASE_BRANCH is not in sync with origin/$RELEASE_BRANCH; pull first"
fi

# ── zapfile (only needed for --tag latest.txt) ───────────────
ensure_zapfile() {
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
}

# ── sync GitHub Release assets to mirrors (--sync) ───────────
sync_github_assets() {
    local ver="$1"
    local tag="v$ver" tmp repo url
    repo="${GITHUB_REPO:-}"
    if [ -z "$repo" ]; then
        url=$(git remote get-url origin 2>/dev/null || true)
        if [ -n "$url" ]; then
            url="${url//:/\/}"          # git@host:owner/repo -> git@host/owner/repo
            url="${url%.git}"           # strip trailing .git
            repo="${url##*/}"           # repo name
            url="${url%/*}"             # drop repo name -> .../owner
            repo="${url##*/}/${repo}"   # owner/repo
        fi
    fi
    [ -n "$repo" ] || repo="zapsh/zap"
    tmp=$(mktemp -d)
    info "downloading GitHub Release assets for $tag (repo: $repo) ..."
    if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
        gh release download "$tag" --repo "$repo" --dir "$tmp" \
            || die "gh release download $tag failed (need 'gh auth login' or set GH_TOKEN)"
    else
        local api="https://api.github.com/repos/$repo/releases/tags/$tag"
        local urls
        urls=$(curl -fsSL "$api" | grep -o '"browser_download_url": *"[^"]*"' | sed 's/.*"browser_download_url": *"//; s/"$//')
        [ -n "$urls" ] || die "no assets found for $tag (or GitHub API unreachable)"
        ( cd "$tmp" && for u in $urls; do
            info "  get $(basename "$u")"
            curl -fsSL -O "$u" || die "download failed: $u"
        done )
    fi
    ensure_zapfile
    [ -n "${COS_ID:-}" ]  || die "COS_ID not set (needed for: zapfile mirror upload)"
    [ -n "${COS_KEY:-}" ] || die "COS_KEY not set (needed for: zapfile mirror upload)"
    local f n
    for f in "$tmp"/*; do
        [ -f "$f" ] || continue
        n=$(basename "$f")
        info "uploading $n -> COS mirror (zap/releases/)"
        zapfile upload zap/releases/ "$f" \
            || die "zapfile upload $n (COS) failed"
        info "uploading $n -> Cloudflare mirror (zap/releases/, --aws)"
        zapfile upload zap/releases/ "$f" --aws \
            || die "zapfile upload $n (Cloudflare, --aws) failed"
    done
    ok "synced GitHub Release $tag -> COS + Cloudflare ($(ls -1 "$tmp" | wc -l) files)"
    rm -rf "$tmp"
}

# ── --sync: download from GitHub, mirror to COS + Cloudflare ──
if [ "$SYNC" -eq 1 ]; then
    sync_github_assets "$VERSION"
    echo
    printf "${GREEN}========================================${NC}\n"
    printf "${GREEN}   ZAP mirror sync v${VERSION} done${NC}\n"
    printf "${GREEN}========================================${NC}\n"
    echo "  Source:    GitHub Release v$VERSION"
    echo "  Target:    COS (mirrors.zap.cn) + Cloudflare (mirrors.zap.sh)"
    exit 0
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

# ── 2. commit + push (branch / no-branch) ──────────────────
if git diff --quiet Cargo.toml; then
    info "Cargo.toml already at $VERSION (no commit needed)"
else
    if [ "$NO_BRANCH" -ne 1 ]; then
        # default contributor PR flow: bump on a dedicated branch
        BUMP_BRANCH_NAME="${BUMP_BRANCH:-bump-version-v$VERSION}"
        info "creating version-bump branch: $BUMP_BRANCH_NAME"
        git checkout -b "$BUMP_BRANCH_NAME"
    fi
    git add Cargo.toml
    git commit -q -m "chore(release): v$VERSION"
    git push origin "$(git rev-parse --abbrev-ref HEAD)" >/dev/null
    if [ "$NO_BRANCH" -ne 1 ]; then
        ok "version set, committed & pushed to branch $BUMP_BRANCH_NAME (open a PR into $RELEASE_BRANCH)"
    else
        ok "version set, committed & pushed to $(git rev-parse --abbrev-ref HEAD)"
    fi
fi

# ── 3. tag + publish to mirrors (release mode, --tag) ───────
if [ "$WITH_TAG" -eq 1 ]; then
    git tag "v$VERSION"
    git push origin "v$VERSION" >/dev/null
    ok "tagged & pushed v$VERSION (GitHub Actions will build & publish)"
    ensure_zapfile
    [ -n "${COS_ID:-}" ]  || die "COS_ID not set (needed for: zapfile mirror upload)"
    [ -n "${COS_KEY:-}" ] || die "COS_KEY not set (needed for: zapfile mirror upload)"
    if [ "$NO_LATEST" -ne 1 ]; then
        info "publishing latest.txt -> $VERSION on COS mirror (mirrors.zap.cn)"
        zapfile put "zap/releases/latest.txt" "$VERSION" \
            || die "zapfile put latest.txt (COS) failed"
        info "publishing latest.txt -> $VERSION on Cloudflare mirror (mirrors.zap.sh, --aws)"
        zapfile put "zap/releases/latest.txt" "$VERSION" --aws \
            || die "zapfile put latest.txt (Cloudflare, --aws) failed"
        ok "mirrors latest.txt = $VERSION (COS + Cloudflare)"
    else
        warn "skipped zapfile put latest.txt (--no-latest)"
    fi
    for s in scripts/install.sh scripts/uninstall.sh; do
        [ -f "$s" ] || die "missing $s"
        info "uploading $s -> COS mirror (zap/)"
        zapfile upload zap/ "$s" \
            || die "zapfile upload $s (COS) failed"
        info "uploading $s -> Cloudflare mirror (zap/, --aws)"
        zapfile upload zap/ "$s" --aws \
            || die "zapfile upload $s (Cloudflare, --aws) failed"
    done
    ok "install scripts uploaded to COS + Cloudflare (zap/install.sh, zap/uninstall.sh)"
fi

echo
printf "${GREEN}========================================${NC}\n"
printf "${GREEN}   ZAP release v${VERSION} prepared${NC}\n"
printf "${GREEN}========================================${NC}\n"
echo "  Version:   $VERSION"
if [ "$WITH_TAG" -eq 1 ]; then
    echo "  Tag:       v$VERSION (pushed)"
    echo "  latest:    zapfile put latest.txt = $VERSION (COS + Cloudflare --aws)"
    echo "  scripts:   zapfile upload install.sh/uninstall.sh (COS + Cloudflare --aws)"
else
    echo "  Tag:       (none; run with --tag to release)"
    if [ "$NO_BRANCH" -ne 1 ]; then
        echo "  Branch:    bump-version-v$VERSION (open PR to merge)"
    else
        echo "  Branch:    $RELEASE_BRANCH (pushed, no tag)"
    fi
fi
echo "  Install:   bash install.sh $VERSION   (or: install.sh latest)"
