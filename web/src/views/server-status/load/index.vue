<template>
  <div>
    <el-row :gutter="20">
      <el-col :sm="8" v-for="item in loadCards" :key="item.label">
        <el-card shadow="hover">
          <div class="stat-card">
            <div class="stat-title">{{ item.label }}</div>
            <div class="stat-value" :style="{ color: item.color }">{{ item.value }}</div>
            <div class="stat-sub">
              {{ t('statusLoad.cpuCores', { n: sysinfo.cpu_num || '-' }) }}
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <el-card v-loading="refreshing" shadow="hover" class="mt-4">
      <template #header>
        <div class="card-header">
          <span>{{ t('statusLoad.trend') }}</span>
          <MonitorRangePicker v-model="range" />
        </div>
      </template>
      <canvas id="loadavg_chart" style="width: 100%; height: 240px"></canvas>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import Chart from 'chart.js/auto'
import { applyChartTheme, watchChartTheme } from '@/utils/chart-theme'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { getRTStatus, getSystemInfo } from '@/api/dashboard.ts'
import MonitorRangePicker from '@/components/MonitorRangePicker.vue'
import { monitorTimeLabel, useMonitorRange } from '@/composables/useMonitorRange'

const { t } = useI18n()

const sysinfo: Record<string, any> = ref({})
const loadValues = ref(['-', '-', '-'])
const loadCards = computed(() => [
  { label: t('statusLoad.load1'), value: loadValues.value[0], color: '#f56c6c' },
  { label: t('statusLoad.load5'), value: loadValues.value[1], color: '#e6a23c' },
  { label: t('statusLoad.load15'), value: loadValues.value[2], color: '#67c23a' },
])

let loadavg_chart: Chart
let timer: ReturnType<typeof setInterval> | undefined
let destroyed = false

// 时间范围：实时高频轮询；历史范围一次加载降采样数据（不自动轮询）
const { range, isLive, refreshing, epoch, run, startPolling, stopPolling } = useMonitorRange(
  (r, ep) => FetchRTStatus(r, ep),
  5,
)

const fmtLoad = (v: number) => (v == null ? '-' : v.toFixed(2))

onMounted(async () => {
  await loadSystemInfo()
  if (destroyed) return
  const container = document.getElementById('loadavg_chart') as HTMLCanvasElement
  applyChartTheme()
  loadavg_chart = new Chart(container, {
    type: 'line',
    data: {
      labels: [],
      datasets: [
        {
          label: '1 M',
          data: [],
          fill: false,
          borderColor: 'rgba(245, 108, 108, 1)',
          tension: 0.1,
          pointRadius: 0,
        },
        {
          label: '5 M',
          data: [],
          fill: false,
          borderColor: 'rgba(230, 162, 60, 1)',
          tension: 0.1,
          pointRadius: 0,
        },
        {
          label: '15 M',
          data: [],
          fill: false,
          borderColor: 'rgba(103, 194, 58, 1)',
          tension: 0.1,
          pointRadius: 0,
        },
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      scales: {
        y: {
          beginAtZero: true,
          title: { display: true, text: 'Load' },
        },
      },
    },
  })

  await FetchRTStatus()
  if (destroyed) return
  startPolling()
})

watchChartTheme(() => [loadavg_chart])

onUnmounted(() => {
  destroyed = true
  stopPolling()
  loadavg_chart?.destroy()
})

const loadSystemInfo = async () => {
  const resp = await getSystemInfo()
  if (destroyed) return
  if (resp.code === 0) {
    sysinfo.value = resp.data
    loadValues.value[0] = fmtLoad(resp.data.loadavg_one)
    loadValues.value[1] = fmtLoad(resp.data.loadavg_five)
    loadValues.value[2] = fmtLoad(resp.data.loadavg_fifteen)
  }
}

const FetchRTStatus = async (rangeValue?: string, reqEpoch?: number) => {
  const resp = await getRTStatus(
    (rangeValue ?? range.value) === 'live' ? undefined : { range: rangeValue ?? range.value },
  )
  if (reqEpoch !== undefined && reqEpoch !== epoch.value) return // 范围已切换，丢弃过期响应
  if (destroyed || resp.code !== 0) return
  {
    const data = resp.data
    sysinfo.value = { ...sysinfo.value, ...data }
    loadValues.value[0] = fmtLoad(data.loadavg_one)
    loadValues.value[1] = fmtLoad(data.loadavg_five)
    loadValues.value[2] = fmtLoad(data.loadavg_fifteen)

    const labels: string[] = []
    const one: any[] = []
    const five: any[] = []
    const fifteen: any[] = []
    data.system_stats.forEach((el: Record<string, any>) => {
      one.push(el.loadavg_one)
      five.push(el.loadavg_five)
      fifteen.push(el.loadavg_fifteen)
    labels.push(monitorTimeLabel(el.created_at, range.value))
    })
    loadavg_chart.data.labels = labels
    loadavg_chart.data.datasets[0].data = one
    loadavg_chart.data.datasets[1].data = five
    loadavg_chart.data.datasets[2].data = fifteen
    loadavg_chart.update('none')
  }
}
</script>

<style scoped>
.stat-card {
  text-align: center;
  padding: 8px 0;
}
.stat-title {
  font-size: 14px;
  color: var(--el-text-color-secondary);
  margin-bottom: 8px;
}
.stat-value {
  font-size: 32px;
  font-weight: bold;
}
.stat-sub {
  margin-top: 8px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.mt-4 {
  margin-top: 16px;
}
</style>
