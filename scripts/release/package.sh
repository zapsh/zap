#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

PROJECT_NAME="${PROJECT_NAME:-zap-rs}"
# 显式传入的 VERSION 优先（CI 可从根 Cargo.toml 解析后传入，与 build.sh 同源），
# 其次才是 GitHub Actions 注入的 GITHUB_REF_NAME（tag / 分支名）；都没有时兜底 dev。
# 注意：顺序不能反，否则 GITHUB_REF_NAME 会盖掉调用方显式指定的版本号
# （workflow_dispatch 时它等于分支名，会打出 zap-vmain-... 这种包名）。
# 打包处统一用 ${VERSION#v} 去掉前导 v。
VERSION="${VERSION:-${GITHUB_REF_NAME:-dev}}"

usage() {
  cat <<EOF
Usage:
  scripts/release/package.sh package <target> <binary> <outdir>
  scripts/release/package.sh checksum <outdir>
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

  # 只写裸 hex，不带文件名：sha256sum "<路径>" 会把路径（可能是绝对路径或
  # dist/ 前缀）一起写进 .sha256，而面板 / zapupgrade 的校验是「整行 vs 计算值」
  # 比较，带路径必然不匹配（老版本二进制尤其如此）。新校验端虽然兼容
  # "<hex>  <文件名>" 格式，但发行侧统一裸 hex 最稳。
  find "$outdir" -type f -name "*.tar.gz" -print | sort | while read -r file; do
    sha256sum "$file" | awk '{print $1}' > "${file}.sha256"
  done
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
  # 下面 tar 前会 cd 到 "$outdir/.pkg"，此时相对路径的 "$outdir/$name" 会被解析成
  # <outdir>/.pkg/<outdir>/<name>（该目录不存在）导致 tar 失败。
  # 故先把 outdir 归一成绝对路径，使调用方传相对路径也能正常出包。
  outdir="$(cd "$outdir" && pwd)"

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
  *)
    usage
    exit 2
    ;;
esac