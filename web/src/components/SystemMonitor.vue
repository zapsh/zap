<template>
  <div class="system-monitor">
    <!-- 时间范围工具条：实时高频刷新；历史范围一次加载降采样数据，手动刷新 -->
    <div class="monitor-toolbar">
      <MonitorRangePicker v-model="range" />
      <div v-if="!isLive" class="asof">
        <span class="asof-text">
          {{ refreshing ? t('monitorRange.loading') : t('monitorRange.asOf', { time: asOfText }) }}
        </span>
        <el-button link type="primary" :loading="refreshing" @click="run">
          {{ t('monitorRange.refresh') }}
        </el-button>
      </div>
    </div>
    <el-row v-loading="refreshing" :gutter="16">
      <el-col v-for="card in cards" :key="card.id" :xs="24" :md="12" class="chart-col">
        <el-card shadow="never" class="chart-card">
          <template #header>
            <div class="chart-header">
              <div class="chart-title">
                <span class="chart-dot" :style="{ background: card.color }"></span>
                <span>{{ card.title }}</span>
              </div>
              <div class="chart-meta">
                <span class="chart-value">{{ card.value }}</span>
                <span v-if="showRefreshHint && isLive" class="refresh-hint">
                  {{ t('systemMonitor.autoRefresh', { sec: countdown }) }}
                </span>
              </div>
            </div>
          </template>
          <div class="chart-body">
            <canvas :ref="(el) => setCanvas(card.id, el)" class="chart-canvas"></canvas>
          </div>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts">
/**
 * 系统实时监控图表（CPU / 内存 / 系统负载 / 网络）。
 *
 * 数据来自 `/system/status`，由组件自行按 `refreshSecs` 轮询；
 * 首页已不再内嵌本组件，现由「服务器状态 → Server Monitor」使用。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Chart from 'chart.js/auto'
import { applyChartTheme, watchChartTheme } from '@/utils/chart-theme'
import { isArray } from '@/utils/validate'
import { formatBytes } from '@/utils/fmt'
import { getRTStatus } from '@/api/dashboard.ts'
import { monitorTimeLabel, useMonitorRange } from '@/composables/useMonitorRange'
import MonitorRangePicker from '@/components/MonitorRangePicker.vue'

const props = withDefaults(
  defineProps<{
    /** 轮询间隔（秒） */
    refreshSecs?: number
    /** 是否在每张图表右上角显示刷新倒计时 */
    showRefreshHint?: boolean
  }>(),
  { refreshSecs: 5, showRefreshHint: false },
)

/** 实时数据回传：调用方（仪表盘）用它更新环形指标等 */
const emit = defineEmits<{ (e: 'stats', data: Record<string, any>): void }>()

const { t, locale } = useI18n()

const COLORS = {
  cpu: 'rgba(64, 158, 255, 1)',
  memory: 'rgba(103, 194, 58, 1)',
  load1: 'rgba(245, 108, 108, 1)',
  load5: 'rgba(230, 162, 60, 1)',
  load15: 'rgba(103, 194, 58, 1)',
  up: 'rgba(64, 158, 255, 1)',
  down: 'rgba(103, 194, 58, 1)',
}

const countdown = ref(props.refreshSecs)
const canvases = new Map<string, HTMLCanvasElement>()
const latest = ref({ cpu: '-', memory: '-', load: '-', up: '-', down: '-' })

// ── 时间范围：live = 5s 倒计时轮询；历史范围一次加载 + 手动刷新 ──
const lastTs = ref(0)
const { range, isLive, refreshing, epoch, run } = useMonitorRange(
  (r, ep) => fetchRTStatus(r, ep),
  props.refreshSecs,
)
const asOfText = computed(() =>
  lastTs.value ? new Date(lastTs.value * 1000).toLocaleString() : '-',
)

let cpu_chart: Chart
let memory_chart: Chart
let loadavg_chart: Chart
let network_chart: Chart
let countdown_timer: any
let destroyed = false

const cards = computed(() => [
  { id: 'cpu', title: t('systemMonitor.cpu'), color: COLORS.cpu, value: latest.value.cpu },
  { id: 'memory', title: t('systemMonitor.memory'), color: COLORS.memory, value: latest.value.memory },
  { id: 'loadavg', title: t('systemMonitor.systemLoad'), color: COLORS.load1, value: latest.value.load },
  {
    id: 'network',
    title: t('systemMonitor.network'),
    color: COLORS.up,
    value: `↑ ${latest.value.up} ↓ ${latest.value.down}`,
  },
])

/** canvas 用 ref 收集（不用 id，避免同页多实例冲突） */
function setCanvas(id: string, el: any) {
  if (el) canvases.set(id, el as HTMLCanvasElement)
  else canvases.delete(id)
}

/** 网络速率原始数据是字节数，统一按 1024 自适应成 B/s、KB/s、MB/s… */
function fmtSpeed(bytes: number) {
  return `${formatBytes(bytes)}/s`
}

/** 纵向渐变填充：顶部有色、底部透明，比纯色块更清爽 */
function areaFill(color: string) {
  return (ctx: any) => {
    const chart = ctx?.chart
    if (!chart?.chartArea) return 'transparent'
    const { ctx: c, chartArea } = chart
    const gradient = c.createLinearGradient(0, chartArea.top, 0, chartArea.bottom)
    gradient.addColorStop(0, color.replace('1)', '0.3)'))
    gradient.addColorStop(1, color.replace('1)', '0)'))
    return gradient
  }
}

function tooltipStyle() {
  return {
    backgroundColor: 'rgba(0, 0, 0, 0.78)',
    titleColor: '#fff',
    bodyColor: '#fff',
    padding: 10,
    cornerRadius: 6,
    displayColors: true,
    boxWidth: 8,
    boxHeight: 8,
    usePointStyle: true,
    borderWidth: 0,
    titleFont: { size: 12 },
    bodyFont: { size: 12 },
  }
}

function legendStyle() {
  return {
    display: true,
    align: 'end' as const,
    labels: {
      usePointStyle: true,
      pointStyle: 'circle' as const,
      boxWidth: 8,
      boxHeight: 8,
      padding: 14,
      font: { size: 12 },
    },
  }
}

/** 去掉网格竖线、隐藏坐标轴边框，只留淡淡的横向参考线 */
function baseScales(max?: number, tick?: (value: any) => string) {
  return {
    x: {
      grid: { display: false },
      border: { display: false },
      ticks: { maxTicksLimit: 6, autoSkip: true, maxRotation: 0, padding: 6 },
    },
    y: {
      min: 0,
      max,
      beginAtZero: true,
      grid: { color: 'rgba(128, 128, 128, 0.15)', drawTicks: false },
      border: { display: false },
      ticks: { maxTicksLimit: 5, padding: 8, callback: tick },
    },
  }
}

/** 通用折线数据集：细线 + 平滑曲线 + 悬停才显示圆点 */
function lineDataset(label: string, color: string, fill: boolean): any {
  return {
    label,
    data: [],
    borderColor: color,
    backgroundColor: fill ? areaFill(color) : 'transparent',
    fill,
    tension: 0.35,
    borderWidth: 2,
    pointRadius: 0,
    pointHoverRadius: 4,
    pointHoverBorderWidth: 2,
    pointHoverBackgroundColor: color,
    pointHoverBorderColor: '#fff',
  }
}

function createUsageChart(canvas: HTMLCanvasElement, label: string, color: string) {
  return new Chart(canvas, {
    type: 'line',
    data: { labels: [], datasets: [lineDataset(label, color, true)] },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      animation: { duration: 400 },
      interaction: { mode: 'index' as const, intersect: false },
      plugins: {
        legend: { display: false },
        tooltip: {
          ...tooltipStyle(),
          callbacks: { label: (context: any) => `${context.dataset.label}: ${context.parsed.y}%` },
        },
      },
      scales: baseScales(100, (value) => `${value}%`),
    },
  })
}

function createLoadChart(canvas: HTMLCanvasElement) {
  return new Chart(canvas, {
    type: 'line',
    data: {
      labels: [],
      datasets: [
        lineDataset(t('systemMonitor.load1m'), COLORS.load1, false),
        lineDataset(t('systemMonitor.load5m'), COLORS.load5, false),
        lineDataset(t('systemMonitor.load15m'), COLORS.load15, false),
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      animation: { duration: 400 },
      interaction: { mode: 'index' as const, intersect: false },
      plugins: {
        legend: legendStyle(),
        tooltip: {
          ...tooltipStyle(),
          callbacks: { label: (context: any) => `${context.dataset.label}: ${Number(context.parsed.y).toFixed(2)}` },
        },
      },
      scales: baseScales(undefined, (value) => Number(value).toFixed(1)),
    },
  })
}

function createNetworkChart(canvas: HTMLCanvasElement) {
  return new Chart(canvas, {
    type: 'line',
    data: {
      labels: [],
      datasets: [
        lineDataset(`↑ ${t('systemMonitor.upload')}`, COLORS.up, true),
        lineDataset(`↓ ${t('systemMonitor.download')}`, COLORS.down, true),
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      animation: { duration: 400 },
      interaction: { mode: 'index' as const, intersect: false },
      plugins: {
        legend: legendStyle(),
        tooltip: {
          ...tooltipStyle(),
          callbacks: { label: (context: any) => `${context.dataset.label}: ${fmtSpeed(context.parsed.y * 1024)}` },
        },
      },
      scales: baseScales(undefined, (value) => fmtSpeed(Number(value) * 1024)),
    },
  })
}

function initCharts() {
  const cpu = canvases.get('cpu')
  const memory = canvases.get('memory')
  const loadavg = canvases.get('loadavg')
  const network = canvases.get('network')
  if (!cpu || !memory || !loadavg || !network) return

  applyChartTheme()
  cpu_chart = createUsageChart(cpu, t('systemMonitor.cpu'), COLORS.cpu)
  memory_chart = createUsageChart(memory, t('systemMonitor.memory'), COLORS.memory)
  loadavg_chart = createLoadChart(loadavg)
  network_chart = createNetworkChart(network)
}

function timeLabel(ts: number) {
  return monitorTimeLabel(ts, range.value)
}

function updateCharts(resp: Record<string, any>) {
  if (destroyed || !cpu_chart) return
  const stats = isArray(resp.system_stats) ? resp.system_stats : []
  const labels: string[] = []
  const cpu_data: number[] = []
  const memory_data: number[] = []
  const one_data: number[] = []
  const five_data: number[] = []
  const fifteen_data: number[] = []

  stats.forEach((element: Record<string, any>) => {
    labels.push(timeLabel(element.created_at))
    cpu_data.push(parseFloat(Number(element.cpu_usage || 0).toFixed(2)))
    memory_data.push(parseFloat(Number(element.memory_usage || 0).toFixed(2)))
    one_data.push(Number(Number(element.loadavg_one || 0).toFixed(2)))
    five_data.push(Number(Number(element.loadavg_five || 0).toFixed(2)))
    fifteen_data.push(Number(Number(element.loadavg_fifteen || 0).toFixed(2)))
  })

  cpu_chart.data.labels = labels
  cpu_chart.data.datasets[0].data = cpu_data
  cpu_chart.update('none')

  memory_chart.data.labels = labels
  memory_chart.data.datasets[0].data = memory_data
  memory_chart.update('none')

  loadavg_chart.data.labels = labels
  loadavg_chart.data.datasets[0].data = one_data
  loadavg_chart.data.datasets[1].data = five_data
  loadavg_chart.data.datasets[2].data = fifteen_data
  loadavg_chart.update('none')

  const net_stats = isArray(resp.network_stats) ? resp.network_stats : []
  const net_labels: string[] = []
  const up_data: number[] = []
  const down_data: number[] = []
  net_stats.forEach((element: Record<string, any>) => {
    net_labels.push(timeLabel(element.created_at))
    up_data.push(parseFloat((Number(element.transmitted || 0) / 1024).toFixed(2)))
    down_data.push(parseFloat((Number(element.received || 0) / 1024).toFixed(2)))
  })
  network_chart.data.labels = net_labels
  network_chart.data.datasets[0].data = up_data
  network_chart.data.datasets[1].data = down_data
  network_chart.update('none')

  // 卡片右上角的最新值，让用户不悬停也能看到当前状态
  const last = stats[stats.length - 1]
  const lastNet = net_stats[net_stats.length - 1]
  lastTs.value = Number(last?.created_at || 0)
  latest.value = {
    cpu: last ? `${Number(last.cpu_usage || 0).toFixed(1)}%` : '-',
    memory: last ? `${Number(last.memory_usage || 0).toFixed(1)}%` : '-',
    load: last ? Number(last.loadavg_one || 0).toFixed(2) : '-',
    up: lastNet ? fmtSpeed(Number(lastNet.transmitted || 0)) : '-',
    down: lastNet ? fmtSpeed(Number(lastNet.received || 0)) : '-',
  }
}

/** 拉一次状态（实时或按当前范围的历史降采样），成功后刷新图表并回传给调用方 */
async function fetchRTStatus(rangeValue?: string, reqEpoch?: number) {
  try {
    const resp: any = await getRTStatus(
      rangeValue && rangeValue !== 'live' ? { range: rangeValue } : undefined,
    )
    if (reqEpoch !== undefined && reqEpoch !== epoch.value) return // 范围已切换，丢弃过期响应
    if (destroyed || resp?.code !== 0) return
    const data = resp.data || {}
    updateCharts(data)
    emit('stats', data)
  } catch {
    // 轮询失败不打断后续刷新（下次 tick 自动重试）
  }
}

/** 停掉实时倒计时（历史范围下不自动轮询） */
function stopTicker() {
  if (countdown_timer) {
    clearInterval(countdown_timer)
    countdown_timer = undefined
  }
}

/** 切换范围后重置实时倒计时（仅 live 模式有自动轮询） */
watch(range, (v) => {
  if (v === 'live') {
    startTicker()
  } else {
    stopTicker()
  }
})

/** 倒计时 + 到点拉取（组件自行维护，父级无需关心） */
function startTicker() {
  countdown.value = props.refreshSecs
  countdown_timer = setInterval(() => {
    countdown.value--
    if (countdown.value <= 0) {
      countdown.value = props.refreshSecs
      clearInterval(countdown_timer)
      fetchRTStatus().finally(startTicker)
    }
  }, 1000)
}

function resizeCharts() {
  cpu_chart?.resize()
  memory_chart?.resize()
  loadavg_chart?.resize()
  network_chart?.resize()
}

/** 切换语言时同步图例 / 提示里的文案 */
watch(locale, () => {
  if (cpu_chart) cpu_chart.data.datasets[0].label = t('systemMonitor.cpu')
  if (memory_chart) memory_chart.data.datasets[0].label = t('systemMonitor.memory')
  if (loadavg_chart) {
    const labels = [t('systemMonitor.load1m'), t('systemMonitor.load5m'), t('systemMonitor.load15m')]
    loadavg_chart.data.datasets.forEach((dataset, index) => {
      dataset.label = labels[index]
    })
  }
  if (network_chart) {
    network_chart.data.datasets[0].label = `↑ ${t('systemMonitor.upload')}`
    network_chart.data.datasets[1].label = `↓ ${t('systemMonitor.download')}`
  }
  ;[cpu_chart, memory_chart, loadavg_chart, network_chart].forEach((c) => c?.update('none'))
})

onMounted(async () => {
  initCharts()
  await run()
  if (destroyed) return
  startTicker()
  window.addEventListener('resize', resizeCharts)
})

watchChartTheme(() => [cpu_chart, memory_chart, loadavg_chart, network_chart])

onUnmounted(() => {
  destroyed = true
  window.removeEventListener('resize', resizeCharts)
  if (countdown_timer) clearInterval(countdown_timer)
  cpu_chart?.destroy()
  memory_chart?.destroy()
  loadavg_chart?.destroy()
  network_chart?.destroy()
  canvases.clear()
})
</script>

<style scoped>
.monitor-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.asof {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.chart-col {
  margin-bottom: 16px;
}
.chart-card {
  border-radius: 10px;
  transition: box-shadow 0.2s ease;
}
.chart-card:hover {
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.08);
}
.chart-card :deep(.el-card__header) {
  padding: 12px 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.chart-card :deep(.el-card__body) {
  padding: 10px 12px 12px;
}
.chart-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-height: 22px;
}
.chart-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
}
.chart-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.chart-meta {
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.chart-value {
  font-size: 15px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--el-text-color-primary);
  white-space: nowrap;
}
.refresh-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
}
.chart-body {
  position: relative;
  height: 200px;
}
.chart-canvas {
  width: 100% !important;
  height: 100% !important;
}
</style>
