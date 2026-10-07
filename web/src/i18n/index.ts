// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 国际化（i18n）基础设施。
 *
 * 用法速查：
 * - 组件模板：`{{ $t('common.save') }}`（`globalInjection` 已开启，免 import）
 * - 组件 script：`const { t } = useI18n()`，然后在 computed / 模板里使用（响应式）
 * - 组件之外（axios 拦截器、store、路由守卫）：`import { t } from '@/i18n'`
 * - 菜单 / 路由标题：`translateTitle(value)`（兼容后端下发的中文标题）
 *
 * 新增文案：先改 `locales/zh-CN.ts`，`en-US.ts` 会因类型约束报缺失，从而不漏译。
 */
import { createI18n } from 'vue-i18n'
import type { App } from 'vue'

import zhCN from './locales/zh-CN'
import enUS from './locales/en-US'

/** 语言记忆的 localStorage key */
export const LOCALE_STORAGE_KEY = 'zap-locale'

export type Locale = 'zh-CN' | 'en-US'

export interface LocaleOption {
  value: Locale
  /** 语言自称：用本民族语言书写，界面已是英文时也能一眼找到「中文」 */
  label: string
}

export const LOCALE_OPTIONS: LocaleOption[] = [
  { value: 'zh-CN', label: '简体中文' },
  { value: 'en-US', label: 'English' },
]

export const DEFAULT_LOCALE: Locale = 'zh-CN'

export const messages = {
  'zh-CN': zhCN,
  'en-US': enUS,
}

function isLocale(value: unknown): value is Locale {
  return LOCALE_OPTIONS.some((item) => item.value === value)
}

/** 语言优先级：本次记忆 > 浏览器语言 > 默认中文 */
function detectLocale(): Locale {
  try {
    const saved = localStorage.getItem(LOCALE_STORAGE_KEY)
    if (isLocale(saved)) return saved
  } catch {
    // 无痕模式 / 禁用存储：忽略，退回浏览器语言
  }
  const nav = typeof navigator !== 'undefined' ? navigator.language : ''
  return nav && nav.toLowerCase().startsWith('en') ? 'en-US' : DEFAULT_LOCALE
}

export const i18n = createI18n({
  // Composition API 模式：组件里 useI18n()，模板可直接用 $t
  legacy: false,
  globalInjection: true,
  locale: detectLocale(),
  fallbackLocale: DEFAULT_LOCALE,
  messages,
})

/**
 * 只取用得到的三个成员：`i18n.global` 的联合类型（Composer | VueI18n）
 * 在 TS 下无法直接调用 `t`，这里收敛成一个最小接口。
 */
interface ComposerLike {
  t(key: string, named?: Record<string, unknown>): string
  te(key: string): boolean
  locale: { value: string }
}

const composer = i18n.global as unknown as ComposerLike

/**
 * 组件之外的翻译入口（axios 拦截器、pinia store、路由守卫…）。
 *
 * 注意：它**不参与 Vue 的响应式追踪**，语言切换不会自动重算调用它的 computed。
 * 组件内请用 `useI18n()` 的 `t`，这里只给「非组件环境」用。
 */
export function t(key: string, named?: Record<string, unknown>): string {
  return named ? composer.t(key, named) : composer.t(key)
}

/** key 是否存在于语言包（用于「能翻译就翻译，否则原样输出」的兜底） */
export function te(key: string): boolean {
  return composer.te(key)
}

export function getLocale(): Locale {
  const current = composer.locale.value
  return isLocale(current) ? current : DEFAULT_LOCALE
}

/** 切换语言：i18n 实例 + localStorage + `<html lang>` 一起改 */
export function setLocale(locale: Locale) {
  composer.locale.value = locale
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, locale)
  } catch {
    // 存不下也无妨，本次会话内依然生效
  }
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('lang', locale)
  }
}

/**
 * 「中文标题 → i18n key」反查表。
 *
 * 为什么需要它：侧边栏 / 面包屑 / 标签页的标题来自**后端菜单表**（`menus.title` 存的是中文），
 * 前端拿不到语言信息。这里以 `menu` 命名空间的中文原文为桥，让后端无需改造也能跟随语言切换。
 * 标题重名（一级与二级同名）时保留先出现的：两者译文一致，不影响观感。
 */
/**
 * 归一化标题用于比对：忽略空白差异。
 * 例：静态路由写 `SSL 证书`、后端菜单写 `SSL证书`，两者应命中同一个 key。
 */
function normalizeTitle(value: string) {
  return value.replace(/\s+/g, '')
}

const titleToKey = new Map<string, string>()
for (const [key, label] of Object.entries(messages[DEFAULT_LOCALE].menu)) {
  const normalized = normalizeTitle(label)
  if (!titleToKey.has(normalized)) titleToKey.set(normalized, `menu.${key}`)
}

/**
 * 翻译路由 / 菜单标题。
 *
 * 依次尝试：i18n key（后端也可直接下发 `menu.user`）→ 中文原文反查 → 原样返回。
 * 最后一步兜底很关键：管理员自定义的菜单标题不该被吞掉。
 */
export function translateTitle(raw?: string | null): string {
  if (!raw) return ''
  if (te(raw)) return t(raw)
  const key = titleToKey.get(normalizeTitle(raw))
  return key ? t(key) : raw
}

/** 在 main.ts 挂载 i18n */
export function setupI18n(app: App) {
  app.use(i18n)
  // 首次进入时对齐 <html lang>（setLocale 只在主动切换时设置）
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('lang', getLocale())
  }
}
