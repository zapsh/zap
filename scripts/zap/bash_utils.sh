#!/usr/bin/env bash
#=============================================================================
# bash_utils.sh — ZAP 应用商店脚本公共函数库
#
# 用法:在应用脚本中 source 后调用其函数:
#     source "${ZAP_PATH}/scripts/zap/bash_utils.sh"
#
# 说明:
#   * 本文件是纯函数库,被 source 时不会改动调用方 shell 选项(不设置
#     set -e/-u/-o pipefail),也不会强制退出;请勿直接运行。
#   * 调用方通常由 zapexec 以 root 执行,并已注入如下环境变量:
#       ZAP_PATH ZAPCTL APPS_DIR PKG_PATH APP_ID APP_NAME APP_PATH
#       BUILD_PATH ZAP_DATA_PATH APP_VERSION MAJOR_VERSION MINOR_VERSION
#       APP_OLD_VERSION CPU_NUM ACTION ...(选项同名变量)
#   * 若未运行在 root 下,请自行调用 assert_root 做前置校验。
#
# 函数清单:
#   assert_root ensure_dir ensure_user ensure_group ensure_usergroup
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
# 顶层变量(被 source 后立即可用):
#   OS_NAME(发行版小写 ID) OS_VERSION(版本号) OS_PRETTY OS_ID_LIKE
#   OS_MACHINE(uname -s) OS_ARCH(uname -m) OS_ARCH_ALIAS(amd64/arm64/...)
#=============================================================================

# 直接运行本文件(非 source)时仅打印帮助
if [ "${BASH_SOURCE[0]:-}" = "$0" ]; then
  echo "bash_utils.sh 是一个函数库,请勿直接运行。" >&2
  echo "在应用脚本中使用: source \"\${ZAP_PATH}/scripts/zap/bash_utils.sh\"" >&2
  exit 0
fi

# ── 日志 ──────────────────────────────────────────────────────────────────
# 统一输出格式;安装日志(含 stderr)会被实时写入 run-<run_id>.log
_log_ts() { date '+%Y-%m-%d %H:%M:%S'; }
log_info()  { printf '[%s] [ INFO ] %s\n' "$(_log_ts)" "$*"; }
log_ok()    { printf '[%s] [  OK  ] %s\n' "$(_log_ts)" "$*"; }
log_warn()  { printf '[%s] [ WARN ] %s\n' "$(_log_ts)" "$*" >&2; }
log_error() { printf '[%s] [ERROR ] %s\n' "$(_log_ts)" "$*" >&2; }

# ── 基础工具 ──────────────────────────────────────────────────────────────
# 检查是否以 root 运行(应用脚本依赖;仅在必要时调用,失败返回 1)
assert_root() {
  if [ "$(id -u)" -ne 0 ]; then
    log_error "需要 root 权限运行(当前 uid=$(id -u))"
    return 1
  fi
}

# 确保目录存在,支持一次传入多个路径;任一创建失败返回 1
ensure_dir() {
  local d
  for d in "$@"; do
    if [ -n "$d" ] && [ ! -d "$d" ]; then
      mkdir -p "$d" || { log_error "创建目录失败: $d"; return 1; }
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

# 确保用户属于指定组:组不存在先创建,已是成员则跳过(幂等)
# 用法: ensure_usergroup <user> <group> [group2 ...]
ensure_usergroup() {
  local user="${1:-}" group
  if [ -z "$user" ]; then
    log_error "ensure_usergroup: 缺少用户名"
    return 1
  fi
  shift
  if [ "$#" -eq 0 ]; then
    log_error "ensure_usergroup: 至少需要一个组名(用法: ensure_usergroup <user> <group> [group2 ...])"
    return 1
  fi
  # 用户必须已存在:请先 ensure_user,避免拼错用户名被静默创建
  if ! id "${user}" >/dev/null 2>&1; then
    log_error "ensure_usergroup: 用户 ${user} 不存在(请先调用 ensure_user)"
    return 1
  fi

  for group in "$@"; do
    [ -n "$group" ] || continue
    ensure_group "${group}" || return 1
    if _user_in_group "${user}" "${group}"; then
      continue
    fi
    if command -v usermod >/dev/null 2>&1; then
      # -aG 追加附加组,不动主组
      usermod -aG "${group}" "${user}" >/dev/null 2>&1 \
        || { log_error "将用户 ${user} 加入组 ${group} 失败(usermod)"; return 1; }
    elif command -v addgroup >/dev/null 2>&1; then
      addgroup "${user}" "${group}" >/dev/null 2>&1 \
        || { log_error "将用户 ${user} 加入组 ${group} 失败(addgroup)"; return 1; }
    else
      log_error "系统缺少 usermod/addgroup,无法将用户 ${user} 加入组 ${group}"
      return 1
    fi
    log_ok "已将用户 ${user} 加入组: ${group}"
  done
}

# 创建运行用户(默认 www);已存在直接成功;无 useradd 时退回 adduser(Alpine)
# 可选附加组: ensure_user www www / ensure_user php-fpm www zap
#   —— 组不存在会自动创建,用户会被加进去(用户已存在时也会补齐附加组)
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
      || { log_error "创建用户 ${user} 失败(useradd)"; return 1; }
  elif command -v adduser >/dev/null 2>&1; then
    adduser -S -D -H -s /sbin/nologin "${user}" >/dev/null 2>&1 \
      || { log_error "创建用户 ${user} 失败(adduser)"; return 1; }
  else
    log_error "系统缺少 useradd/adduser,无法创建用户 ${user}"
    return 1
  fi
  log_ok "已创建运行用户: ${user}"
  if [ "$#" -gt 0 ]; then
    ensure_usergroup "${user}" "$@" || return 1
  fi
}

# ── 系统 / 架构探测 ───────────────────────────────────────────────────────
# x86_64 -> amd64;aarch64 -> arm64;x86 -> i386;其余原样
normalize_arch() {
  case "${1:-$(uname -m)}" in
    x86_64 | amd64) echo "amd64" ;;
    aarch64 | arm64) echo "arm64" ;;
    i?86 | x86) echo "i386" ;;
    *) echo "${1:-unknown}" ;;
  esac
}

# 探测发行版 / 内核 / 架构,设置顶层变量(可随时重跑刷新)
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

# 判断发行版:is_os ubuntu debian / is_os centos rocky alma ...
# 同时匹配 ID 与 ID_LIKE(如 ubuntu 的 ID_LIKE 含 debian)
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
# 系统包管理器:apt / dnf / yum / apk / zypper,未识别返回 1。
# 只认 Linux 发行版的那几家;部分 Linux 上也有叫 pkg 的零散命令(如 pkgsrc 的
# 包装脚本),只按命令名认会误判。
pkg_manager() {
  if command -v apt-get >/dev/null 2>&1; then echo apt
  elif command -v dnf >/dev/null 2>&1; then echo dnf
  elif command -v yum >/dev/null 2>&1; then echo yum
  elif command -v apk >/dev/null 2>&1; then echo apk
  elif command -v zypper >/dev/null 2>&1; then echo zypper
  else return 1
  fi
}

# 可用 CPU 核数(带容错;若注入 CPU_NUM 则以其为上限)
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

# ── 下载 ──────────────────────────────────────────────────────────────────
# 私有下载器:curl 优先,回退 wget;自动重试;成功返回 0
# 进度输出:curl --progress-bar / wget --show-progress 强制在非 TTY(日志文件)下
# 也以 `\r` 刷新同一行进度,面板日志(xterm 渲染)中表现为一条实时进度条。
fetch_file() {
  # fetch_file <url> <dest> [重试次数,默认3]
  local url="$1" dest="$2" retries="${3:-3}" i=0
  if command -v curl >/dev/null 2>&1; then
    while [ "$i" -lt "$retries" ]; do
      if curl -fL --progress-bar --connect-timeout 15 -4 -o "$dest" "$url"; then return 0; fi
      i=$((i + 1)); [ "$i" -lt "$retries" ] && sleep 1
    done
  elif command -v wget >/dev/null 2>&1; then
    while [ "$i" -lt "$retries" ]; do
      if wget -q --show-progress -4 --timeout=60 --tries=2 -O "$dest" "$url"; then return 0; fi
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
    log_info "下载完成: ${dest}"
    return 0
  fi
  log_error "下载失败,中止: ${url}"
  exit 1
}

# http_fetch <url> <dest>(旧版兼容):失败即中止
http_fetch() {
  local url="$1" dest="$2"
  fetch_file "$url" "$dest" || {
    log_error "Failed to fetch ${url}. Aborting install."
    exit 1
  }
}

# 解压 tar.*/zip 到目标目录(自动识别格式;依赖系统 tar/unzip)
extract_archive() {
  # extract_archive <归档文件> [目标目录,默认当前目录]
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
        log_error "缺少 unzip,无法解压: ${archive}"
      fi ;;
    *) log_error "不支持的文件格式: ${archive}" ;;
  esac
  [ "$ok" -eq 1 ] && return 0
  log_error "解压失败: ${archive}"
  return 1
}

# download_extract <url> <本地归档名> <目标目录>:下载并解压一步到位
download_extract() {
  local url="$1" name="$2" dir="$3"
  fetch_file "$url" "$name" || { log_error "下载失败: ${url}"; return 1; }
  extract_archive "$name" "$dir"
}

# ── 编译 ──────────────────────────────────────────────────────────────────
# GNU make 可执行文件名：发行版自带的 make 未必是 GNU make，优先用包管理器装的 gmake。
# 判定看 `make --version` 的自报家门而不是管道 grep -q —— GNU make 输出多行时
# grep -q 命中即退会让 make 收到 SIGPIPE，pipefail 下反把「是 GNU make」判成不是。
# 优先级：ZAP_MAKE 覆盖 > gmake > make；都遇不上 GNU make 时回退第一个能用的那个。
make_bin() {
  local m first="" ver
  for m in "${ZAP_MAKE:-}" gmake make; do
    [ -n "$m" ] || continue
    command -v "$m" >/dev/null 2>&1 || continue
    [ -n "$first" ] || first="$m"
    ver="$("$m" --version 2>/dev/null || printf '')"
    case "$ver" in *"GNU Make"*) printf '%s\n' "$m"; return 0 ;; esac
  done
  [ -n "$first" ] || { log_error "系统缺少 make / gmake，无法编译"; return 1; }
  log_warn "未找到 GNU make，回退 ${first}：非 GNU make 不认 GNU Makefile，建议先装 gmake"
  printf '%s\n' "$first"
}

# 并行编译 + 安装，失败自动退回串行；可传并行数，缺省 CPU_NUM -> cpu_count
# make 一律走 make_bin：直接敲 make 可能撞上非 GNU 的那份。
MakeInstall() {
  local jobs="${1:-}" mb
  [ -n "$jobs" ] || jobs="${CPU_NUM:-}"
  [ -n "$jobs" ] || jobs="$(cpu_count)"
  mb="$(make_bin)" || return 1
  if "${mb}" -j "${jobs}"; then
    "${mb}" install
  else
    log_warn "并行 ${mb} -j${jobs} 失败，退回串行编译"
    "${mb}"
    "${mb}" install
  fi
}

# 版本比较(点分数字,忽略字母段,如 1.24.0p1 视为 1.24.0):
#   version_compare <a> <b> -> 0 相等 / 1 a>b / 2 a<b
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

# ── 版本分段解析(与 version_compare 同样忽略字母后缀) ────────────────────
# 取版本号第 N 段(从 1 开始),自动去掉字母后缀:1.1.1w → 第1段=1 第2段=1 第3段=1
# 用法: version_field <版本> <段号>
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

# 主版本号:version_major 1.1.1w → 1
version_major() { version_field "${1:-}" 1; }

# 次版本号:version_minor 1.1.1w → 1
version_minor() { version_field "${1:-}" 2; }

# 一次取出主次版本,输出 "major minor"(缺失的段为空串)
# 典型用法: read -r MAJOR MINOR <<<"$(version_major_minor "${APP_OLD_VERSION}")"
version_major_minor() {
  local major minor
  major="$(version_field "${1:-}" 1 || printf '')"
  minor="$(version_field "${1:-}" 2 || printf '')"
  printf '%s %s\n' "$major" "$minor"
}

# 生成随机密码(默认 16 位字母数字)
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

# ── 属性文件 / 配置读写 ───────────────────────────────────────────────────
# 读取 key=value 属性文件中的值(允许前导空格/行内注释值),无匹配返回空
getPropsValue() {
  # getPropsValue <文件> <key>
  local file="$1" key="$2" esc v
  [ -f "$file" ] || return 1
  esc="$(printf '%s' "$key" | sed 's/[][\\^$.*]/\\&/g')"
  v="$(grep -m1 "^[[:space:]]*${esc}[[:space:]]*=" "$file" 2>/dev/null | sed 's/^[^=]*=[[:space:]]*//')" || true
  printf '%s' "$v"
}

# ── YAML 键值读取 / 目录安全校验(应用商店通用) ──────────────────────────

# 读取 yaml 顶层键值(仅支持 `key: value` 简单形式),无匹配返回空
# 用法: yaml_value <文件> <key>
yaml_value() {
  local file="${1:-}" key="${2:-}" v
  [ -n "$key" ] || return 1
  [ -f "$file" ] || return 1
  v="$(sed -n "s/^[[:space:]]*${key}:[[:space:]]*//p" "$file" | head -1 | sed 's/[[:space:]]*$//')" || true
  printf '%s' "$v"
}

# 规范化目录:去掉末尾多余斜杠(保留根目录 "/",不解析软链)
# 用法: normalize_dir <路径>;路径为空返回非 0
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
RH_DNF_EXTRA="${ZAP_RH_DNF_EXTRA:-libzip-devel oniguruma-devel libicu-devel libffi-devel libxslt-devel gd-devel libsodium-devel}"
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

# ── 安装残局处理(失败后重跑) ─────────────────────────────────
# 系统只在脚本【成功退出】后才写 apps/<cat>/<name>/meta.yaml(面板据此显示「已安装」),
# 而脚本自己常以「安装目录在不在」做守卫。一旦中途失败(mysqld 初始化缺 libaio 等),
# 就形成死锁:面板说没装(不给卸载)+ 脚本说装了(不给重装)。
# 约定:以脚本末尾登记的 APP_PATH/info.yaml 为「装完了」的唯一依据;
# 目录还在但没登记 = 装了一半的残局 → 清理后继续装,而不是报错退出。
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

# 数据目录是否已完成初始化(MySQL / MariaDB 通用:有 mysql 系统库或 InnoDB 系统表空间)
# 未初始化(或半初始化)的数据目录里不会有用户数据,可随残局安全清掉;
# 已初始化的目录必须留给人工确认,绝不自动删。
db_data_initialized() {
  local d="${1:-}"
  [ -n "$d" ] || return 1
  [ -d "${d}/mysql" ] || [ -f "${d}/ibdata1" ] || [ -f "${d}/mysql.ibd" ]
}

# 安装系统编译依赖:批量失败后自动逐项补装(单项失败仅告警,不中断脚本);
# 因为依赖缺失会在 configure/make 阶段暴露,不应因个别包名差异中止整个安装
install_system_deps() {
  local p pm deps="" rc=0
  if is_os ubuntu debian; then
    log_info "apt 安装编译依赖 ..."
    apt-get update -y >/dev/null 2>&1 || log_warn "apt-get update 失败(继续尝试安装)"
    deps="${UBUNTU_DEPS}"
    # shellcheck disable=SC2086
    if ! apt-get install -y --no-install-recommends ${deps} >/dev/null 2>&1; then
      log_warn "批量安装失败,逐项补装(个别包缺失忽略) ..."
      # shellcheck disable=SC2086
      for p in ${deps}; do
        apt-get install -y --no-install-recommends "$p" >/dev/null 2>&1 \
          || { log_warn "  跳过(安装失败): ${p}"; rc=1; }
      done
    fi
  elif is_os centos rhel rocky alma ol amazon fedora; then
    if command -v dnf >/dev/null 2>&1; then pm="dnf"; deps="${RH_DEPS} ${RH_DNF_EXTRA}"; else pm="yum"; deps="${RH_DEPS}"; fi
    log_info "${pm} 安装编译依赖 ..."
    # shellcheck disable=SC2086
    if ! ${pm} install -y ${deps} >/dev/null 2>&1; then
      log_warn "批量安装失败,逐项补装(个别包缺失忽略) ..."
      # shellcheck disable=SC2086
      for p in ${deps}; do
        ${pm} install -y "$p" >/dev/null 2>&1 || { log_warn "  跳过(安装失败): ${p}"; rc=1; }
      done
    fi
  elif is_os alpine; then
    log_info "apk 安装编译依赖 ..."
    # shellcheck disable=SC2086
    apk add --no-cache ${ALPINE_DEPS} >/dev/null 2>&1 || rc=1
  else
    log_warn "暂不支持自动安装依赖的发行版(${OS_NAME:-unknown}),跳过;请手动安装编译依赖"
    rc=1
  fi
  if [ "$rc" -eq 0 ]; then log_ok "系统编译依赖就绪"; else log_warn "部分依赖安装失败,如编译报缺头文件/库请手动补装"; fi
  # 返回真实结果(0=全部就绪),供 prepare_install_env 决定是否写依赖锁
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
  # 未显式给组时补一个同名组(www→www),跨发行版保证组一定存在
  if [ "$#" -eq 0 ]; then
    ensure_user "${user}" "${user}" || return 1
  else
    ensure_user "${user}" "$@" || return 1
  fi

  lock_dir="${ZAP_DATA_PATH:-/tmp}/tmp"
  ensure_dir "${PKG_PATH:-/tmp/pkg}" "${BUILD_PATH:-/tmp/build}" "${lock_dir}" \
    || log_warn "部分运行目录创建失败(PKG_PATH/BUILD_PATH 由执行器确保)"

  log_info "系统: ${OS_PRETTY:-${OS_NAME:-unknown}}, arch: ${OS_ARCH:-unknown} (alias: ${OS_ARCH_ALIAS:-unknown})"

  lock="${lock_dir}/system_deps.lock"
  # 旧锁名是 preinstall.lock:一并认,免得升级后每台机器都重跑一次包管理器
  if [ "${ZAP_FORCE_DEPS:-0}" != "1" ] && { [ -f "$lock" ] || [ -f "${lock_dir}/preinstall.lock" ]; }; then
    log_info "检测到依赖锁 ${lock},系统编译依赖已就绪,跳过安装"
    return 0
  fi
  if install_system_deps && touch "$lock" 2>/dev/null; then
    log_ok "系统编译依赖就绪(已记录 ${lock})"
  else
    # 装了一半(断网 / 源不可用)时不写锁,下次运行会重试,而不是带着残缺依赖去编译
    log_warn "系统编译依赖未完全就绪,未写锁 ${lock},下次运行会重试"
  fi
  return 0
}

# ── 顶层一次性探测(被 source 时执行;放在函数定义之后以保证可用) ──────────
os_detect
