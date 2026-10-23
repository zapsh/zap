#!/usr/bin/env bash
#
# ZAP Development One-Click Run Script
#
# Usage:
#   ./rundev.sh                 # Build frontend + backend, start zapexec (root) and zapd
#                               # (fmt / clippy are skipped by default: clippy uses its own
#                               #  driver to recompile the entire workspace, so waiting several
#                               #  minutes after changing one line isn't worth it)
#   ./rundev.sh --release       # Use a release build
#   ./rundev.sh --with-pro      # Enable the commercial module Zap Pro (cargo --features zapd/commercial);
#                               # requires zappro/ to be cloned into the repo root (see zappro/README.md),
#                               # automatically runs zappro/pro.sh setup to sync pages before building
#   ./rundev.sh --skip-web      # Skip frontend build
#   ./rundev.sh --skip-build    # Skip cargo build
#   ./rundev.sh --skip-install  # Skip nodejs install (node_modules)
#   ./rundev.sh --check-code    # Run fmt / clippy before building
#   ./rundev.sh --skip-check    # Checks are skipped by default; this flag is kept only for
#                               # backward compatibility with old usage
#   ./rundev.sh --reset-db      # Delete data/zap.db and rebuild a fresh database
#   ./rundev.sh --admin-user zapops --admin-pass secret
#                               # Specify the initial admin username/password (default admin / 123456),
#                               # only takes effect when the database doesn't exist and is being
#                               # created for the first time
#   ./rundev.sh --check         # Only check Rust formatting (fmt) and code (clippy); don't build
#                               # or start services
#   ./rundev.sh --fmt            # Only auto-format (cargo fmt --all) and exit; don't check/build/start
#                               # services (opposite of --check: --check is read-only inspection,
#                               # --fmt directly modifies files to fix them)
#   ./rundev.sh --clippy-fix     # Only auto-fix clippy warnings (cargo clippy --fix) and exit;
#                               # don't build/start (suggestions the machine can apply automatically
#                               # are rewritten directly; remaining warnings still need manual handling)
#
# Note: zapexec requires root privileges; the script starts it via sudo (may prompt for a
#       password the first time).
#
# Software Default Install Path: /usr/local/apps
# export ZAP_APPS_DIR=custom path
set -euo pipefail

# ── terminal colors ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { echo -e "${BLUE}[*]${NC} $*"; }
ok()   { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
die()  { echo -e "${RED}[✗]${NC} $*" >&2; exit 1; }

usage() { awk 'NR>2 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"; }

if [ ! -d "zappro" ]; then
  mkdir -p "zappro/src"
  echo '// placehlder' > "zappro/src/mod.rs"
fi

# ── arguments parsing ────────────────────────────────────────────────
RELEASE=false; SKIP_WEB=false; SKIP_BUILD=false; SKIP_INSTALL=false; CHECK_CODE=false; RESET_DB=false; CHECK=false; FMT_ONLY=false; CLIPPY_FIX=false
WITH_PRO=false
ADMIN_USER=""; ADMIN_PASS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --release)      RELEASE=true; shift ;;
    --with-pro)     WITH_PRO=true; shift ;;
    --skip-web)     SKIP_WEB=true; shift ;;
    --skip-build)   SKIP_BUILD=true; shift ;;
    --skip-install) SKIP_INSTALL=true; shift ;;
    --check-code|--lint) CHECK_CODE=true; shift ;;
    --skip-check)   shift ;;
    --reset-db|--fresh-db) RESET_DB=true; shift ;;
    --check)        CHECK=true; shift ;;
    --fmt|--format)  FMT_ONLY=true; shift ;;
    --clippy-fix|--lint-fix) CLIPPY_FIX=true; shift ;;
    --admin-user)   ADMIN_USER="${2:-}"
                    [ -n "$ADMIN_USER" ] || die "--admin-user requires a username"
                    shift 2 ;;
    --admin-user=*) ADMIN_USER="${1#*=}"
                    [ -n "$ADMIN_USER" ] || die "--admin-user requires a username"
                    shift ;;
    --admin-pass|--admin-password)
                    ADMIN_PASS="${2:-}"
                    [ -n "$ADMIN_PASS" ] || die "--admin-pass requires a password"
                    shift 2 ;;
    --admin-pass=*|--admin-password=*)
                    ADMIN_PASS="${1#*=}"
                    [ -n "$ADMIN_PASS" ] || die "--admin-pass requires a password"
                    shift ;;
    -h|--help)      usage; exit 0 ;;
    *)              die "unknown option: $1 (--help for usage)" ;;
  esac
done

# initial admin user/password only take effect when database is first created.
[ -n "$ADMIN_USER" ] || ADMIN_USER="${ZAP_ADMIN_USER:-admin}"
[ -n "$ADMIN_PASS" ] || ADMIN_PASS="${ZAP_ADMIN_PASSWORD:-123456}"

# project root dir
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# --with-pro 是手动开关（同 build.sh）：不带就是开源构建，产物与现在完全一致
if [ "$WITH_PRO" = true ]; then
  [ -f "$ROOT_DIR/zappro/pro.sh" ] \
    || die "specified --with-pro but failed to find $ROOT_DIR/zappro (please clone the commercial module repository first)"
  info "sync pro page ..."
  bash "$ROOT_DIR/zappro/pro.sh" setup || die "failed to sync Pro pages"
  info "enable Zap Pro(-features zapd/commercial)"
fi

if [ -f "$ROOT_DIR/zappro/pro.sh" ]; then
  CLIPPY_FEATURES=(--all-features)
else
  CLIPPY_FEATURES=()
fi

if [ "$CHECK" = true ] && [ "$SKIP_BUILD" = true ]; then
  die "--check 与 --skip-build 互斥（--check 本就不构建，只做 fmt / clippy 检查）"
fi

# ── --check： Only check Rust code formatting (fmt) and code (clippy); do not build or start services.
if [ "$CHECK" = true ]; then
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
  info "检查格式 (cargo fmt --all -- --check) ..."
  if ! cargo fmt --all -- --check; then
    die "格式检查未通过：执行 cargo fmt --all 自动修复后重试"
  fi
  ok "格式检查通过"
  info "检查代码 (cargo clippy --all-targets --all-features -- -D warnings) ..."
  cargo clippy --all-targets "${CLIPPY_FEATURES[@]}" -- -D warnings || die "代码检查未通过（详见上方 clippy 输出）"
  ok "代码检查通过，一切正常"
  exit 0
fi

# ── --fmt：format code (cargo fmt --all) and exit; 
if [ "$FMT_ONLY" = true ]; then
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
  info "格式化代码 (cargo fmt --all) ..."
  cargo fmt --all || die "格式化失败"
  ok "代码已格式化"
  exit 0
fi

# ── --clippy-fix：auto-fix clippy warnings (cargo clippy --fix) and exit; 
if [ "$CLIPPY_FIX" = true ]; then
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
  info "自动修复 clippy 告警 (cargo clippy --fix) ..."
  cargo clippy --fix "${CLIPPY_FEATURES[@]}" --all-targets --allow-dirty \
    || warn "clippy 自动修复未全部完成（部分告警需手动修复，或存在编译错误）"
  ok "clippy 自动修复完成"
  exit 0
fi

WEB_DIR="$ROOT_DIR/web"
RUN_DIR="$ROOT_DIR/data/run"
EXEC_SOCKET="$RUN_DIR/sock/exec.sock"
EXEC_SECRET="$RUN_DIR/exec.key"
DEV_CONF="$RUN_DIR/zap.dev.yaml"
DEV_USER="$(id -un)"
# 面板运行组：凭据文件（/etc/zap/credentials）要 chgrp 给它，zapd 才读得到。
# 生产默认组是 zapadm；dev 下 zapd 跑在当前用户上，组不存在时 chgrp 会静默跳过，
# 凭据留在 root:root → 数据库菜单的环境门禁探测（zapadm 凭据可读性）永远 false。
DEV_GROUP="$(id -gn "$DEV_USER")"


CONF_URL_PREFIX=""
if [ -f "$DEV_CONF" ]; then
  CONF_URL_PREFIX=$(sed -n 's/^[[:space:]]*url_prefix:[[:space:]]*"\{0,1\}\([^"#]*\)"\{0,1\}[[:space:]]*$/\1/p' \
    "$DEV_CONF" | head -1 | sed 's/[[:space:]]*$//')
elif [ -f "$ROOT_DIR/conf/zap.yaml" ]; then
  CONF_URL_PREFIX=$(sed -n 's/^[[:space:]]*url_prefix:[[:space:]]*"\{0,1\}\([^"#]*\)"\{0,1\}[[:space:]]*$/\1/p' \
    "$ROOT_DIR/conf/zap.yaml" | head -1 | sed 's/[[:space:]]*$//')
fi

# development mode: use self-signed certs (zap.crt / zap.key) in the repo root
DEV_CRT="$RUN_DIR/zap.crt"
DEV_KEY="$RUN_DIR/zap.key"

if [ "$RELEASE" = true ]; then
  BIN_DIR="$ROOT_DIR/target/release"
  CARGO_FLAGS=(--release)
else
  BIN_DIR="$ROOT_DIR/target/debug"
  CARGO_FLAGS=()
fi

# commercial mode
if [ "$WITH_PRO" = true ]; then
  CARGO_FLAGS+=(--features zapd/commercial)
fi

# ── dependencies check ────────────────────────────────────────────────
command -v cargo >/dev/null 2>&1 || die "not found cargo, please install Rust"
if [ "$SKIP_WEB" != true ]; then
  command -v npm >/dev/null 2>&1 || die "not found npm, please install Node.js"
fi

# ── sudo 授权（zapexec 需要 root）───────────────────────────
if [ "$(id -u)" -eq 0 ]; then
  SUDO_CMD=()
else
  command -v sudo >/dev/null 2>&1 || die "not found sudo, please install sudo"
  SUDO_CMD=(sudo -n)
  if ! sudo -n true 2>/dev/null; then
    info "zapexec requires root privileges, please enter your sudo password"
    sudo -v || die "sudo authorization failed"
  fi
fi

# ── 1. 构建前端 ─────────────────────────────────────────────
if [ "$SKIP_WEB" = true ]; then
  warn "Skip web build (use --skip-web to skip)"
else
  if [ ! -d "$WEB_DIR/node_modules" ] && [ "$SKIP_INSTALL" != true ]; then
    info "not found node_modules, executing npm install ..."
    (cd "$WEB_DIR" && npm install) || die "npm install error"
  fi
  info "building frontend (npm run build:prod) ..."
  (cd "$WEB_DIR" && npm run build:prod) || die "frontend build failed"
  ok "frontend build completed -> $WEB_DIR/dist"
fi

# ── 2. Build Backend ─────────────────────────────────────────────
if [ "$SKIP_BUILD" = true ]; then
  warn "Skip cargo build (use --skip-build to skip)"
else
  if [ "$CHECK_CODE" = true ]; then
    info "Format Code (cargo fmt --all) ..."
    cargo fmt --all
    info "Clippy Check Code (cargo clippy --all-targets --all-features -- -D warnings) ..."
    cargo clippy --all-targets "${CLIPPY_FEATURES[@]}" -- -D warnings || die "Code Check Failed (see clippy output above)"
  else
    warn "Skip fmt / clippy check (add --check-code to run)"
  fi
  info "Build Backend (cargo build ${CARGO_FLAGS[*]} --bin zapd --bin zapexec --bin zapctl --bin zapupgrade) ..."
  cargo build "${CARGO_FLAGS[@]}" --bin zapd --bin zapexec --bin zapctl --bin zapupgrade || die "Build Failed"
  ok "Zapd/Zapexec/Zapctl/Zapupgrade Build Success-> $BIN_DIR"
fi

# ── 3. configuration ─────────────────────────────
mkdir -p "$RUN_DIR"
if [ ! -f "$DEV_CONF" ]; then
  info "Generating dev config : $DEV_CONF"
  cat > "$DEV_CONF" <<EOF
server:
  address: 0.0.0.0
  port: 2663
  cert_file: $ROOT_DIR/conf/zap.crt
  key_file: $ROOT_DIR/conf/zap.key
  url_prefix: "$CONF_URL_PREFIX"
jwt:
  jwt_secure: zap-dev-insecure-secret
  jwt_expire: 3600
exec:
  socket_path: $EXEC_SOCKET
  secret_path: $EXEC_SECRET
db:
  path: $ROOT_DIR/data/zap.db
EOF
  ok "development config generated"
fi

# ── 3.5 remove zap.db（--reset-db）──────────────────────────────
if [ "$RESET_DB" = true ]; then
  if [ -f "$ROOT_DIR/data/zap.db" ]; then
    warn "deleting database $ROOT_DIR/data/zap.db ..."
    rm -f "$ROOT_DIR/data/zap.db" "$ROOT_DIR/data/zap.db-wal" "$ROOT_DIR/data/zap.db-shm"
    ok "database deleted, will be rebuilt on startup"
  else
    info "database does not exist, skipping deletion"
  fi
  # 只删库不删 nginx vhost 配置，会让残留的 zap-site-*.conf（站点 id 复用后）指向
  # 别人的站点目录。reset-db 顺手清掉它们（00-default.conf 不匹配 zap-site-*，安全）。
  warn "cleaning orphan nginx vhost configs ..."
  for d in sites-available sites-enabled; do
    "${SUDO_CMD[@]}" rm -f /etc/zap/webservers/nginx/$d/zap-site-*.conf
  done
  # 仍运行就重载 nginx，让 include 立即生效（配置无误才 reload，否则跳过）
  if command -v nginx >/dev/null 2>&1; then
    "${SUDO_CMD[@]}" nginx -t >/dev/null 2>&1 && "${SUDO_CMD[@]}" nginx -s reload >/dev/null 2>&1 || true
  fi
  ok "orphan vhost configs cleaned"
fi

# ── cleanup and exit ──────────────────────────────────────────────
ZAPEXEC_PID=""
ZAPD_PID=""

cleanup() {
  trap - EXIT INT TERM
  echo ""
  info "正在停止服务 ..."
  [ -n "$ZAPD_PID" ] && kill "$ZAPD_PID" 2>/dev/null || true
  if [ -n "$ZAPEXEC_PID" ]; then
    "${SUDO_CMD[@]}" kill "$ZAPEXEC_PID" 2>/dev/null || true
    "${SUDO_CMD[@]}" pkill -f "$BIN_DIR/zapexec" 2>/dev/null || true
  fi
  ZAPD_PID=""
  ZAPEXEC_PID=""
  ok "服务已停止"
}
trap cleanup EXIT INT TERM

# ── 4. 启动 zapexec (root) ──────────────────────────────────
info "启动 zapexec (root)：socket=$EXEC_SOCKET client-user=$DEV_USER"
ZAPEXEC_ENVS=(env ZAP_PATH="$ROOT_DIR" ZAP_PANEL_GROUP="$DEV_GROUP")
if [ -n "${ZAP_APPS_DIR:-}" ]; then
  ZAPEXEC_ENVS+=(ZAP_APPS_DIR="$ZAP_APPS_DIR")
fi
"${SUDO_CMD[@]}" "${ZAPEXEC_ENVS[@]}" "$BIN_DIR/zapexec" \
  --socket "$EXEC_SOCKET" \
  --secret "$EXEC_SECRET" \
  --client-user "$DEV_USER" &
ZAPEXEC_PID=$!
sleep 1

# ── 5. 启动 zapd ────────────────────────────────────────────
info "启动 zapd ..."
# 库还不存在时先建库并写入初始管理员（凭据走命令行，不落文件；
# 库已存在则原样不动，避免每次重启都覆盖改动过的密码）
if [ ! -f "$ROOT_DIR/data/zap.db" ]; then
  info "初始化管理员 ${ADMIN_USER} ..."
  ZAP_CONFIG="$DEV_CONF" "$BIN_DIR/zapd" --init-admin "$ADMIN_USER" \
    --admin-password "$ADMIN_PASS"
fi
# zapd 也要注入同一个 ZAP_PATH：否则其数据目录回退到默认 /usr/local/zap，
# 与 zapexec（已注入 ZAP_PATH=$ROOT_DIR）错位 —— 典型表现就是异步插件日志流
# 打开失败（「非法日志路径」：zapexec 把日志写到 $ROOT_DIR/...，zapd 却只白名单 /usr/local/zap/...）。
ZAP_PATH="$ROOT_DIR" ZAP_CONFIG="$DEV_CONF" ZAP_PANEL_GROUP="$DEV_GROUP" "$BIN_DIR/zapd" &
ZAPD_PID=$!

echo ""
ok "全部服务已启动"
info "  zapd    : https://127.0.0.1:2663"
info "  管理员  : ${ADMIN_USER} / ${ADMIN_PASS}（仅全新数据库生效）"
if [ "$RESET_DB" = true ]; then
  info "  家目录  : /home/${ADMIN_USER}（不存在时由 zapexec 自动创建）"
fi
info "  zapexec : $EXEC_SOCKET （root 特权守护进程）"
info "  zapupgrade: $BIN_DIR/zapupgrade （系统升级器，开发环境为手动重启模式）"
info "  按 Ctrl+C 停止全部服务"
echo ""

# ── 6. 等待任一服务退出 ─────────────────────────────────────
wait -n "$ZAPEXEC_PID" "$ZAPD_PID" || true
