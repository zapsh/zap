#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

PROJECT_NAME="${PROJECT_NAME:-zap-rs}"
VERSION="${GITHUB_REF_NAME:-${VERSION:-dev}}"

usage() {
  cat <<EOF
Usage:
  scripts/release/package.sh package <target> <binary> <outdir>
  scripts/release/package.sh checksum <outdir>
  scripts/release/package.sh sign <outdir>
EOF
}

package_artifact() {
  local target="$1"
  local bin="$2"
  local outdir="$3"

  [[ -f "$bin" ]] || { echo "error: binary not found: $bin"; exit 1; }

  mkdir -p "$outdir"
  local pkg_dir="${outdir}/${PROJECT_NAME}-${VERSION}-${target}"
  rm -rf "$pkg_dir"
  mkdir -p "$pkg_dir"

  cp "$bin" "$pkg_dir/${PROJECT_NAME}"

  # 给释放包追加说明文件（可选）
  if [[ -f "README.md" ]]; then
    cp README.md "$pkg_dir/"
  fi

  if [[ -f "LICENSE" ]]; then
    cp LICENSE "$pkg_dir/"
  fi

  tar -czf "${outdir}/${PROJECT_NAME}-${VERSION}-${target}.tar.gz" -C "$outdir" "${PROJECT_NAME}-${VERSION}-${target}"
  rm -rf "$pkg_dir"
}

checksum_artifacts() {
  local outdir="$1"
  mkdir -p "$outdir"

  find "$outdir" -type f -name "*.tar.gz" -print | sort | while read -r file; do
    sha256sum "$file" > "${file}.sha256"
  done
}

sign_checksums() {
  local outdir="$1"
  mkdir -p "$outdir"

  # 仅 SSH 签名：用 ssh-keygen -Y sign 对校验和文件签名（与 git SSH 签名同一套）。
  # 未配置 SSH_PRIVATE_KEY 时跳过。
  if [[ -z "${SSH_PRIVATE_KEY:-}" ]]; then
    echo "info: SSH_PRIVATE_KEY not set; skip checksum signing."
    return 0
  fi

  sign_ssh "$outdir"
}

# SSH 签名：对每个 *.sha256 生成 *.sha256.sig（armored 内部格式，无需 .asc）
# 验证方需准备 allowed_signers 文件，例：
#   echo "release@example.com $(cat ssh_public_key.pub)" > allowed_signers
#   ssh-keygen -Y verify -f allowed_signers -I release@example.com -n file <file>.sha256 < <file>.sha256.sig
sign_ssh() {
  local outdir="$1"

  local keyfile
  keyfile="$(mktemp)"
  chmod 600 "$keyfile"
  printf '%s\n' "$SSH_PRIVATE_KEY" > "$keyfile"

  local extra=()
  if [[ -n "${SSH_KEY_PASSPHRASE:-}" ]]; then
    extra=(-P "$SSH_KEY_PASSPHRASE")
  fi

  find "$outdir" -type f -name "*.sha256" -print | sort | while read -r file; do
    if ssh-keygen -Y sign -f "$keyfile" "${extra[@]}" "$file" >/dev/null 2>&1; then
      echo "signed (ssh): ${file}.sig"
    else
      echo "error: ssh signing failed: $file" >&2
      rm -f "$keyfile"
      return 1
    fi
  done

  rm -f "$keyfile"
}

# 完整发布包（与 build.sh 的 package_variant 一致）：
#   zap/{zapd, zapctl, zapexec, zapupgrade} + zap/scripts + zap/data
# 打包名：zap-v<version>-linux-<arch>.tar.gz（arch 由 CI 矩阵传入 amd64/arm64）
package_full() {
  local arch="$1" target="$2" bin_dir="$3" outdir="$4"
  [[ -d "$bin_dir" ]] || { echo "error: bin dir not found: $bin_dir"; exit 1; }

  local version="${VERSION#v}"
  [[ -n "$version" ]] || version="dev"

  mkdir -p "$outdir"
  local zap_dir="${outdir}/.pkg/zap"
  rm -rf "$zap_dir"
  mkdir -p "$zap_dir"

  local bins=(zapd zapctl zapexec zapupgrade)
  for b in "${bins[@]}"; do
    local src="$bin_dir/$b"
    [[ -f "$src" ]] || { echo "error: binary not found: $src"; exit 1; }
    cp -f "$src" "$zap_dir/$b"
  done

  # 脚本与资源（与 build.sh 同一套）
  cp -Rf scripts "$zap_dir/" 2>/dev/null || true

  local data_dir="$zap_dir/data"
  mkdir -p "$data_dir/apps" "$data_dir/appstore/repos" "$data_dir/www/html"
  cp -Rf data/appstore/repos/appstore "$data_dir/appstore/repos/" 2>/dev/null || true
  cp -f data/appstore/repos.yaml "$data_dir/appstore/" 2>/dev/null || true
  cp -f data/apps/README.md "$data_dir/apps/" 2>/dev/null || true
  # 插件公共库 + UIKit（*.lua 是公共函数库，*.css / *.js 注入插件界面）
  mkdir -p "$data_dir/plugins/_lib"
  cp -f data/plugins/_lib/*.lua "$data_dir/plugins/_lib/" 2>/dev/null || true
  cp -f data/plugins/_lib/*.css "$data_dir/plugins/_lib/" 2>/dev/null || true
  cp -f data/plugins/_lib/*.js  "$data_dir/plugins/_lib/" 2>/dev/null || true
  cp -Rf data/www "$data_dir/" 2>/dev/null || true
  for f in CHANGELOG.md USER_MANUAL.md FAQ.md UPGRADE.md CHANGELOG_zh-CN.md USER_MANUAL_zh-CN.md FAQ_zh-CN.md UPGRADE_zh-CN.md; do
    cp -f "$f" "$data_dir/www/html/" 2>/dev/null || true
  done
  find "$data_dir/www/html" -type f -name '*:Zone.Identifier' -delete 2>/dev/null || true

  local name="zap-v${version}-linux-${arch}.tar.gz"
  ( cd "$outdir/.pkg" && tar -czf "$outdir/$name" zap ) || { echo "error: tar failed: $name"; exit 1; }
  rm -rf "$outdir/.pkg"
  echo "packaged: $name"
}

case "${1:-}" in
  package)
    [[ $# -eq 4 ]] || { usage; exit 2; }
    package_artifact "$3" "$4" "$5"
    ;;
  package-full)
    [[ $# -eq 5 ]] || { usage; exit 2; }
    package_full "$2" "$3" "$4" "$5"
    ;;
  checksum)
    [[ $# -eq 2 ]] || { usage; exit 2; }
    checksum_artifacts "$2"
    ;;
  sign)
    [[ $# -eq 2 ]] || { usage; exit 2; }
    sign_checksums "$2"
    ;;
  *)
    usage
    exit 2
    ;;
esac