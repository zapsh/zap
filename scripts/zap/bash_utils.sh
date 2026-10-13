#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#=============================================================================
# bash_utils.sh — ZAP 应用商店脚本公共函数库
#
# usage:
#     source "${ZAP_PATH}/scripts/zap/bash_utils.sh"
#
#   * Executed by zapexec, and inject these variables:
#       ZAP_PATH    /usr/local/zap
#       ZAPCTL      /usr/local/zap/zapctl
#       APPS_DIR 
#       PKG_PATH 
#       APP_ID 
#       APP_NAME 
#       APP_PATH
#       BUILD_PATH ZAP_DATA_PATH APP_VERSION MAJOR_VERSION MINOR_VERSION
#       APP_OLD_VERSION CPU_NUM ACTION
#
# functions:
#   assert_root 
#   ensure_dir 
#   ensure_user 
#   ensure_group 
#   ensure_usergroup
#   log_info/log_ok/log_warn/log_error
#   os_detect is_os is_os_strict is_os_ge os_version_major os_version_ge os_version_lt
#   is_deb_family is_rpm_family pkg_manager normalize_arch cpu_count
#   fetch_file download_file http_fetch download_extract extract_archive
#   make_bin MakeInstall version_compare version_ge version_lt version_gt
#   random_password has_git getPropsValue wzap_conf
#   have_lib lib_path link_lib_compat(运行时库探测 / 兼容 soname 软链)
#   pkg_install_any(多候选包名安装)
#   app_install_complete path_under remove_path service_stop_disable
#   db_data_initialized(安装残局处理 / 失败重跑)
#   prepare_install_env(汇总:运行用户/关键目录/首次系统编译依赖)
#
# OS DETECH:
#   OS_NAME(发行版小写 ID) OS_VERSION(版本号) OS_PRETTY OS_ID_LIKE
#   OS_MACHINE(uname -s) OS_ARCH(uname -m) OS_ARCH_ALIAS(amd64/arm64/...)
#=============================================================================

if [ "${BASH_SOURCE[0]:-}" = "$0" ]; then
  echo "bash_utils.sh It is a function library, please do not run it directly." >&2
  echo "Used in appstore scripts: source \"\${ZAP_PATH}/scripts/zap/bash_utils.sh\"" >&2
  exit 0
fi

# ── log ──────────────────────────────────────────────────────────────────
# log to run-<run_id>.log
_log_ts() { date '+%Y-%m-%d %H:%M:%S'; }
log_info()  { printf '[%s] [ INFO ] %s\n' "$(_log_ts)" "$*"; }
log_ok()    { printf '[%s] [  OK  ] %s\n' "$(_log_ts)" "$*"; }
log_warn()  { printf '[%s] [ WARN ] %s\n' "$(_log_ts)" "$*" >&2; }
log_error() { printf '[%s] [ERROR ] %s\n' "$(_log_ts)" "$*" >&2; }

# ── utils ──────────────────────────────────────────────────────────────
# Must be run as root
assert_root() {
  if [ "$(id -u)" -ne 0 ]; then
    log_error "Must be run as root (current uid=$(id -u))"
    return 1
  fi
}

# ensure dir
# ensure_dir /www /web
ensure_dir() {
  local d
  for d in "$@"; do
    if [ -n "$d" ] && [ ! -d "$d" ]; then
      mkdir -p "$d" || { log_error "Failed to create directory: $d"; return 1; }
    fi
  done
}

# ── 用户 / 组 ─────────────────────────────────────────────────────────────
# 组是否存在(优先 getent,缺失时回退解析 /etc/group,兼容 Alpine/BusyBox)
_group_exists() {
  local g="${1:-}"
  [ -n "$g" ] || return 1
  if command -v getent >/dev/null 2>&1; then
    getent group "${g}" >/dev/null 2>&1
  else
    grep -q "^${g}:" /etc/group 2>/dev/null
  fi
}

# 用户是否已属于某组(附加组 / 主组均算)
_user_in_group() {
  local u="${1:-}" g="${2:-}"
  [ -n "$u" ] && [ -n "$g" ] || return 1
  id -nG "${u}" 2>/dev/null | tr ' ' '\n' | grep -qx "${g}"
}

# 确保运行组存在(系统组);已存在直接成功
# 用法: ensure_group <group>
ensure_group() {
  local group="${1:-}"
  if [ -z "$group" ]; then
    log_error "ensure_group: 缺少组名"
    return 1
  fi
  if _group_exists "${group}"; then
    return 0
  fi
  if command -v groupadd >/dev/null 2>&1; then
    # -r 系统组;个别发行版不支持时退回普通组
    groupadd -r "${group}" >/dev/null 2>&1 \
      || groupadd "${group}" >/dev/null 2>&1 \
      || { log_error "创建组 ${group} 失败(groupadd)"; return 1; }
  elif command -v addgroup >/dev/null 2>&1; then
    addgroup -S "${group}" >/dev/null 2>&1 \
      || addgroup "${group}" >/dev/null 2>&1 \
      || { log_error "创建组 ${group} 失败(addgroup)"; return 1; }
  else
    log_error "系统缺少 groupadd/addgroup,无法创建组 ${group}"
    return 1
  fi
  log_ok "已创建运行组: ${group}"
}

# Ensure that the user is in this group; if exists, skip
# usage: ensure_usergroup <user> <group> [group2 ...]
ensure_usergroup() {
  local user="${1:-}" group
  if [ -z "$user" ]; then
    log_error "ensure_usergroup: Username is missing"
    return 1
  fi
  shift
  if [ "$#" -eq 0 ]; then
    log_error "ensure_usergroup: group name is required (usage: ensure_usergroup <user> <group> [group2 ...])"
    return 1
  fi
  
  if ! id "${user}" >/dev/null 2>&1; then
    log_error "ensure_usergroup: The user ${user} does not exist (please call ensure_user first)"
    return 1
  fi

  for group in "$@"; do
    [ -n "$group" ] || continue
    ensure_group "${group}" || return 1
    if _user_in_group "${user}" "${group}"; then
      continue
    fi
    if command -v usermod >/dev/null 2>&1; then
      usermod -aG "${group}" "${user}" >/dev/null 2>&1 \
        || { log_error "Failed to add user ${user} to group ${group} (usermod)"; return 1; }
    elif command -v addgroup >/dev/null 2>&1; then
      addgroup "${user}" "${group}" >/dev/null 2>&1 \
        || { log_error "Failed to add user ${user} to group ${group} (addgroup)"; return 1; }
    else
      log_error "Unable to add user ${user} to group ${group}, the usermod/addgroup command does not exist"
      return 1
    fi
    log_ok "${user} has been added to the group: ${group}"
  done
}

# create user (default www)
ensure_user() {
  local user="${1:-www}"
  [ "$#" -gt 0 ] && shift
  if id "${user}" >/dev/null 2>&1; then
    if [ "$#" -gt 0 ]; then
      ensure_usergroup "${user}" "$@" || return 1
    fi
    return 0
  fi
  if command -v useradd >/dev/null 2>&1; then
    useradd -r -s /sbin/nologin -M "${user}" >/dev/null 2>&1 \
      || useradd --no-create-home --shell /bin/false "${user}" >/dev/null 2>&1 \
      || { log_error "Failed to create user ${user} (useradd)"; return 1; }
  elif command -v adduser >/dev/null 2>&1; then
    adduser -S -D -H -s /sbin/nologin "${user}" >/dev/null 2>&1 \
      || { log_error "Failed to create user ${user} (adduser)"; return 1; }
  else
    log_error "useradd/adduser Command not found, unable to create user ${user}"
    return 1
  fi
  log_ok "user has been created: ${user}"
  if [ "$#" -gt 0 ]; then
    ensure_usergroup "${user}" "$@" || return 1
  fi
}

# ── os and arch detech ───────────────────────────────────────────────────────
# x86_64 -> amd64;aarch64 -> arm64;x86 -> i386;其余原样
normalize_arch() {
  case "${1:-$(uname -m)}" in
    x86_64 | amd64) echo "amd64" ;;
    aarch64 | arm64) echo "arm64" ;;
    i?86 | x86) echo "i386" ;;
    *) echo "${1:-unknown}" ;;
  esac
}

# detech os arch
os_detect() {
  local id="" version_id="" pretty="" id_like=""
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    id="${ID:-linux}"; version_id="${VERSION_ID:-}"; pretty="${PRETTY_NAME:-}"; id_like="${ID_LIKE:-}"
  else
    id="linux"
  fi
  OS_NAME="$(printf '%s' "${id}" | tr '[:upper:]' '[:lower:]')"
  OS_VERSION="${version_id}"
  OS_PRETTY="${pretty}"
  OS_ID_LIKE="${id_like}"
  OS_MACHINE="$(uname -s 2>/dev/null || echo Linux)"
  OS_ARCH="$(uname -m 2>/dev/null || echo unknown)"
  OS_ARCH_ALIAS="$(normalize_arch "${OS_ARCH}")"
}

# check os:is_os ubuntu debian / is_os centos rocky alma ...
is_os() {
  local id
  for id in "$@"; do
    [ "${OS_NAME:-}" = "$id" ] && return 0
    case " ${OS_ID_LIKE:-} " in *" ${id} "*) return 0 ;; esac
  done
  return 1
}

# ── 发行版家族 / 版本判断(基于 os_detect 填充的 OS_NAME / OS_VERSION) ──────
# OS_VERSION 主版本号:24.04 -> 24 ; 7.9 -> 7 ; 12 -> 12
os_version_major() { version_field "${OS_VERSION:-0}" 1; }
# 与当前系统版本比较:os_version_ge 24.04 / os_version_lt 22.04
os_version_ge() { version_ge "${OS_VERSION:-0}" "${1:-0}"; }
os_version_lt() { version_lt "${OS_VERSION:-0}" "${1:-0}"; }
# 严格按 ID 匹配(不匹配 ID_LIKE):is_os_strict debian
# 例:Ubuntu 的 ID_LIKE 含 debian,is_os debian 为真,而 is_os_strict debian 为假
is_os_strict() { [ -n "${1:-}" ] && [ "${OS_NAME:-}" = "$1" ]; }
# 发行版 + 版本下限:is_os_ge ubuntu 24.04(同时满足才是 0)
# 沿用 is_os 的语义(匹配 ID 与 ID_LIKE),因此 Ubuntu 24.04 上 is_os_ge debian 13 也为真
# —— 对「包名改名」这类判断通常正是想要的结果;只认 ID 请用 is_os_strict + os_version_ge。
# 典型用途:Ubuntu 24.04+ / Debian 13+ 的 libaio1 改名 libaio1t64、libncurses5 下线
is_os_ge() {
  local id="${1:-}" ver="${2:-0}"
  [ -n "$id" ] || return 1
  is_os "$id" || return 1
  os_version_ge "$ver"
}
is_deb_family() { is_os ubuntu debian raspbian linuxmint pop neon kali deepin; }
is_rpm_family() { is_os centos rhel rocky alma almalinux ol oracle amazon fedora opensuse sles suse; }
# system pkg:apt / dnf / yum / apk / zypper,Unrecognized, returning 1
pkg_manager() {
  if command -v apt-get >/dev/null 2>&1; then echo apt
  elif command -v dnf >/dev/null 2>&1; then echo dnf
  elif command -v yum >/dev/null 2>&1; then echo yum
  elif command -v apk >/dev/null 2>&1; then echo apk
  elif command -v zypper >/dev/null 2>&1; then echo zypper
  else return 1
  fi
}

# Number of available CPU cores (env CPU_NUM)
cpu_count() {
  local n
  if command -v nproc >/dev/null 2>&1; then
    n="$(nproc 2>/dev/null || echo 1)"
  elif [ -r /proc/cpuinfo ]; then
    n="$(grep -c '^processor' /proc/cpuinfo 2>/dev/null || echo 1)"
  else
    n=1
  fi
  case "$n" in '' | 0 | *[!0-9]*) n=1 ;; esac
  if [ -n "${CPU_NUM:-}" ] && [ "$n" -gt "$CPU_NUM" ] 2>/dev/null; then
    n="$CPU_NUM"
  fi
  printf '%s' "$n"
}

# ── download ──────────────────────────────────────────────────────────────────
# zap pkg mirror
# 1. ZAP_PKG_MIRROR 
# 2. /etc/zap/mirror.conf
# 3. zap mirror
# 4. file:///opt/zap-pkg
pkg_mirror() {
  local base="${ZAP_PKG_MIRROR:-https://mirrors.zap.cn/pkg}"
  printf '%s' "${base}" | sed 's:/*$::'
}

# download cache (default) : $ZAP_PATH/data/appstore/cache，
# ZAP_PKG_CACHE override
_pkg_cache_dir() {
  printf '%s' "${ZAP_PKG_CACHE:-$ZAP_PATH/data/appstore/cache}"
}

# cache path：<sha256 first 16 bit>_<URL base name>
_pkg_cache_path() {
  local url="$1" base key
  base="$(basename "${url%%\?*}")"
  key="$(printf '%s' "$url" | sha256sum | cut -c1-16)"
  printf '%s/%s_%s' "$(_pkg_cache_dir)" "$key" "$base"
}

# fetch_file
# curl > wget. auto retry.   success : 0
# progress bar: curl --progress-bar / wget --show-progress. force in non-tty(log file) with `\r` 
# local mirror (/path or file:///path) cp -f to dest.
# For remote sources, first check the download cache, and if it is hit, skip the download.
fetch_file() {
  # fetch_file <url> <dest> [retry,default 3]
  local url="$1" dest="$2" retries="${3:-3}" i=0
  case "$url" in
    /*|file://*)
      local src="${url#file://}"
      if [ -f "$src" ] && cp -f "$src" "$dest"; then
        log_info "local mirror: ${src}"
        return 0
      fi
      log_error "Failed to retrieve the package from the local source (file does not exist or is unreadable): ${src}"
      return 1
      ;;
  esac

  # Remote source: First check the download cache. If it's found, reuse it; otherwise, do not download it again
  local cache_dir cachefile
  cache_dir="$(_pkg_cache_dir)"
  if [ -n "$cache_dir" ]; then
    ensure_dir "$cache_dir"
    cachefile="$(_pkg_cache_path "$url")"
    if [ -f "$cachefile" ]; then
      log_info "If the download is cached, skip: ${cachefile}"
      cp -f "$cachefile" "$dest" && return 0
    fi
  fi

  if command -v curl >/dev/null 2>&1; then
    while [ "$i" -lt "$retries" ]; do
      if curl -fL --progress-bar --connect-timeout 15 -4 -o "$dest" "$url"; then
        [ -n "$cachefile" ] && cp -f "$dest" "$cachefile"
        return 0
      fi
      i=$((i + 1)); [ "$i" -lt "$retries" ] && sleep 1
    done
  elif command -v wget >/dev/null 2>&1; then
    while [ "$i" -lt "$retries" ]; do
      if wget -q --show-progress -4 --timeout=60 --tries=2 -O "$dest" "$url"; then
        [ -n "$cachefile" ] && cp -f "$dest" "$cachefile"
        return 0
      fi
      i=$((i + 1)); [ "$i" -lt "$retries" ] && sleep 1
    done
  else
    log_error "系统缺少 curl / wget,无法下载: ${url}"
    return 1
  fi
  log_error "下载失败(已重试 ${retries} 次): ${url}"
  return 1
}

# download_file <url> <dest>:下载并打印结果;失败以退出码 1 中止脚本
download_file() {
  local url="$1" dest="$2"
  if fetch_file "$url" "$dest"; then
    log_info "download success: ${dest}"
    return 0
  fi
  log_error "Download failed, aborted: ${url}"
  exit 1
}

# http_fetch <url> <dest> 
# Abort if failed
http_fetch() {
  local url="$1" dest="$2"
  fetch_file "$url" "$dest" || {
    log_error "Failed to fetch ${url}. Aborting install."
    exit 1
  }
}

# Extract tar.*/zip files to the target directory (automatically recognizing the format; dependent on the system tar/unzip)
# extract_archive <archive file> [destination directory, default current directory]
extract_archive() {
  local archive="$1" dest="${2:-.}" ok=0
  ensure_dir "$dest" || return 1
  case "$archive" in
    *.tar.gz | *.tgz)         tar -xzf "$archive" -C "$dest" && ok=1 ;;
    *.tar.xz)                 tar -xJf "$archive" -C "$dest" && ok=1 ;;
    *.tar.bz2 | *.tbz2 | *.tb2) tar -xjf "$archive" -C "$dest" && ok=1 ;;
    *.tar)                    tar -xf "$archive" -C "$dest" && ok=1 ;;
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -qo "$archive" -d "$dest" && ok=1
      else
        log_error "Missing unzip, unable to decompress: ${archive}"
      fi ;;
    *) log_error "Unsupported file format: ${archive}" ;;
  esac
  [ "$ok" -eq 1 ] && return 0
  log_error "Decompression failed: ${archive}"
  return 1
}

# download_extract <url> <local_archive_name> <target_directory>: Download and extract in one step
download_extract() {
  local url="$1" name="$2" dir="$3"
  fetch_file "$url" "$name" || { log_error "download failed: ${url}"; return 1; }
  extract_archive "$name" "$dir"
}

# compile
# ZAP_MAKE > gmake > make
make_bin() {
  local m first="" ver
  for m in "${ZAP_MAKE:-}" gmake make; do
    [ -n "$m" ] || continue
    command -v "$m" >/dev/null 2>&1 || continue
    [ -n "$first" ] || first="$m"
    ver="$("$m" --version 2>/dev/null || printf '')"
    case "$ver" in *"GNU Make"*) printf '%s\n' "$m"; return 0 ;; esac
  done
  [ -n "$first" ] || { log_error "make / gmake not found , can't compile"; return 1; }
  log_warn "GNU make not found , use ${first}"
  printf '%s\n' "$first"
}

# cpu number :  CPU_NUM -> cpu_count
# parallel make : make -j${jobs}
# make install
MakeInstall() {
  local jobs="${1:-}" mb
  [ -n "$jobs" ] || jobs="${CPU_NUM:-}"
  [ -n "$jobs" ] || jobs="$(cpu_count)"
  mb="$(make_bin)" || return 1
  if "${mb}" -j "${jobs}"; then
    "${mb}" install
  else
    log_warn "parallel ${mb} -j${jobs} failed, use ${mb} install"
    "${mb}"
    "${mb}" install
  fi
}

# version compare (1.24.0p1 => 1.24.0):
# version_compare <a> <b> -> 0 equal / 1 a>b  / 2 a<b
version_compare() {
  local va vb i n ai bi
  IFS='.' read -ra va <<< "$(printf '%s' "${1:-}" | sed 's/[^0-9.]//g')"
  IFS='.' read -ra vb <<< "$(printf '%s' "${2:-}" | sed 's/[^0-9.]//g')"
  n="${#va[@]}"; [ "${#vb[@]}" -gt "$n" ] && n="${#vb[@]}"
  for ((i = 0; i < n; i++)); do
    ai="${va[$i]:-0}"; bi="${vb[$i]:-0}"
    [ "$ai" -gt "$bi" ] 2>/dev/null && return 1
    [ "$ai" -lt "$bi" ] 2>/dev/null && return 2
  done
  [ "${#va[@]}" -gt "${#vb[@]}" ] && return 1
  [ "${#va[@]}" -lt "${#vb[@]}" ] && return 2
  return 0
}
version_ge() { version_compare "$1" "$2"; [ $? -ne 2 ]; }
version_gt() { version_compare "$1" "$2"; [ $? -eq 1 ]; }
version_lt() { version_compare "$1" "$2"; [ $? -eq 2 ]; }

# ── Version Segmentation Analysis (ignoring suffixes of letters in the same way as version_compare) ────────────────────
# Extract the Nth segment of the version number (starting from 1), automatically removing the suffixes of letters: 1.1.1w → The 1st segment = 1, the 2nd segment = 1, the 3rd segment = 1
# Usage: version_field <version> <segment number>
version_field() {
  local ver="${1:-}" idx="${2:-1}" parts v
  [ -n "$ver" ] || return 1
  case "$idx" in '' | *[!0-9]*) return 1 ;; esac
  [ "$idx" -ge 1 ] || return 1
  IFS='.' read -ra parts <<<"$(printf '%s' "$ver" | sed 's/[^0-9.]//g')"
  v="${parts[$((idx - 1))]:-}"
  [ -n "$v" ] || return 1
  printf '%s' "$v"
}

# Major Version Number: version_major 1.1.1w → 1
version_major() { version_field "${1:-}" 1; }

# minor number: version_minor 1.1.1w → 1
version_minor() { version_field "${1:-}" 2; }

# Extract both major and minor versions and output "major minor" (the missing section is an empty string)
# Typical usage: read -r MAJOR MINOR <<< "$(version_major_minor "${APP_OLD_VERSION}")"
version_major_minor() {
  local major minor
  major="$(version_field "${1:-}" 1 || printf '')"
  minor="$(version_field "${1:-}" 2 || printf '')"
  printf '%s %s\n' "$major" "$minor"
}

# Generate a random password (default 16-character alphanumeric)
random_password() {
  local len="${1:-16}"
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -base64 48 2>/dev/null | tr -dc 'A-Za-z0-9' | head -c "$len"
  elif [ -r /dev/urandom ]; then
    tr -dc 'A-Za-z0-9' < /dev/urandom | head -c "$len"
  else
    printf '%s' "$$$(date +%s)" | sha256sum | base64 | head -c "$len"
  fi
  printf '\n'
}

has_git() {
  command -v git >/dev/null 2>&1
}

# ── Attribute file / Configuration reading and writing ─────────────────────────────────────────────────────
# Read the values from the key=value attribute file (allowing leading spaces / inline comment values), return empty if no match is found
getPropsValue() {
  # getPropsValue <文件> <key>
  local file="$1" key="$2" esc v
  [ -f "$file" ] || return 1
  esc="$(printf '%s' "$key" | sed 's/[][\\^$.*]/\\&/g')"
  v="$(grep -m1 "^[[:space:]]*${esc}[[:space:]]*=" "$file" 2>/dev/null | sed 's/^[^=]*=[[:space:]]*//')" || true
  printf '%s' "$v"
}

# ── YAML Key-Value Reading / Directory Security Check (Common for App Stores) ──────────────────────────

# Read the top-level key-value in YAML (only supports the simple form of `key: value`), return empty if no match
# Usage: yaml_value <file> <key>
yaml_value() {
  local file="${1:-}" key="${2:-}" v
  [ -n "$key" ] || return 1
  [ -f "$file" ] || return 1
  v="$(sed -n "s/^[[:space:]]*${key}:[[:space:]]*//p" "$file" | head -1 | sed 's/[[:space:]]*$//')" || true
  printf '%s' "$v"
}

# Normalized Directory: Remove redundant slashes at the end (retain root directory "/", do not resolve symbolic links)
# Usage: normalize_dir <path>; returns non-zero value if the path is empty
normalize_dir() {
  local p="${1:-}"
  [ -n "$p" ] || return 1
  while [ "${p%/}" != "$p" ] && [ "$p" != "/" ]; do p="${p%/}"; done
  printf '%s' "$p"
}

# 确定应用安装目录:info 文件登记优先,软链指向次之;取不到返回空
# 用法: resolve_install_dir <info文件> <软链路径> [键名(默认 install_dir)]
# 例:   resolve_install_dir "${APP_PATH}/info.yaml" "${APPS_DIR}/phpmyadmin"
resolve_install_dir() {
  local info_file="${1:-}" link_dir="${2:-}" key="${3:-install_dir}" dir
  dir="$(yaml_value "$info_file" "$key" || true)"
  if [ -z "$dir" ] && [ -n "$link_dir" ] && [ -L "$link_dir" ]; then
    dir="$(readlink "$link_dir" || true)"
  fi
  normalize_dir "$dir" 2>/dev/null || printf ''
}

# 校验目录可安全删除:必须是 apps_dir 的**直接子目录**
# 卸载前调用,防止误删(拒绝空值 / 根 / apps_dir 自身 / 任何越界路径)
# 用法: assert_under_apps_dir <目标目录> <应用根目录>
assert_under_apps_dir() {
  local target="${1:-}" apps="${2:-${APPS_DIR:-}}" parent
  apps="$(normalize_dir "$apps" || printf '')"
  if [ -z "$apps" ]; then
    log_error "应用根目录无效：${2:-（APPS_DIR 为空）}"
    return 1
  fi

  target="$(normalize_dir "$target" || printf '')"
  if [ -z "$target" ]; then
    log_error "目标目录为空，拒绝执行删除"
    return 1
  fi
  if [ "$target" = "/" ]; then
    log_error "拒绝删除根目录"
    return 1
  fi
  if [ "$target" = "$apps" ]; then
    log_error "拒绝删除应用根目录 ${apps}"
    return 1
  fi

  parent="$(normalize_dir "$(dirname "$target")" || printf '')"
  if [ "$parent" != "$apps" ]; then
    log_error "目录不在 ${apps} 下（实际：${target}），中止以避免误删"
    return 1
  fi
  return 0
}

# 写 /root/zap.conf 的 key=value(默认文件可用 WZAP_CONF_FILE 覆盖)
# key 仅允许 [A-Za-z0-9_];value 原样保存(允许空格与特殊字符)
wzap_conf() {
  local conf_file="${WZAP_CONF_FILE:-/root/zap.conf}" key="$1" val="$2" val_esc
  case "$key" in
    *[!A-Za-z0-9_]*) log_error "wzap_conf: 非法 key '${key}',仅允许字母/数字/下划线"; return 1 ;;
  esac
  ensure_dir "$(dirname "$conf_file")" || return 1
  [ -f "$conf_file" ] || : > "$conf_file" || { log_error "无法创建 ${conf_file}"; return 1; }
  # 值中 sed 替换特殊字符(& 和分隔符 | 与 \)转义
  val_esc="$(printf '%s' "$val" | sed 's/[&|\\]/\\&/g')"
  if grep -Eq "^${key}=" "$conf_file"; then
    sed -i "s|^${key}=.*|${key}=${val_esc}|" "$conf_file"
  else
    # 文件末尾若无换行先补一个,避免拼接
    [ -z "$(tail -c 1 "$conf_file")" ] || printf '\n' >> "$conf_file"
    printf '%s=%s\n' "$key" "$val" >> "$conf_file"
  fi
  log_info "已写入 ${conf_file}: ${key}=${val}"
}

# ── 系统编译依赖(按发行版分组,可用环境变量整体覆盖) ─────────────────────
# 用法: ZAP_UBUNTU_DEPS="..." 自定义后再调用 prepare_install_env / install_system_deps
UBUNTU_DEPS="${ZAP_UBUNTU_DEPS:-wget curl git ca-certificates build-essential gcc g++ autoconf automake libtool bison re2c pkg-config libxml2-dev libssl-dev libsqlite3-dev libcurl4-openssl-dev libpcre3-dev libbz2-dev zlib1g-dev libpq-dev libzip-dev libonig-dev libpng-dev libjpeg-dev libwebp-dev libavif-dev libicu-dev libreadline-dev libffi-dev libxslt1-dev libfreetype6-dev libgd-dev libsodium-dev}"
RH_DEPS="${ZAP_RH_DEPS:-wget curl git make gcc gcc-c++ autoconf automake libtool bison re2c pkgconfig openssl-devel libxml2-devel sqlite-devel libcurl-devel libpcre-devel bzip2-devel zlib-devel ncurses-devel libpng-devel libjpeg-turbo-devel libwebp-devel}"
RH_DNF_EXTRA="${ZAP_RH_DNF_EXTRA:-libzip-devel oniguruma-devel libicu-devel libffi-devel libxslt-devel gd-devel libsodium-devel perl-core}"
ALPINE_DEPS="${ZAP_ALPINE_DEPS:-build-base autoconf automake libtool bison re2c pkgconf curl wget git openssl-dev libxml2-dev zlib-dev ncurses-dev bzip2-dev libpng-dev libjpeg-turbo-dev}"

# ── 运行时库 / 系统包(发行版与版本间包名不同) ──────────────────────
#   libaio    : libaio1(Ubuntu 22.04-/Debian 12-) / libaio1t64(Ubuntu 24.04+/Debian 13+)
#   libncurses: libncurses5(旧) / libncurses6(新) / ncurses-compat-libs(RHEL)

ldconfig_bin() {
  local c
  for c in ldconfig /sbin/ldconfig /usr/sbin/ldconfig /usr/bin/ldconfig; do
    if command -v "$c" >/dev/null 2>&1; then printf '%s\n' "$c"; return 0; fi
  done
  return 1
}

# 动态链接器的默认搜索目录(缓存未收录时按文件系统复核用)
lib_search_dirs() {
  local d
  printf '%s\n' /lib /usr/lib /lib64 /usr/lib64 /usr/local/lib
  for d in /lib/*-linux-gnu /usr/lib/*-linux-gnu /lib/*-linux-musl /usr/lib/*-linux-musl; do
    if [ -d "$d" ]; then printf '%s\n' "$d"; fi
  done
}

# 解析 ldconfig -p 缓存:按 soname 精确 / 通配匹配,命中则把库路径打到标准输出
# 不用 `ldconfig -p | grep -q`:调用方普遍开着 pipefail,grep -q 命中即退出会让
# ldconfig 收到 SIGPIPE(141),管道状态非零 → 明明有库也被判成缺失。
# 解析必须用 read 分词:ldconfig 输出每行以 Tab 开头,${line%% *} 会把 Tab 留在
# soname 里("	libaio.so.1t64"),于是永远匹配不上、库明明在却判成缺失。
_ldconfig_lookup() {
  local pattern="${1:-}" cache name rest
  [ -n "$pattern" ] || return 1
  cache="$("$(ldconfig_bin)" -p 2>/dev/null)" || cache=""
  # 条目形如: libaio.so.1t64 (libc6,x86-64) => /lib/x86_64-linux-gnu/libaio.so.1t64
  while read -r name rest; do
    case "$rest" in *" => "*) ;; *) continue ;; esac
    # shellcheck disable=SC2254
    case "$name" in $pattern) printf '%s\n' "${rest##* => }"; return 0 ;; esac
  done <<<"$cache"
  return 1
}

# 系统里是否已有该库:参数按 soname【精确 / 通配】匹配
#   have_lib libaio.so.1        只认 libaio.so.1,不认 libaio.so.1t64
#   have_lib 'libncurses.so.*'  认 libncurses.so.5 / libncurses.so.6
# 必须精确匹配 soname:ldconfig 缓存里 libaio.so.1t64 也「包含」libaio.so.1 字样,
# 但动态链接器按 soname 精确加载,只有 1t64 时 mysqld 依旧报缺 libaio.so.1。
have_lib() {
  local pattern="${1:-}" dir f
  [ -n "$pattern" ] || return 1
  _ldconfig_lookup "$pattern" >/dev/null 2>&1 && return 0
  # 缓存查不到时按文件系统复核:ldconfig 只按真实 SONAME 登记,不收录「别名软链」
  # (如自建的 libaio.so.1 -> libaio.so.1t64),但动态链接器在缓存未命中时会回退按
  # 默认目录搜索并正常加载(已实测 dlopen("libaio.so.1") 成功)→ 只看缓存会误判。
  while IFS= read -r dir; do
    [ -n "$dir" ] || continue
    # shellcheck disable=SC2086
    for f in ${dir}/${pattern}; do
      if [ -e "$f" ]; then return 0; fi
    done
  done <<<"$(lib_search_dirs)"
  return 1
}

# 取库的实际路径,参数同 have_lib;未找到返回 1(缓存优先,再按默认目录找)
lib_path() {
  local pattern="${1:-}" dir f
  [ -n "$pattern" ] || return 1
  _ldconfig_lookup "$pattern" && return 0
  while IFS= read -r dir; do
    [ -n "$dir" ] || continue
    # shellcheck disable=SC2086
    for f in ${dir}/${pattern}; do
      if [ -e "$f" ]; then printf '%s\n' "$f"; return 0; fi
    done
  done <<<"$(lib_search_dirs)"
  return 1
}

# 兼容 soname:发行版改了库文件名、而官方二进制仍按旧名加载时,补同名软链并刷新缓存
# 例:Ubuntu 24.04+/Debian 13+ 的 libaio1t64 只提供 libaio.so.1t64,
#    而 MySQL/MariaDB 官方二进制按 libaio.so.1 加载:
#      link_lib_compat libaio.so.1 libaio.so.1t64
# 已存在目标 soname 时直接返回 0(幂等)。
link_lib_compat() {
  local want="${1:-}" alt="${2:-}" src dst ld_bin
  if [ -z "$want" ] || [ -z "$alt" ]; then
    log_error "link_lib_compat: 用法 <需要的 soname> <现有 soname>"
    return 1
  fi
  have_lib "$want" && return 0
  src="$(lib_path "$alt")" || {
    log_warn "系统中找不到 ${alt},无法为 ${want} 建立兼容软链"
    return 1
  }
  dst="$(dirname "$src")/${want}"
  # 同目录内用相对链接(只写基名),以免把绝对路径固化在链接里
  ln -sfn "$(basename "$src")" "$dst" || {
    log_error "创建软链失败: ${dst} -> $(basename "$src")"
    return 1
  }
  if ld_bin="$(ldconfig_bin)"; then
    "${ld_bin}" >/dev/null 2>&1 || log_warn "ldconfig 刷新失败,请手动执行 ldconfig"
  fi
  if ! have_lib "$want"; then
    log_warn "已创建 ${dst},但 ldconfig 缓存未收录 ${want}(运行时会回退按目录搜索,一般无影响)"
  fi
  log_info "已建立兼容软链: ${dst} -> $(basename "$src")"
  return 0
}

# 按包管理器装单个包:成功输出打到 stdout(供调用方入日志),失败返回 1。
# 之所以逐个 case 分派而不是统一拼 "<pm> install -y <包>":各家的静默开关不同,
# 拼出来的命令未必成立。
_pkg_install_one() {
  local pm="${1:-}" p="${2:-}"
  [ -n "$pm" ] && [ -n "$p" ] || return 1
  case "$pm" in
    apt)
      DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$p" 2>&1
      ;;
    apk) apk add --no-cache "$p" 2>&1 ;;
    *) "$pm" install -y "$p" 2>&1 ;;
  esac
}

# 依次尝试候选包名,装上任意一个即成功;全失败返回 1(不中断脚本,由调用方决定后果)
# 用法: pkg_install_any apt libaio1t64 libaio1 ; pkg_install_any dnf libaio
#       ; pkg_install_any apk png
pkg_install_any() {
  local pm="$1" p out
  shift
  if [ "$#" -eq 0 ]; then
    log_error "pkg_install_any: 未提供候选包名"
    return 1
  fi
  for p in "$@"; do
    out=""
    if out="$(_pkg_install_one "$pm" "$p")"; then
      log_info "已安装依赖: ${p}"
      return 0
    fi
    # 把失败原因写进日志(源不可达 / dpkg 锁 / 包名不存在…):
    # 否则「装不上」只剩一行结论,排查时无从下手(用 tail 避免 grep -m 触发 SIGPIPE)
    log_warn "安装 ${p} 失败: $(printf '%s' "$out" | tail -n 2 | tr '\n' ' ')"
  done
  log_warn "以下候选包均未能安装: $*"
  return 1
}

app_install_complete() { [ -n "${APP_PATH:-}" ] && [ -f "${APP_PATH}/info.yaml" ]; }

# 路径是否在指定前缀下(删除前的围栏):path_under ${APPS_DIR}/mysql-8.0 ${APPS_DIR}
path_under() {
  local p="${1:-}" pre="${2:-}"
  [ -n "$p" ] && [ -n "$pre" ] || return 1
  case "$p" in "$pre"/*) return 0 ;; *) return 1 ;; esac
}

# 安全删除(目录 / 文件 / 软链通吃;空路径与 / 一律拒绝),不存在时静默返回 0
remove_path() {
  local p="${1:-}"
  [ -n "$p" ] && [ "$p" != "/" ] || { log_error "remove_path: 拒绝删除 '${p:-空}'"; return 1; }
  [ -e "$p" ] || [ -L "$p" ] || return 0
  rm -rf -- "$p"
}

# 停止并禁用服务(幂等;systemctl / service / chkconfig 都试,均缺失返回 1)
service_stop_disable() {
  local unit="${1:-}" rc=1
  [ -n "$unit" ] || { log_error "service_stop_disable: 需要 unit 名"; return 1; }
  if command -v systemctl >/dev/null 2>&1; then
    systemctl stop "$unit" >/dev/null 2>&1 || true
    systemctl disable "$unit" >/dev/null 2>&1 || true
    rc=0
  fi
  if command -v service >/dev/null 2>&1; then
    service "$unit" stop >/dev/null 2>&1 || true
    rc=0
  fi
  if command -v chkconfig >/dev/null 2>&1; then chkconfig --del "$unit" >/dev/null 2>&1 || true; rc=0; fi
  return "$rc"
}

db_data_initialized() {
  local d="${1:-}"
  [ -n "$d" ] || return 1
  [ -d "${d}/mysql" ] || [ -f "${d}/ibdata1" ] || [ -f "${d}/mysql.ibd" ]
}


install_system_deps() {
  local p pm deps="" rc=0
  if is_os ubuntu debian; then
    log_info "apt install build-dependencies ..."
    apt-get update -y >/dev/null 2>&1 || log_warn "apt-get update failed (continuing to attempt installation)"
    deps="${UBUNTU_DEPS}"
    # shellcheck disable=SC2086
    if ! apt-get install -y --no-install-recommends ${deps} >/dev/null 2>&1; then
      log_warn "Now installing missing packages ..."
      # shellcheck disable=SC2086
      for p in ${deps}; do
        apt-get install -y --no-install-recommends "$p" >/dev/null 2>&1 \
          || { log_warn "  Skip (Installation failed): ${p}"; rc=1; }
      done
    fi
  elif is_os centos rhel rocky alma ol amazon fedora; then
    if command -v dnf >/dev/null 2>&1; then pm="dnf"; deps="${RH_DEPS} ${RH_DNF_EXTRA}"; else pm="yum"; deps="${RH_DEPS}"; fi
    log_info "${pm} Install compilation dependencies..."
    # shellcheck disable=SC2086
    if ! ${pm} install -y ${deps} >/dev/null 2>&1; then
      log_warn "Now installing missing packages ..."
      # shellcheck disable=SC2086
      for p in ${deps}; do
        ${pm} install -y "$p" >/dev/null 2>&1 || { log_warn "  Skip (Installation failed): ${p}"; rc=1; }
      done
    fi
  elif is_os alpine; then
    log_info "apk install build-dependencies ..."
    # shellcheck disable=SC2086
    apk add --no-cache ${ALPINE_DEPS} >/dev/null 2>&1 || rc=1
  else
    log_warn "The distribution (${OS_NAME:-unknown}) that supports automatic installation of dependencies is not supported temporarily, skipping; please manually install the compilation dependencies"
    rc=1
  fi
  if [ "$rc" -eq 0 ]; then log_ok "System compilation dependencies are ready"; else log_warn "Some dependencies failed to install. If there are compilation errors reporting missing header files/libraries, please manually install them"; fi
  # 0 = all ready
  return "$rc"
}

# ── prepare_install_env:安装前置汇总(运行用户 + 关键目录 + 首次系统依赖) ─────
# 用法: prepare_install_env [运行用户] [附加组...]
#   prepare_install_env            # www 用户 + www 组(缺省)
#   prepare_install_env mysql      # mysql 用户 + mysql 组(组缺省与用户同名)
#   prepare_install_env www www zap
# 行为:
#   * 运行用户 / 组与 PKG_PATH、BUILD_PATH 目录每次都保证(幂等);
#   * 系统编译依赖只在首次安装:成功写 system_deps.lock,未装全则不写锁(下次重试);
#     设 ZAP_FORCE_DEPS=1 可强制重装。
# 返回:用户建不出来返回 1;依赖装得成装不成都不中断脚本(缺包会在 configure/make 阶段暴露)
prepare_install_env() {
  local user="${1:-www}" lock_dir lock
  [ "$#" -gt 0 ] && shift
  # If a group is not explicitly provided, add a group with the same name (www→www)
  if [ "$#" -eq 0 ]; then
    ensure_user "${user}" "${user}" || return 1
  else
    ensure_user "${user}" "$@" || return 1
  fi

  # deps lock : ZAP_PATH/data/state
  lock_dir="${ZAP_DATA_PATH:-/tmp}/state"
  ensure_dir "${PKG_PATH:-/tmp/pkg}" "${BUILD_PATH:-/tmp/build}" "${lock_dir}" \
    || log_warn "Directory creation failed"

  log_info "OS Name: ${OS_PRETTY:-${OS_NAME:-unknown}}, arch: ${OS_ARCH:-unknown} (alias: ${OS_ARCH_ALIAS:-unknown})"

  lock="${lock_dir}/system_deps.lock"
  if [ "${ZAP_FORCE_DEPS:-0}" != "1" ] && [ -f "$lock" ]; then
    log_info "Dependency lock ${lock} detected, system compilation dependencies ready, skipping installation"
    return 0
  fi
  if install_system_deps && touch "$lock" 2>/dev/null; then
    log_ok "System compilation dependencies are ready (${lock} has been recorded)"
  else
    log_warn "Failed, try again next time"
  fi
  return 0
}


os_detect
