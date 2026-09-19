#!/usr/bin/env python3
"""ZAP 应用商店脚本辅助库（Python 版）。

与 `bash_utils.sh` 等价，供 `run_as: user` 的建站类包（WordPress 等）用
python3 编写安装/卸载/升级脚本。zapexec 以 `python3 -I -B <脚本>` 执行：

- `-I` 隔离模式会忽略 PYTHON* 环境变量、不加载用户 site-packages、
  也不把脚本所在目录放进 sys.path，因此必须显式把 `ZAP_PY_LIB` 加进来：

      import os, sys
      sys.path.insert(0, os.environ["ZAP_PY_LIB"])
      from zapweb import *

- `-B` 不生成 __pycache__，家目录不留可执行字节码。

由 zapexec 注入的环境（与 bash 脚本一致）：
  ZAP_PATH APPS_DIR APP_PATH PKG_PATH PKG_SRC_PATH BUILD_PATH APP_NAME APP_VERSION
  LOG_FILE CPU_NUM ZAP_USER ZAP_LINUX_USER ZAP_HOME ZAPCTL ZAP_PY_LIB
降权运行时额外有：HOME USER LOGNAME TMPDIR（指向家目录下的 tmp）。
"""

from __future__ import annotations

import hashlib
import os
import secrets
import shutil
import string
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile
from pathlib import Path

# ── 日志 ──────────────────────────────────────────────────────
# 日志会落盘并回显到面板，密码类参数务必用 mask() 打码后再输出。

_SENSITIVE = ("PASS", "PASSWORD", "SECRET", "TOKEN", "KEY")


def mask(value: str, keep: int = 2) -> str:
    """密码/密钥打码：只保留首尾各 keep 个字符，避免凭据进日志。"""
    if not value:
        return ""
    if len(value) <= keep * 2:
        return "*" * len(value)
    return f"{value[:keep]}{'*' * (len(value) - keep * 2)}{value[-keep:]}"


def _log(level: str, *parts: object) -> None:
    ts = time.strftime("%Y-%m-%d %H:%M:%S")
    print(f"[{ts}] [{level:^5}] " + " ".join(str(p) for p in parts), flush=True)


def log_info(*parts: object) -> None:
    _log("INFO", *parts)


def log_ok(*parts: object) -> None:
    _log(" OK ", *parts)


def log_warn(*parts: object) -> None:
    _log("WARN", *parts)


def log_error(*parts: object) -> None:
    _log("ERROR", *parts)


def die(msg: str, code: int = 1) -> "NoReturn":  # type: ignore[valid-type]
    log_error(msg)
    sys.exit(code)


# ── 环境读取 ──────────────────────────────────────────────────


def env(name: str, default: str = "") -> str:
    return os.environ.get(name, default)


def env_required(name: str) -> str:
    v = os.environ.get(name, "").strip()
    if not v:
        die(f"缺少必需的环境变量: {name}")
    return v


def home() -> Path:
    """运行账号家目录（降权脚本的一切写入都应该发生在这里）。"""
    return Path(env_required("ZAP_HOME"))


def random_password(length: int = 24) -> str:
    """随机密码：字母数字，剔除 0/O/1/l/I 等易混淆字符。"""
    alphabet = "".join(c for c in string.ascii_letters + string.digits if c not in "0O1lI")
    return "".join(secrets.choice(alphabet) for _ in range(length))


# ── 命令执行 ──────────────────────────────────────────────────


def run(cmd: list[str], cwd: str | Path | None = None, check: bool = True) -> str:
    """执行命令并返回 stdout；check=True 时非零退出直接结束脚本。"""
    log_info("$", " ".join(cmd))
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if p.returncode != 0 and check:
        die(f"命令失败 ({p.returncode}): {' '.join(cmd)}\n{p.stderr.strip()}")
    return p.stdout


# ── 目录 / 路径围栏 ───────────────────────────────────────────


def ensure_dir(*paths: str | Path) -> None:
    for p in paths:
        Path(p).mkdir(parents=True, exist_ok=True)


def assert_under(path: str | Path, root: str | Path) -> Path:
    """路径围栏：目标必须落在 root 之内，防止脚本被选项值带出站点目录。"""
    p = Path(path).resolve()
    r = Path(root).resolve()
    if not (p == r or r in p.parents):
        die(f"路径越界：{p} 不在 {r} 之下")
    return p


# ── 下载 / 解压 ───────────────────────────────────────────────


def download(url: str, dest: str | Path, sha256: str = "") -> Path:
    """下载文件；给了 sha256 就强制校验（官方包应在 app.yaml 声明摘要）。"""
    dest = Path(dest)
    ensure_dir(dest.parent)
    run(["curl", "-fsSL", "--retry", "3", "-o", str(dest), url])
    if sha256:
        h = hashlib.sha256(dest.read_bytes()).hexdigest()
        if h.lower() != sha256.lower():
            dest.unlink(missing_ok=True)
            die(f"校验和不匹配：{dest.name}（期望 {sha256[:12]}…，实际 {h[:12]}…）")
        log_ok("校验和一致：", dest.name)
    return dest


def extract(archive: str | Path, dest: str | Path) -> Path:
    """解压 tar.* / zip 到 dest，返回 dest。"""
    archive, dest = Path(archive), Path(dest)
    ensure_dir(dest)
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as z:
            z.extractall(dest)
    else:
        with tarfile.open(archive) as t:
            t.extractall(dest)
    return dest


def single_subdir(path: str | Path) -> Path:
    """解压结果只有一层封装目录时返回该目录，否则返回原目录。"""
    p = Path(path)
    items = [x for x in p.iterdir() if not x.name.startswith(".")]
    if len(items) == 1 and items[0].is_dir():
        return items[0]
    return p


def deploy(src: str | Path, dest: str | Path, root: str | Path) -> Path:
    """把解压结果部署到 dest（必须先通过 root 围栏），已存在时先备份。"""
    src, dest = Path(src), assert_under(dest, root)
    if dest.exists():
        bak = dest.with_name(f"{dest.name}.bak.{int(time.time())}")
        log_warn("目标已存在，备份为", bak)
        dest.rename(bak)
    ensure_dir(dest.parent)
    shutil.move(str(src), str(dest))
    log_ok("已部署：", dest)
    return dest


def chmod_tree(path: str | Path, dir_mode: int = 0o755, file_mode: int = 0o644) -> None:
    p = Path(path)
    for d in p.rglob("*"):
        if d.is_dir():
            d.chmod(dir_mode)
        elif d.is_file():
            d.chmod(file_mode)


# ── 配置文件渲染 ──────────────────────────────────────────────
# 用简单的 `{{NAME}}` 占位替换，值一律原样写入（不做 shell 展开），
# 避免选项值里的 `$()`、反引号被当成命令执行。


def render(template: str | Path, dest: str | Path, mapping: dict[str, str]) -> Path:
    text = Path(template).read_text(encoding="utf-8")
    for k, v in mapping.items():
        text = text.replace("{{%s}}" % k, str(v))
    dest = Path(dest)
    ensure_dir(dest.parent)
    dest.write_text(text, encoding="utf-8")
    try:
        dest.chmod(0o640)  # 配置里可能含数据库密码：不给其它账号读
    except OSError:
        pass
    log_ok("已生成配置：", dest)
    return dest


# ── 实例登记 ──────────────────────────────────────────────────


def write_info(app_path: str | Path, **fields: str) -> Path:
    """写 `info.yaml`（面板「已安装」与实例信息的权威来源）。

    注意：**不要把数据库密码写进这里**，只登记库名/用户名，避免明文落盘。
    """
    import yaml  # 延迟导入：多数脚本不需要

    app_path = Path(app_path)
    ensure_dir(app_path)
    data = dict(fields)
    secret_like = [k for k in data if any(s in k.upper() for s in _SENSITIVE)]
    if secret_like:
        die(f"info.yaml 不允许写入敏感字段: {', '.join(secret_like)}")
    p = app_path / "info.yaml"
    p.write_text(yaml.safe_dump(data, allow_unicode=True, sort_keys=False), encoding="utf-8")
    log_ok("已登记实例信息：", p)
    return p


def tmp_dir(prefix: str = "zapweb-") -> Path:
    """本次运行的临时目录：优先家目录下的 tmp（降权账号私有，不走共享 /tmp）。"""
    zap_home = env("ZAP_HOME")
    base = Path(env("TMPDIR")) if env("TMPDIR") else (Path(zap_home) / "tmp" if zap_home else None)
    if base:
        ensure_dir(base)
        return Path(tempfile.mkdtemp(prefix=prefix, dir=str(base)))
    return Path(tempfile.mkdtemp(prefix=prefix))
