<template>
  <div>
    <el-card shadow="hover">
      <template #header>
        <div class="card-header">
          <span>{{ t('statusNetwork.cardList') }}</span>
          <el-button size="small" :icon="Refresh" circle @click="FetchRTStatus" />
        </div>
      </template>
      <el-table :data="netCards" v-loading="loading" border stripe>
        <el-table-column prop="name" :label="t('statusNetwork.iface')" width="140" />
        <el-table-column :label="t('statusNetwork.ipAddr')">
          <template #default="{ row }">{{ row.ipaddrs }}</template>
        </el-table-column>
        <el-table-column :label="t('statusNetwork.downRate')" width="140">
          <template #default="{ row }">{{ row.downRate || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('statusNetwork.upRate')" width="140">
          <template #default="{ row }">{{ row.upRate || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('statusNetwork.totalReceived')" width="140">
          <template #default="{ row }">{{ formatBytes(row.total_received) }}</template>
        </el-table-column>
        <el-table-column :label="t('statusNetwork.totalTransmitted')" width="140">
          <template #default="{ row }">{{ formatBytes(row.total_transmitted) }}</template>
        </el-table-column>
      </el-table>
    </el-card>

    <el-card v-loading="refreshing" shadow="hover" class="mt-4">
      <template #header>
        <div class="card-header">
          <span>{{ t('statusNetwork.trend') }}</span>
          <MonitorRangePicker v-model="range" />
          <el-select
            v-model="selectedNet"
            size="small"
            style="width: 180px"
            @change="rebuildNetworkChart"
          >
            <el-option v-for="n in netNames" :key="n" :label="n" :value="n" />
          </el-select>
        </div>
      </template>
      <canvas id="network_chart" style="width: 100%; height: 240px"></canvas>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import Chart from 'chart.js/auto'
import { applyChartTheme, watchChartTheme } from '@/utils/chart-theme'
import { onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import { getRTStatus } from '@/api/dashboard.ts'
import MonitorRangePicker from '@/components/MonitorRangePicker.vue'
import { monitorTimeLabel, useMonitorRange } from '@/composables/useMonitorRange'
import { formatBytes } from '@/utils/fmt'

const { t } = useI18n()

const loading = ref(false)
const netCards = ref<any[]>([])
const netNames = ref<string[]>([])
const selectedNet = ref('')
let history: Record<string, any[]> = {}

let network_chart: Chart
let timer: ReturnType<typeof setInterval> | undefined
let destroyed = false

// 时间范围：实时高频轮询；历史范围一次加载降采样数据（不自动轮询）
const { range, isLive, refreshing, epoch, run, startPolling, stopPolling } = useMonitorRange(
  (r, ep) => FetchRTStatus(r, ep),
  5,
)

onMounted(async () => {
  const container = document.getElementById('network_chart') as HTMLCanvasElement
  applyChartTheme()
  network_chart = new Chart(container, {
    type: 'line',
    data: {
      labels: [],
      datasets: [
        {
          label: t('statusNetwork.down'),
          data: [],
          fill: false,
          borderColor: 'rgba(103, 194, 58, 1)',
          tension: 0.1,
          pointRadius: 0,
        },
        {
          label: t('statusNetwork.up'),
          data: [],
          fill: false,
          borderColor: 'rgba(245, 108, 108, 1)',
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
          title: { display: true, text: 'KB' },
        },
      },
      plugins: {
        tooltip: {
          callbacks: {
            label: (context) => {
              const v = context.parsed.y ?? 0
              return `${context.dataset.label}: ${formatBytes(v * 1024)}`
            },
          },
        },
      },
    },
  })

  await FetchRTStatus()
  if (destroyed) return
  startPolling()
})

watchChartTheme(() => [network_chart])

onUnmounted(() => {
  destroyed = true
  stopPolling()
  network_chart?.destroy()
})

const rebuildNetworkChart = () => {
  const labels: string[] = []
  const down: any[] = []
  const up: any[] = []
  const samples = history[selectedNet.value] || []
  samples.forEach((el: Record<string, any>) => {
    down.push((el.received / 1024).toFixed(2))
    up.push((el.transmitted / 1024).toFixed(2))
    labels.push(monitorTimeLabel(el.created_at, range.value))
  })
  network_chart.data.labels = labels
  network_chart.data.datasets[0].data = down
  network_chart.data.datasets[1].data = up
  network_chart.update('none')
}

const FetchRTStatus = async (rangeValue?: string, reqEpoch?: number) => {
  if (destroyed) return
  loading.value = true
  try {
    const resp = await getRTStatus(
      (rangeValue ?? range.value) === 'live' ? undefined : { range: rangeValue ?? range.value },
    )
    if (reqEpoch !== undefined && reqEpoch !== epoch.value) return // 范围已切换，丢弃过期响应
    if (destroyed || resp.code !== 0) return
    const stats: Record<string, any>[] = resp.data.network_stats || []
    if (!stats.length) return

    history = {}
    // 按网卡分组历史样本
    stats.forEach((el) => {
      if (!history[el.name]) history[el.name] = []
      history[el.name].push(el)
    })

    netNames.value = Object.keys(history)
    if (!selectedNet.value || !history[selectedNet.value]) {
      selectedNet.value = netNames.value[0] || ''
    }

    // 网卡列表：每个网卡取最新样本，并计算速率
    netCards.value = netNames.value.map((name) => {
      const samples = history[name]
      const latest = samples[samples.length - 1]
      let downRate = ''
      let upRate = ''
      if (samples.length >= 2 && isLive.value) {
        const prev = samples[samples.length - 2]
        const dt = Math.max(latest.created_at - prev.created_at, 1)
        downRate = `${formatBytes(Math.max(latest.received - prev.received, 0) / dt)}/s`
        upRate = `${formatBytes(Math.max(latest.transmitted - prev.transmitted, 0) / dt)}/s`
      } else if (latest) {
        downRate = `${formatBytes(latest.received)}/s`
        upRate = `${formatBytes(latest.transmitted)}/s`
      }
      return { ...latest, downRate, upRate }
    })

    rebuildNetworkChart()
  } finally {
    loading.value = false
  }
}
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.mt-4 {
  margin-top: 16px;
}
</style>
