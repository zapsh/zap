// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 权限点文案。
 *
 * 后端 `permission-catalog` 下发的是**中文 label**（取自 `zapd/src/routers/access.rs`
 * 的 `NS_LABELS` / `ACTION_LABELS`），前端无法直接用。
 * 但 ns（`system.cloud`）与 action（`view`）本身是稳定的英文标识符，所以这里
 * **按标识符查语言包**，无需像菜单标题那样做中文反查，后端也不必改。
 *
 * 语言包位于 `i18n/locales/*.ts` 的 `perm` 命名空间；查不到时原样返回标识符，
 * 保证后端新增权限点、前端还没补翻译时界面不会空白。
 */
import { t, te } from '@/i18n'

/**
 * 命名空间标签：`system.cloud` → 「云存储」/「Cloud Storage」。
 *
 * i18n 路径以 `.` 分层，而 ns 自带 `.`，故把 `.` 换成 `_` 再查
 * （语言包里的 key 即 `perm.ns.system_cloud`）。
 */
export function permNsLabel(ns: string): string {
  const key = `perm.ns.${ns.replace(/\./g, '_')}`
  return te(key) ? t(key) : ns
}

/** 动作标签：`view` → 「查看」/「View」 */
export function permActionLabel(action: string): string {
  const key = `perm.action.${action}`
  return te(key) ? t(key) : action
}

/** 取出权限点的动作部分：`system.cloud:view` → `view` */
export function permActionOf(permKey: string): string {
  return permKey.split(':')[1] ?? 'view'
}

/** 完整权限点：`system.cloud:view` → 「云存储 · 查看」 */
export function permKeyLabel(permKey: string): string {
  const [ns] = permKey.split(':')
  return `${permNsLabel(ns)} · ${permActionLabel(permActionOf(permKey))}`
}

/** 权限分组标题（同上，语义化别名） */
export const permGroupLabel = permNsLabel
