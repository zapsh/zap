// SPDX-License-Identifier: AGPL-3.0-only
/// 主题模式：浅色 / 深色 / 跟随系统。
///
/// 深色基于 Element Plus 官方方案：`<html>` 上加 `.dark` class，并引入
/// `element-plus/theme-chalk/dark/css-vars.css`（见 main.ts）。
/// 自定义样式里请统一使用 `--el-*` CSS 变量，两套主题会自动适配。
import { ref } from 'vue'

export type ThemeMode = 'light' | 'dark' | 'auto'

const STORAGE_KEY = 'zap-theme-mode'

const media = window.matchMedia('(prefers-color-scheme: dark)')

function loadMode(): ThemeMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY)
    if (v === 'light' || v === 'dark' || v === 'auto') return v
  } catch {
    // localStorage 不可用（隐私模式等）时忽略
  }
  return 'auto'
}

/** 用户选择的模式（auto = 跟随系统） */
export const themeMode = ref<ThemeMode>(loadMode())

/** 当前实际生效的是否为深色（auto 模式下随系统变化） */
export const isDark = ref(false)

function apply() {
  const dark =
    themeMode.value === 'dark' || (themeMode.value === 'auto' && media.matches)
  isDark.value = dark
  const el = document.documentElement
  el.classList.toggle('dark', dark)
  // 让浏览器原生控件（滚动条、表单控件等）同步配色
  el.style.setProperty('color-scheme', dark ? 'dark' : 'light')
}

export function setThemeMode(mode: ThemeMode) {
  themeMode.value = mode
  try {
    localStorage.setItem(STORAGE_KEY, mode)
  } catch {
    // 忽略写入失败
  }
  apply()
}

// 系统主题变化时，auto 模式实时跟随
media.addEventListener('change', () => {
  if (themeMode.value === 'auto') apply()
})

apply()
