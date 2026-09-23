<template>
  <div>
    <el-row :gutter="20">
      <el-col :sm="8">
        <el-card shadow="hover">
          <div class="stat-card">
            <el-progress type="dashboard" :percentage="cpuPct" :color="cpuColor" />
            <div class="stat-title">{{ t('statusCpu.usage') }}</div>
            <div class="stat-sub">{{ cpuPct }}%</div>
          </div>
        </el-card>
      </el-col>
      <el-col :sm="16">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ t('statusCpu.infoTitle') }}</span>
            </div>
          </template>
          <el-descriptions :column="3" border>
            <el-descriptions-item :label="t('statusCpu.physicalCores')">{{
              sysinfo.physical_core_count ?? '-'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('statusCpu.logicalCores')">{{
              sysinfo.cpu_num ?? '-'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('statusCpu.arch')">{{
              sysinfo.arch || '-'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('statusCpu.productName')">{{
              sysinfo.product_name || '-'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('statusCpu.uptime')">{{
              sysinfo.uptime || '-'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('statusCpu.os')">{{
              sysinfo.os_name_version || '-'
            }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>
    </el-row>

    <el-card v-loading="refreshing" shadow="hover" class="mt-4">
      <template #header>
        <div class="card-header">
          <span>{{ t('statusCpu.trend') }}</span>
          <MonitorRangePicker v-model="range" />
        </div>
      </template>
      <canvas id="cpu_chart" style="width: 100%; height: 240px"></canvas>
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
const cpuPct = ref(0)
const cpuColor = computed(() => {
  return cpuPct.value > 90 ? '#f56c6c' : cpuPct.value > 70 ? '#e6a23c' : '#67c23a'
})

let cpu_chart: Chart
let timer: ReturnType<typeof setInterval> | undefined
let destroyed = false

// 时间范围：实时高频轮询；历史范围一次加载降采样数据（不自动轮询）
const { range, isLive, refreshing, epoch, run, startPolling, stopPolling } = useMonitorRange(
  (r, ep) => FetchRTStatus(r, ep),
  5,
)

onMounted(async () => {
  const container = document.getElementById('cpu_chart') as HTMLCanvasElement
  applyChartTheme()
  cpu_chart = new Chart(container, {
    type: 'line',
    data: {
      labels: [],
      datasets: [
        {
          label: t('statusCpu.usage'),
          data: [],
          fill: false,
          borderColor: 'rgba(245, 158, 11, 1)',
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
          max: 100,
          title: { display: true, text: '%' },
        },
      },
    },
  })

  await loadSystemInfo()
  if (destroyed) return
  await FetchRTStatus()
  if (destroyed) return
  startPolling()
})

watchChartTheme(() => [cpu_chart])

onUnmounted(() => {
  destroyed = true
  stopPolling()
  cpu_chart?.destroy()
})

const loadSystemInfo = async () => {
  const resp = await getSystemInfo()
  if (destroyed) return
  if (resp.code === 0) {
    sysinfo.value = resp.data
    if (resp.data.cpu_usage != null) cpuPct.value = parseFloat(resp.data.cpu_usage.toFixed(1))
  }
}

const FetchRTStatus = async (rangeValue?: string, reqEpoch?: number) => {
  const resp = await getRTStatus(
    (rangeValue ?? range.value) === 'live' ? undefined : { range: rangeValue ?? range.value },
  )
  if (reqEpoch !== undefined && reqEpoch !== epoch.value) return // 范围已切换，丢弃过期响应
  if (destroyed || resp.code !== 0) return
  const data = resp.data
  sysinfo.value = { ...sysinfo.value, ...data }
  if (data.cpu_usage != null) cpuPct.value = parseFloat(data.cpu_usage.toFixed(1))

  const labels: string[] = []
  const cpu: any[] = []
  data.system_stats.forEach((el: Record<string, any>) => {
    cpu.push(parseFloat(el.cpu_usage.toFixed(2)))
    labels.push(monitorTimeLabel(el.created_at, range.value))
  })
  cpu_chart.data.labels = labels
  cpu_chart.data.datasets[0].data = cpu
  cpu_chart.update('none')
}
</script>

<style scoped>
.stat-card {
  text-align: center;
  padding: 8px 0;
}
.stat-title {
  margin-top: 8px;
  font-size: 14px;
  color: var(--el-text-color-secondary);
}
.stat-sub {
  margin-top: 4px;
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
