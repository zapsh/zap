// SPDX-License-Identifier: AGPL-3.0-only
//! 构建期版本元信息注入。
//!
//! 通过 vergen 把构建日期、git 提交信息、rustc 版本等写入编译期环境变量
//! （`cargo:rustc-env=VERGEN_*`），运行时由 `/api/system/about` 读取并展示在
//! Dashboard 的 About Zap 卡片上。
//!
//! 非 git 环境（发布 tarball、CI 无 .git 目录）下 vergen 取不到 git 信息，
//! vergen 默认容错：缺失字段写入占位值 `VERGEN_IDEMPOTENT_OUTPUT`，
//! 由代码侧统一替换为 `unknown`。
use anyhow::Result;
use vergen::EmitBuilder;

fn main() -> Result<()> {
    // 每次构建都重新生成时间戳 / git 状态
    println!("cargo:rerun-if-changed=build.rs");

    EmitBuilder::builder()
        .all_build()
        .all_cargo()
        .all_git()
        .all_rustc()
        .emit()?;

    Ok(())
}
