#!/usr/bin/env bash
#
# ZAP 开发一键运行脚本
#
# 用法：
#   ./rundev.sh                 # 构建前端 + 后端，启动 zapexec(root) 和 zapd
#   ./rundev.sh --release       # 使用 release 构建
#   ./rundev.sh --skip-web      # 跳过前端构建
#   ./rundev.sh --skip-build    # 跳过 cargo 构建
#   ./rundev.sh --skip-install  # 缺 node_modules 时不自动 npm install
#   ./rundev.sh --skip-check    # 跳过构建前的 fmt / clippy 检查（改一行代码想快点跑起来时用）
#   ./rundev.sh --reset-db      # 删除 data/zap.db 重建全新数据库（admin 初始密码 A123456）
#   ./rundev.sh --check         # 只检查 Rust 格式(fmt)与代码(clippy)，不构建、不启动服务
#
# 注意：zapexec 需要 root 权限，脚本通过 sudo 启动（首次可能提示输入密码）。
#
# 软件安装根目录默认 /usr/local/apps（第三方软件安装位置，与面板数据解耦）；
# 需要自定义时，先 export ZAP_APPS_DIR=/你的/安装/目录 再运行本脚本即可。
set -euo pipefail

# ── 终端颜色 ────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info() { echo -e "${BLUE}[*]${NC} $*"; }
ok()   { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
die()  { echo -e "${RED}[✗]${NC} $*" >&2; exit 1; }

# 帮助文本取自文件头注释块（# 开头，直到第一行非注释为止）
usage() { awk 'NR>2 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"; }

# ── 参数解析 ────────────────────────────────────────────────
RELEASE=false; SKIP_WEB=false; SKIP_BUILD=false; SKIP_INSTALL=false; SKIP_CHECK=false; RESET_DB=false; CHECK=false
for arg in "$@"; do
  case "$arg" in
    --release)      RELEASE=true ;;
    --skip-web)     SKIP_WEB=true ;;
    --skip-build)   SKIP_BUILD=true ;;
    --skip-install) SKIP_INSTALL=true ;;
    --skip-check)   SKIP_CHECK=true ;;
    --reset-db|--fresh-db) RESET_DB=true ;;
    --check)        CHECK=true ;;
    -h|--help)      usage; exit 0 ;;
    *)              die "未知参数: $arg（--help 查看用法）" ;;
  esac
done

# ── 路径 ────────────────────────────────────────────────────
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# --check 就是只做检查，与"跳过检查"语义相反，同时给出视为用法错误
if [ "$CHECK" = true ] && [ "$SKIP_CHECK" = true ]; then
  die "--check 与 --skip-check 互斥（--check 的含义就是只做 fmt / clippy 检查）"
fi

# ── --check：只做静态检查（fmt + clippy），不构建、不启动服务 ──
# 提交前 / CI 用：纯只读检查（不会自动改写代码），失败以非零码退出。
if [ "$CHECK" = true ]; then
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
  info "检查格式 (cargo fmt --all -- --check) ..."
  if ! cargo fmt --all -- --check; then
    die "格式检查未通过：执行 cargo fmt --all 自动修复后重试"
  fi
  ok "格式检查通过"
  info "检查代码 (cargo clippy --all-targets --all-features -- -D warnings) ..."
  cargo clippy --all-targets --all-features -- -D warnings || die "代码检查未通过（详见上方 clippy 输出）"
  ok "代码检查通过，一切正常"
  exit 0
fi

WEB_DIR="$ROOT_DIR/web"
RUN_DIR="$ROOT_DIR/data/run"
EXEC_SOCKET="$RUN_DIR/sock/exec.sock"
EXEC_SECRET="$RUN_DIR/exec.key"
DEV_CONF="$RUN_DIR/zap.dev.yaml"
DEV_USER="$(id -un)"

# URL 前缀：优先沿用已有开发配置里的值；仅首次生成时才从 conf/zap.yaml 带入。
# 开发配置是自洽的（配置与证书都在 data/run/ 下），运行期不依赖 conf/ 目录。
CONF_URL_PREFIX=""
if [ -f "$DEV_CONF" ]; then
  CONF_URL_PREFIX=$(sed -n 's/^[[:space:]]*url_prefix:[[:space:]]*"\{0,1\}\([^"#]*\)"\{0,1\}[[:space:]]*$/\1/p' \
    "$DEV_CONF" | head -1 | sed 's/[[:space:]]*$//')
elif [ -f "$ROOT_DIR/conf/zap.yaml" ]; then
  CONF_URL_PREFIX=$(sed -n 's/^[[:space:]]*url_prefix:[[:space:]]*"\{0,1\}\([^"#]*\)"\{0,1\}[[:space:]]*$/\1/p' \
    "$ROOT_DIR/conf/zap.yaml" | head -1 | sed 's/[[:space:]]*$//')
fi

# 开发用自签证书（与配置同目录，首次启动自动生成，不进 git）
DEV_CRT="$RUN_DIR/zap.crt"
DEV_KEY="$RUN_DIR/zap.key"

if [ "$RELEASE" = true ]; then
  BIN_DIR="$ROOT_DIR/target/release"
  CARGO_FLAGS=(--release)
else
  BIN_DIR="$ROOT_DIR/target/debug"
  CARGO_FLAGS=()
fi

# ── 依赖检查 ────────────────────────────────────────────────
command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
if [ "$SKIP_WEB" != true ]; then
  command -v npm >/dev/null 2>&1 || die "未找到 npm，请先安装 Node.js"
fi

# ── sudo 授权（zapexec 需要 root）───────────────────────────
if [ "$(id -u)" -eq 0 ]; then
  SUDO_CMD=()
else
  command -v sudo >/dev/null 2>&1 || die "未找到 sudo，请以 root 运行或安装 sudo"
  SUDO_CMD=(sudo -n)
  if ! sudo -n true 2>/dev/null; then
    info "zapexec 需要 root 权限，请在提示时输入 sudo 密码"
    sudo -v || die "sudo 授权失败"
  fi
fi

# ── 1. 构建前端 ─────────────────────────────────────────────
if [ "$SKIP_WEB" = true ]; then
  warn "跳过前端构建"
else
  if [ ! -d "$WEB_DIR/node_modules" ] && [ "$SKIP_INSTALL" != true ]; then
    info "未检测到 node_modules，执行 npm install ..."
    (cd "$WEB_DIR" && npm install) || die "npm install 失败"
  fi
  info "构建前端 (npm run build:prod) ..."
  (cd "$WEB_DIR" && npm run build:prod) || die "前端构建失败"
  ok "前端构建完成 -> $WEB_DIR/dist"
fi

# ── 2. 构建后端 ─────────────────────────────────────────────
if [ "$SKIP_BUILD" = true ]; then
  warn "跳过后端构建"
else
  if [ "$SKIP_CHECK" = true ]; then
    # 只跳过 fmt / clippy，构建照做：代码能编译就能跑，只是少了规范把关
    warn "跳过 fmt / clippy 检查（--skip-check）"
  else
    info "格式化代码 (cargo fmt --all) ..."
    cargo fmt --all
    info "检查代码 (cargo clippy --all-targets --all-features -- -D warnings) ..."
    cargo clippy --all-targets --all-features -- -D warnings || die "代码检查失败"
  fi
  info "构建后端 (cargo build ${CARGO_FLAGS[*]} --bin zapd --bin zapexec --bin zapctl --bin zapupgrade) ..."
  cargo build "${CARGO_FLAGS[@]}" --bin zapd --bin zapexec --bin zapctl --bin zapupgrade || die "后端构建失败"
  ok "后端构建完成 -> $BIN_DIR"
fi

# ── 3. 准备开发运行时目录与配置 ─────────────────────────────
mkdir -p "$RUN_DIR"
if [ ! -f "$DEV_CONF" ]; then
  info "生成开发配置 $DEV_CONF"
  cat > "$DEV_CONF" <<EOF
server:
  address: 0.0.0.0
  port: 2600
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
  ok "开发配置已生成"
fi

# 已存在的开发配置（首次生成后不再重建）：同步 conf/zap.yaml 的 url_prefix，
# 否则在 conf/zap.yaml 里改前缀不会生效。
if [ -f "$DEV_CONF" ]; then
  if grep -qE '^[[:space:]]*url_prefix:' "$DEV_CONF"; then
    sed -i "s|^[[:space:]]*url_prefix:.*|  url_prefix: \"$CONF_URL_PREFIX\"|" "$DEV_CONF"
  else
    sed -i "0,\|^[[:space:]]*key_file:.*|s||&\n  url_prefix: \"$CONF_URL_PREFIX\"|" "$DEV_CONF"
  fi
  if [ -n "$CONF_URL_PREFIX" ]; then
    ok "URL 前缀: /$CONF_URL_PREFIX/ （同步自 conf/zap.yaml）"
  fi
fi

# ── 3.5 删除数据库（--reset-db）──────────────────────────────
if [ "$RESET_DB" = true ]; then
  if [ -f "$ROOT_DIR/data/zap.db" ]; then
    warn "删除数据库 $ROOT_DIR/data/zap.db ..."
    rm -f "$ROOT_DIR/data/zap.db" "$ROOT_DIR/data/zap.db-wal" "$ROOT_DIR/data/zap.db-shm"
    ok "数据库已删除，启动时将重建全新数据库"
  else
    info "数据库不存在，跳过删除"
  fi
fi

# ── 清理与退出 ──────────────────────────────────────────────
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
# 注入 ZAP_PATH 使 zapexec 数据目录（$ZAP_PATH/data）与 zapd 配置 db.path 的父目录保持一致，
# 否则 zapexec 默认 /usr/local/zap 会导致日志/源/安装目录错位。
# 软件安装根：默认 /usr/local/apps；若用户在 shell export 了 ZAP_APPS_DIR 则透传。
ZAPEXEC_ENVS=(env ZAP_PATH="$ROOT_DIR")
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
if [ "$RESET_DB" = true ]; then
  info "注入 ZAP_ADMIN_PASSWORD=A123456（全新数据库 admin 初始密码）"
  ZAP_ADMIN_PASSWORD="A123456" ZAP_CONFIG="$DEV_CONF" "$BIN_DIR/zapd" &
else
  ZAP_CONFIG="$DEV_CONF" "$BIN_DIR/zapd" &
fi
ZAPD_PID=$!

echo ""
ok "全部服务已启动"
if [ "$RESET_DB" = true ]; then
  info "  zapd    : https://127.0.0.1:2600 （默认 admin / A123456，请登录后尽快修改）"
else
  info "  zapd    : https://127.0.0.1:2600 （默认 admin / 123456）"
fi
info "  zapexec : $EXEC_SOCKET （root 特权守护进程）"
info "  zapupgrade: $BIN_DIR/zapupgrade （系统升级器，开发环境为手动重启模式）"
info "  按 Ctrl+C 停止全部服务"
echo ""

# ── 6. 等待任一服务退出 ─────────────────────────────────────
wait -n "$ZAPEXEC_PID" "$ZAPD_PID" || true
