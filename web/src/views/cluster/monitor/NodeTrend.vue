<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
/**
 * 单节点历史曲线（大屏下钻用）。
 *
 * 数据全部读主控自己的 `node_stats`，**不访问节点**：所以这里没有网络速率图 ——
 * 网络是主控侧 job 采的本机指标，跨节点要额外接口，第一版不做。
 * 图表样式与 `components/SystemMonitor.vue` 保持一致，用户切换本机 / 节点无违和感。
 */
import { computed, nextTick, onMounted, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Chart from 'chart.js/auto'
import { applyChartTheme, watchChartTheme } from '@/utils/chart-theme'
import { getNodeStatus } from '@/api/node'
import { monitorTimeLabel, useMonitorRange } from '@/composables/useMonitorRange'
import MonitorRangePicker from '@/components/MonitorRangePicker.vue'

const props = defineProps<{ nodeId: number }>()

const { t, locale } = useI18n()

const COLORS = {
  cpu: 'rgba(64, 158, 255, 1)',
  memory: 'rgba(103, 194, 58, 1)',
  disk: 'rgba(230, 162, 60, 1)',
  load1: 'rgba(245, 108, 108, 1)',
  load5: 'rgba(230, 162, 60, 1)',
  load15: 'rgba(103, 194, 58, 1)',
}

const canvases = new Map<string, HTMLCanvasElement>()
let cpuChart: Chart
let memoryChart: Chart
let diskChart: Chart
let loadChart: Chart
let destroyed = false

const { range, refreshing, epoch, run, startPolling, stopPolling } = useMonitorRange(
  (r, ep) => fetchStatus(r, ep),
  // 节点数据本身每 15s 才变一次，实时模式按 5s 轮询纯属浪费
  15,
)

/** 四张图：CPU / 内存 / 磁盘使用率 / 系统负载（节点侧不采网络速率） */
const cards = computed(() => [
  { id: 'cpu', title: t('systemMonitor.cpu'), color: COLORS.cpu },
  { id: 'memory', title: t('systemMonitor.memory'), color: COLORS.memory },
  { id: 'disk', title: t('cluster.diskUsage'), color: COLORS.disk },
  { id: 'loadavg', title: t('systemMonitor.systemLoad'), color: COLORS.load1 },
])

function setCanvas(id: string, el: any) {
  if (el) canvases.set(id, el as HTMLCanvasElement)
  else canvases.delete(id)
}

function areaFill(color: string) {
  return (ctx: any) => {
    const chart = ctx?.chart
    if (!chart?.chartArea) return 'transparent'
    const { ctx: c, chartArea } = chart
    const g = c.createLinearGradient(0, chartArea.top, 0, chartArea.bottom)
    g.addColorStop(0, color.replace('1)', '0.3)'))
    g.addColorStop(1, color.replace('1)', '0)'))
    return g
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
          callbacks: { label: (c: any) => `${c.dataset.label}: ${c.parsed.y}%` },
        },
      },
      scales: baseScales(100, (v) => `${v}%`),
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
          callbacks: {
            label: (c: any) => `${c.dataset.label}: ${Number(c.parsed.y).toFixed(2)}`,
          },
        },
      },
      scales: baseScales(undefined, (v) => Number(v).toFixed(1)),
    },
  })
}

function initCharts() {
  const cpu = canvases.get('cpu')
  const memory = canvases.get('memory')
  const disk = canvases.get('disk')
  const load = canvases.get('loadavg')
  if (!cpu || !memory || !disk || !load) return
  applyChartTheme()
  cpuChart = createUsageChart(cpu, t('systemMonitor.cpu'), COLORS.cpu)
  memoryChart = createUsageChart(memory, t('systemMonitor.memory'), COLORS.memory)
  diskChart = createUsageChart(disk, t('cluster.diskUsage'), COLORS.disk)
  loadChart = createLoadChart(load)
}

function render(points: any[]) {
  if (destroyed || !cpuChart) return
  const labels = points.map((p) => monitorTimeLabel(Number(p.created_at), range.value))
  const pick = (key: string) => points.map((p) => Number(Number(p[key] ?? 0).toFixed(2)))
  const cpu = pick('cpu_usage')
  const mem = pick('memory_usage')
  const disk = pick('disk_usage')
  const l1 = pick('loadavg_one')
  const l5 = pick('loadavg_five')
  const l15 = pick('loadavg_fifteen')

  cpuChart.data.labels = labels
  cpuChart.data.datasets[0].data = cpu
  cpuChart.update('none')
  memoryChart.data.labels = labels
  memoryChart.data.datasets[0].data = mem
  memoryChart.update('none')
  diskChart.data.labels = labels
  diskChart.data.datasets[0].data = disk
  diskChart.update('none')
  loadChart.data.labels = labels
  loadChart.data.datasets[0].data = l1
  loadChart.data.datasets[1].data = l5
  loadChart.data.datasets[2].data = l15
  loadChart.update('none')
}

async function fetchStatus(rangeValue?: string, reqEpoch?: number) {
  try {
    const resp = await getNodeStatus(props.nodeId, rangeValue)
    if (reqEpoch !== undefined && reqEpoch !== epoch.value) return // 范围已切换，丢弃过期响应
    if (destroyed || resp?.code !== 0) return
    render(resp.data?.points || [])
  } catch {
    /* 轮询失败不打断后续刷新 */
  }
}

function resizeCharts() {
  ;[cpuChart, memoryChart, diskChart, loadChart].forEach((c) => c?.resize())
}

/** 切换语言时同步图例文案 */
watch(locale, () => {
  if (cpuChart) cpuChart.data.datasets[0].label = t('systemMonitor.cpu')
  if (memoryChart) memoryChart.data.datasets[0].label = t('systemMonitor.memory')
  if (diskChart) diskChart.data.datasets[0].label = t('cluster.diskUsage')
  if (loadChart) {
    const labels = [
      t('systemMonitor.load1m'),
      t('systemMonitor.load5m'),
      t('systemMonitor.load15m'),
    ]
    loadChart.data.datasets.forEach((d: any, i: number) => (d.label = labels[i]))
  }
  ;[cpuChart, memoryChart, diskChart, loadChart].forEach((c) => c?.update('none'))
})

/** MonitorRangePicker 直接写 range.value（不走 setRange），取数与轮询开关要在这里接上 */
watch(range, (v) => {
  if (v === 'live') startPolling()
  else stopPolling()
  run()
})

/** 下钻切节点时重建数据（canvas 复用，只重画） */
watch(
  () => props.nodeId,
  () => run(),
)

onMounted(async () => {
  // 抽屉有展开动画，等 DOM 定位完成再建图，否则 canvas 尺寸会算成 0
  await nextTick()
  initCharts()
  await run()
  if (destroyed) return
  startPolling()
  window.addEventListener('resize', resizeCharts)
})

watchChartTheme(() => [cpuChart, memoryChart, diskChart, loadChart])

onUnmounted(() => {
  destroyed = true
  window.removeEventListener('resize', resizeCharts)
  stopPolling()
  cpuChart?.destroy()
  memoryChart?.destroy()
  diskChart?.destroy()
  loadChart?.destroy()
  canvases.clear()
})
</script>

<template>
  <div class="node-trend">
    <div class="trend-toolbar">
      <MonitorRangePicker v-model="range" />
      <el-button link type="primary" :loading="refreshing" @click="run">
        {{ t('monitorRange.refresh') }}
      </el-button>
    </div>
    <el-alert
      type="info"
      :closable="false"
      show-icon
      :title="t('cluster.noNetworkHint')"
      class="trend-hint"
    />
    <el-row v-loading="refreshing" :gutter="16">
      <el-col v-for="card in cards" :key="card.id" :xs="24" :md="12" class="chart-col">
        <el-card shadow="never" class="chart-card">
          <template #header>
            <div class="chart-title">
              <span class="chart-dot" :style="{ background: card.color }"></span>
              <span>{{ card.title }}</span>
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

<style scoped>
.trend-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.trend-hint {
  margin-bottom: 12px;
}
.chart-col {
  margin-bottom: 16px;
}
.chart-card {
  border-radius: 10px;
}
.chart-card :deep(.el-card__header) {
  padding: 12px 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.chart-card :deep(.el-card__body) {
  padding: 10px 12px 12px;
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
.chart-body {
  position: relative;
  height: 190px;
}
.chart-canvas {
  width: 100% !important;
  height: 100% !important;
}
</style>
