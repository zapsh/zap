// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 语言切换 composable。
 *
 * 除了应用自身的语言包，还要把 **Element Plus 组件库的语言包**一起换掉，
 * 否则会出现「页面是英文、分页器/日期选择器还是中文」的割裂（App.vue 用
 * el-config-provider 绑定这里的 elementLocale）。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import zhCn from 'element-plus/dist/locale/zh-cn.mjs'
import en from 'element-plus/dist/locale/en.mjs'

import { DEFAULT_LOCALE, LOCALE_OPTIONS, setLocale, type Locale, type LocaleOption } from '@/i18n'

/** Element Plus 语言包：新增语言时在这里补一行即可 */
const ELEMENT_LOCALES: Record<Locale, unknown> = {
  'zh-CN': zhCn,
  'en-US': en,
}

export function useLocale() {
  // 全局作用域：与 i18n 实例共享同一个 locale ref，setLocale 改了它这里立刻响应
  const { locale } = useI18n({ useScope: 'global' })

  const current = computed(() => locale.value as Locale)

  return {
    /** 当前语言 */
    locale: current,
    /** 可选语言列表（切菜单用） */
    options: LOCALE_OPTIONS as LocaleOption[],
    /** 切换语言：同时更新语言包、本地记忆与 <html lang> */
    change(next: Locale) {
      setLocale(next)
    },
    /** 交给 el-config-provider */
    elementLocale: computed(
      () => ELEMENT_LOCALES[current.value] ?? ELEMENT_LOCALES[DEFAULT_LOCALE],
    ),
  }
}
