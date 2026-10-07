// SPDX-License-Identifier: AGPL-3.0-only
#[derive(Debug, Clone, PartialEq)]
pub enum ZapValue {
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    Bool(bool),
    String(String),
    Null,
    Raw(String),
}
