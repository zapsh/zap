import { useI18n } from 'vue-i18n'

export type UserStatusTag = 'success' | 'warning' | 'danger'

/** 账号状态可选值：1=正常 0=已禁用 -1=已封禁 -2=欠费停用 -3=已暂停（可登录，停服） */
export const USER_STATUS_VALUES: number[] = [1, 0, -1, -2, -3]

/**
 * 账号状态 → { 翻译标签, 标签色 }。
 * 标签完全走 i18n（users.statusMap.<code>），新增状态只需补词条。
 */
export function useUserStatus() {
  const { t } = useI18n()
  function meta(code: number): { label: string; type: UserStatusTag } {
    const label =
      (t(`users.statusMap.${code}`, {}, { default: String(code) }) as string) || String(code)
    let type: UserStatusTag = 'success'
    if (code !== 1) type = code === 0 ? 'danger' : 'warning'
    return { label, type }
  }
  return { meta, values: USER_STATUS_VALUES }
}
