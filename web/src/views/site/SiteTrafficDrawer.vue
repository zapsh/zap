<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-drawer
    v-model="visible"
    :title="`${t('site.traffic')} — ${siteName}`"
    size="60%"
    :close-on-click-modal="false"
    destroy-on-close
    class="site-drawer"
  >
    <div class="traffic-toolbar">
      <el-radio-group v-model="days" @change="load">
        <el-radio-button :value="7">{{ t('site.trafficDays7') }}</el-radio-button>
        <el-radio-button :value="30">{{ t('site.trafficDays30') }}</el-radio-button>
        <el-radio-button :value="90">{{ t('site.trafficDays90') }}</el-radio-button>
      </el-radio-group>
      <el-button :icon="Refresh" @click="load">{{ t('common.refresh') }}</el-button>
    </div>

    <el-descriptions :column="3" border class="traffic-summary">
      <el-descriptions-item :label="t('site.trafficToday')">
        {{ formatBytes(todayBytes) }}
      </el-descriptions-item>
      <el-descriptions-item :label="t('site.trafficMonth')">
        {{ formatBytes(monthBytes) }}
      </el-descriptions-item>
      <el-descriptions-item :label="t('site.trafficTotal')">
        {{ formatBytes(totalBytes) }}
      </el-descriptions-item>
    </el-descriptions>

    <div class="chart-wrap">
      <canvas ref="canvasRef"></canvas>
    </div>

    <div class="section-title">{{ t('site.trafficTop', { n: days }) }}</div>
    <el-table :data="top" size="small" max-height="260">
      <el-table-column
        prop="path"
        :label="t('site.trafficPath')"
        min-width="240"
        show-overflow-tooltip
      />
      <el-table-column prop="hits" :label="t('site.trafficHits')" width="120" align="right" />
      <el-table-column :label="t('site.trafficBytes')" width="140" align="right">
        <template #default="{ row }">{{ formatBytes(row.bytes) }}</template>
      </el-table-column>
      <template #empty>
        <span class="muted">{{ t('site.trafficNoData') }}</span>
      </template>
    </el-table>
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import Chart from 'chart.js/auto'
import { Refresh } from '@/icons'
import { ElMessage } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { formatBytes } from '@/utils/fmt'
import { applyChartTheme, watchChartTheme } from '@/utils/chart-theme'
import type { SiteTrafficPoint, SiteTrafficTop } from '@/api/site'
import { getSiteTraffic } from '@/api/site'

const props = defineProps<{
  modelValue: boolean
  siteId: number
  siteName: string
}>()
const emit = defineEmits<{ (e: 'update:modelValue', v: boolean): void }>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const days = ref(30)
const daily = ref<SiteTrafficPoint[]>([])
const top = ref<SiteTrafficTop[]>([])
const todayBytes = ref(0)
const monthBytes = ref(0)
const totalBytes = ref(0)
const canvasRef = ref<HTMLCanvasElement | null>(null)
let chart: Chart | null = null

watch(
  () => props.modelValue,
  (v) => {
    if (v && props.siteId) load()
  },
)

async function load() {
  if (!props.siteId) return
  try {
    const res = await getSiteTraffic(props.siteId, days.value)
    const d = (res.data || {}) as any
    daily.value = d.daily || []
    top.value = d.top || []
    todayBytes.value = Number(d.today_bytes || 0)
    monthBytes.value = Number(d.month_bytes || 0)
    totalBytes.value = Number(d.total_bytes || 0)
    await nextTick()
    renderChart()
  } catch (e: any) {
    ElMessage.error(e.message || t('error.system'))
  }
}

function dayLabel(day: string) {
  return day.length === 8 ? `${day.slice(4, 6)}-${day.slice(6)}` : day
}

function renderChart() {
  if (!canvasRef.value) return
  chart?.destroy()
  applyChartTheme()
  chart = new Chart(canvasRef.value, {
    type: 'line',
    data: {
      labels: daily.value.map((d) => dayLabel(d.day)),
      datasets: [
        {
          label: t('site.trafficBytes'),
          data: daily.value.map((d) => Number((Number(d.bytes || 0) / 1024 / 1024).toFixed(2))),
          borderColor: '#409eff',
          backgroundColor: 'rgba(64, 158, 255, 0.15)',
          fill: true,
          tension: 0.3,
          pointRadius: 2,
        },
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { display: false } },
      scales: {
        y: {
          beginAtZero: true,
          title: { display: true, text: 'MB' },
        },
      },
    },
  })
}

watchChartTheme(() => (chart ? [chart] : []))

onUnmounted(() => {
  chart?.destroy()
  chart = null
})
</script>

<style scoped>
.traffic-toolbar {
  display: flex;
  gap: 10px;
  align-items: center;
  margin-bottom: 12px;
}
.traffic-summary {
  margin-bottom: 14px;
}
.chart-wrap {
  position: relative;
  height: 220px;
  margin-bottom: 16px;
}
.section-title {
  margin: 6px 0 8px;
  font-size: 14px;
  font-weight: 600;
}
.muted {
  color: var(--el-text-color-secondary);
}
</style>
