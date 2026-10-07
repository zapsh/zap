// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 角色名称映射（与后端 roles 表 role_key 对应）。
 *
 * 显示名来自语言包的 `role` 命名空间；管理员自建的角色（role_key 不在语言包里）
 * 原样返回标识符，避免界面出现空白。
 *
 * 这里用 `@/i18n` 的全局 `t` 而不是组件内的 `useI18n()`——`t` 在渲染期被调用时会读到
 * 当前 locale，因此模板里依然响应语言切换（原理见 i18n/index.ts 的注释）。
 */
import { t, te } from '@/i18n'

/** 全部内置角色（下拉框用），顺序即展示顺序 */
export const ROLE_KEYS = ['admin', 'reseller', 'user', 'demo'] as const

/** 根据角色标识返回当前语言下的名称，未知角色原样返回 */
export function roleLabel(role: string): string {
  const key = `role.${role}`
  return te(key) ? t(key) : role
}

/** 角色下拉选项（值 + 当前语言下的标签） */
export function roleOptions(): { label: string; value: string }[] {
  return ROLE_KEYS.map((value) => ({ label: roleLabel(value), value }))
}

/** 系统内置角色标识（不可删除、不可禁用、不可修改标识） */
export const BUILTIN_ROLE_KEYS: string[] = [...ROLE_KEYS]

export function isBuiltinRole(roleKey: string): boolean {
  return BUILTIN_ROLE_KEYS.includes(roleKey)
}
