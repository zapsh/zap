// SPDX-License-Identifier: AGPL-3.0-only
/// Chart.js 图表配色跟随面板主题。
///
/// Chart.js 默认的坐标轴文字（#666）与网格线（黑色半透明）在深色模式下几乎不可见，
/// 这里直接读取 Element Plus 当前的 CSS 变量值写回 `Chart.defaults`，
/// 浅色 / 深色都能自动拿到正确的颜色，无需在两处各自维护一套色值。
import { watch } from 'vue'
import type { Chart as ChartType } from 'chart.js'
import Chart from 'chart.js/auto'
import { isDark } from '@/composables/useTheme'

function cssVar(name: string, fallback: string) {
  return (
    getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback
  )
}

/** 按当前主题刷新 Chart.js 默认配色（应在创建图表前调用一次） */
export function applyChartTheme() {
  Chart.defaults.color = cssVar('--el-text-color-secondary', '#909399')
  Chart.defaults.borderColor = cssVar('--el-border-color-lighter', 'rgba(0, 0, 0, 0.1)')
}

/**
 * 主题切换时刷新已有图表：`getCharts()` 返回当前组件里的图表实例，
 * 需要在组件卸载后返回空数组（用可选链即可）。
 */
export function watchChartTheme(getCharts: () => (ChartType | undefined)[]) {
  watch(isDark, () => {
    applyChartTheme()
    for (const c of getCharts()) c?.update()
  })
}
