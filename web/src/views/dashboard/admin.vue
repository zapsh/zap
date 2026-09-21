<template>
  <div class="admin-dashboard">
    <!-- 快捷入口 -->
    <el-row :gutter="16" class="shortcut-row">
      <el-col
        v-for="item in shortcuts"
        :key="item.path"
        :xs="12"
        :sm="8"
        :md="6"
        :lg="3"
        class="shortcut-col"
      >
        <el-card shadow="hover" class="shortcut-card" @click="go(item.path)">
          <div class="shortcut-body">
            <el-icon :size="28" class="shortcut-icon" :style="{ color: item.color }">
              <Icon :icon="item.icon" />
            </el-icon>
            <span class="shortcut-title">{{ item.title }}</span>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 服务器状态 -->
    <div class="section-title-row">
      <span class="section-title">{{ t('dashboardAdmin.serverStatus') }}</span>
      <el-link type="primary" underline="never" @click="go('/server-status/index?tab=monitor')">
        {{ t('dashboardAdmin.viewDetails') }}
      </el-link>
    </div>
    <el-row :gutter="16" class="section-row">
      <el-col v-for="item in statusCards" :key="item.id" :xs="12" :sm="6" :lg="6" class="metric-col">
        <el-card shadow="hover" class="metric-card">
          <div class="metric-body">
            <div class="metric-left">
              <div class="metric-header">
                <el-icon :size="22" class="metric-icon" :style="{ color: item.color }">
                  <Icon :icon="item.icon" />
                </el-icon>
                <span class="metric-title">{{ item.title }}</span>
              </div>
              <div class="metric-desc">
                <div v-for="(row, idx) in item.desc" :key="idx" class="metric-desc-row">
                  <span class="metric-desc-label">{{ row.label }}</span>
                  <span class="metric-desc-value">{{ row.value }}</span>
                </div>
              </div>
            </div>
            <div class="metric-right">
              <el-progress
                type="dashboard"
                :percentage="item.value"
                :color="item.color"
                :stroke-width="8"
                :width="82"
              />
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 服务器信息 / About Zap -->
    <el-row :gutter="16" class="section-row">
      <el-col :xs="24" :md="12" :lg="12">
        <el-card shadow="hover" class="info-card" v-loading="loading">
          <template #header>
            <div class="card-header">
              <span>{{ t('dashboardAdmin.serverInfo') }}</span>
              <el-link type="primary" underline="never" @click="go('/server-status/index')">
                {{ t('dashboardAdmin.viewDetails') }}
              </el-link>
            </div>
          </template>
          <el-descriptions :column="1" size="small" border>
            <el-descriptions-item :label="t('dashboardAdmin.hostname')">
              {{ overview.host_name || sysinfo.host_name || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.os')">
              {{ overview.os_version || sysinfo.os_name_version || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.arch')">
              {{ overview.arch || sysinfo.arch || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.cpu')">
              {{ cpuModel }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.systemLoad')">
              {{ loadAvgText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.kernel')">
              {{ kernelText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.publicIp')">
              {{ sysinfo.public_ip || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.uptime')">
              {{ overview.uptime || sysinfo.uptime || '-' }}
            </el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

<el-col :xs="24" :md="12" :lg="12">
        <el-card shadow="hover" class="info-card" v-loading="loading">
          <template #header>
            <div class="card-header">
              <span>{{ t('dashboardAdmin.aboutZap') }}</span>
            </div>
          </template>
          <el-descriptions :column="1" size="small" border>
            <el-descriptions-item :label="t('dashboardAdmin.version')">
              {{ about.version || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.buildDate')">
              {{ buildDateText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.commit')">
              {{ commitText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.branch')">
              {{ about.git_branch || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.rust')">
              {{ rustText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.target')">
              {{ targetText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.license')">
              {{ about.license || '-' }}
            </el-descriptions-item>
          </el-descriptions>
          <div class="about-actions">
            <el-link type="primary" underline="never" @click="go('/system/about')">
              {{ t('dashboardAdmin.docs') }}
            </el-link>
            <el-link type="primary" underline="never" @click="go('/dev/api-docs')">
              {{ t('dashboardAdmin.apiDocs') }}
            </el-link>
            <el-link type="primary" underline="never" target="_blank" href="https://github.com/zapsh/zap">
              Github
            </el-link>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 磁盘使用 / 网络接口 -->
    <el-row :gutter="16" class="section-row">
      <el-col :span="24">
        <el-card shadow="hover" v-loading="loading">
          <el-tabs v-model="resourceTab" class="resource-tabs">
            <el-tab-pane :label="t('dashboardAdmin.diskUsage')" name="disk">
              <el-table :data="diskUsage" size="small" :empty-text="t('dashboardAdmin.unknown')">
                <el-table-column prop="mount_point" label="Mount" min-width="100" />
                <el-table-column label="Usage" min-width="160">
                  <template #default="{ row }">
                    <el-progress :percentage="row.usage_pct" :color="diskColor(row.usage_pct)" />
                  </template>
                </el-table-column>
                <el-table-column prop="used" :label="t('dashboardAdmin.used')" min-width="90">
                  <template #default="{ row }">{{ formatBytes(row.used) }}</template>
                </el-table-column>
                <el-table-column prop="available" :label="t('dashboardAdmin.available')" min-width="90">
                  <template #default="{ row }">{{ formatBytes(row.available) }}</template>
                </el-table-column>
              </el-table>
            </el-tab-pane>
            <el-tab-pane :label="t('dashboardAdmin.network')" name="network">
              <el-table :data="networks" size="small" :empty-text="t('dashboardAdmin.unknown')">
                <el-table-column prop="interface_name" label="Interface" min-width="110" />
                <el-table-column prop="ipaddrs" label="IP" min-width="160">
                  <template #default="{ row }">{{ row.ipaddrs.join(', ') || '-' }}</template>
                </el-table-column>
                <el-table-column prop="down" :label="`↓ ${t('dashboardAdmin.total')}`" min-width="110">
                  <template #default="{ row }">{{ formatBytes(row.down) }}</template>
                </el-table-column>
                <el-table-column prop="up" :label="`↑ ${t('dashboardAdmin.total')}`" min-width="110">
                  <template #default="{ row }">{{ formatBytes(row.up) }}</template>
                </el-table-column>
              </el-table>
            </el-tab-pane>
          </el-tabs>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { Icon } from '@/icons'
import { formatBytes } from '@/utils/fmt'
import { isArray } from '@/utils/validate'
import { getSystemAbout, getSystemInfo, getSystemOverview } from '@/api/dashboard.ts'

const { t } = useI18n()
const router = useRouter()

const loading = ref(true)
const sysinfo: Record<string, any> = ref({})
const overview: Record<string, any> = ref({})
const about: Record<string, any> = ref({})
const resourceTab = ref('disk')

const shortcuts = computed(() => [
  { title: t('dashboardAdmin.sites'), path: '/site/index', icon: 'material-symbols:public', color: '#409eff' },
  { title: t('dashboardAdmin.databases'), path: '/database/index', icon: 'material-symbols:database', color: '#67c23a' },
  { title: t('dashboardAdmin.files'), path: '/files/index', icon: 'material-symbols:folder', color: '#e6a23c' },
  // 用户管理与角色管理已合并为一页（/system/access 页内 nav pill 切换）：
  // 旧路径 /system/users 已随菜单合并下架，点进去没有任何路由匹配，页面打不开。
  {
    title: t('dashboardAdmin.users'),
    path: '/system/access?tab=users',
    icon: 'material-symbols:person',
    color: '#9254de',
  },
  { title: t('dashboardAdmin.settings'), path: '/system/zap-config', icon: 'material-symbols:settings', color: '#606266' },
  { title: t('dashboardAdmin.serverStatusPage'), path: '/server-status/index', icon: 'material-symbols:monitoring', color: '#f56c6c' },
  { title: t('dashboardAdmin.terminal'), path: '/terminal/index', icon: 'material-symbols:monitor', color: '#13c2c2' },
  { title: 'SSL/TLS', path: '/ssl-tls/certs', icon: 'material-symbols:lock', color: '#eb2f96' },
])

/** 构建信息缺失时 vergen 会返回 unknown，统一不展示内部占位串 */
function meta(value: any) {
  return !value || value === 'unknown' ? '' : String(value)
}

const buildDateText = computed(() => {
  const date = meta(about.value.build_date)
  if (!date) return '-'
  const ts = meta(about.value.build_timestamp)
  const time = ts.length >= 19 ? ts.slice(11, 19) : ''
  return time ? `${date} ${time}` : date
})

const commitText = computed(() => {
  const sha = meta(about.value.git_sha)
  if (!sha) return '-'
  const short = sha.slice(0, 7)
  return about.value.git_dirty === 'true' ? `${short} (dirty)` : short
})

const rustText = computed(() => {
  const version = meta(about.value.rustc_version)
  if (!version) return '-'
  const channel = meta(about.value.rustc_channel)
  return channel ? `${version} (${channel})` : version
})

const targetText = computed(() => {
  const triple = meta(about.value.target_triple)
  if (!triple) return '-'
  const profile = meta(about.value.profile)
  return profile ? `${triple} / ${profile}` : triple
})

const cpuModel = computed(() => {
  const cpu = overview.value.cpu || {}
  const model = cpu.model || sysinfo.value.product_name || '-'
  const cores = cpu.logical_cores || sysinfo.value.cpu_num || 0
  return cores ? `${model} (${cores} ${t('dashboardAdmin.cores')})` : model
})

const loadAvgPct = computed(() => {
  const one = Number(sysinfo.value.loadavg_one ?? overview.value.cpu?.loadavg_one ?? 0)
  const cores = Number(sysinfo.value.cpu_num ?? overview.value.cpu?.logical_cores ?? 1)
  return cores ? parseFloat(((one / cores) * 100).toFixed(2)) : 0
})

const loadAvgText = computed(() => {
  const cpu = overview.value.cpu || {}
  const one = Number(sysinfo.value.loadavg_one ?? cpu.loadavg_one ?? 0)
  const five = Number(sysinfo.value.loadavg_five ?? cpu.loadavg_five ?? 0)
  const fifteen = Number(sysinfo.value.loadavg_fifteen ?? cpu.loadavg_fifteen ?? 0)
  if (!one && !five && !fifteen) return '-'
  return `${one.toFixed(2)} / ${five.toFixed(2)} / ${fifteen.toFixed(2)}`
})

const kernelText = computed(() => overview.value.kernel_version || sysinfo.value.kernel_version || '-')

const cpuUsage = computed(() => parseFloat((sysinfo.value.cpu_usage ?? overview.value.cpu?.usage ?? 0).toFixed(2)))

const memoryPct = computed(() => {
  const total = Number(sysinfo.value.memory_total_b ?? overview.value.memory_usage?.total ?? 0)
  const available = Number(sysinfo.value.available_memory_b ?? overview.value.memory_usage?.available ?? 0)
  if (!total) return 0
  return parseFloat((((total - available) / total) * 100).toFixed(2))
})

const diskRootPct = computed(() => {
  const root = diskUsage.value.find((d: any) => d.mount_point === '/')
  return root ? root.usage_pct : 0
})

const usedMemoryText = computed(() => {
  const total = Number(sysinfo.value.memory_total_b ?? overview.value.memory_usage?.total ?? 0)
  const available = Number(sysinfo.value.available_memory_b ?? overview.value.memory_usage?.available ?? 0)
  return total ? `${formatBytes(total - available)} / ${formatBytes(total)}` : '-'
})

const diskRootText = computed(() => {
  const root = diskUsage.value.find((d: any) => d.mount_point === '/')
  return root ? `${formatBytes(root.used)} / ${formatBytes(root.total)}` : '-'
})

const rootAvailable = computed(() => {
  const root = diskUsage.value.find((d: any) => d.mount_point === '/')
  return root ? root.available : 0
})

const statusCards = computed(() => {
  const cores = Number(sysinfo.value.cpu_num ?? overview.value.cpu?.logical_cores ?? 0)
  const load1m = Number(sysinfo.value.loadavg_one ?? overview.value.cpu?.loadavg_one ?? 0).toFixed(2)
  return [
    {
      id: 'load',
      icon: 'material-symbols:monitoring',
      title: t('dashboardAdmin.systemLoad'),
      value: loadAvgPct.value,
      color: '#e6a23c',
      desc: [
        { label: t('dashboardAdmin.cores'), value: cores ? String(cores) : '-' },
        { label: t('systemMonitor.load1m'), value: load1m },
      ],
    },
    {
      id: 'cpu',
      icon: 'material-symbols:memory',
      title: 'CPU',
      value: cpuUsage.value,
      color: '#409eff',
      desc: [
        { label: t('dashboardAdmin.cores'), value: cores ? String(cores) : '-' },
        { label: t('dashboardAdmin.usage'), value: `${cpuUsage.value}%` },
      ],
    },
    {
      id: 'memory',
      icon: 'material-symbols:memory',
      title: t('dashboardAdmin.memory'),
      value: memoryPct.value,
      color: '#67c23a',
      desc: [
        { label: t('dashboardAdmin.used'), value: usedMemoryText.value },
        { label: t('dashboardAdmin.available'), value: formatBytes(sysinfo.value.available_memory_b ?? overview.value.memory_usage?.available ?? 0) },
      ],
    },
    {
      id: 'disk',
      icon: 'material-symbols:storage',
      title: t('dashboardAdmin.diskRoot'),
      value: diskRootPct.value,
      color: '#f56c6c',
      desc: [
        { label: t('dashboardAdmin.used'), value: diskRootText.value },
        { label: t('dashboardAdmin.available'), value: formatBytes(rootAvailable.value) },
      ],
    },
  ]
})

const diskUsage = computed(() => {
  const list = overview.value.disk_usage || sysinfo.value.disk_info || []
  return isArray(list) ? list : []
})

const networks = computed(() => {
  const list = overview.value.networks || []
  return isArray(list) ? list : []
})

function go(path: string) {
  router.push(path)
}

function diskColor(pct: number) {
  if (pct >= 90) return '#f56c6c'
  if (pct >= 70) return '#e6a23c'
  return '#67c23a'
}

async function loadStaticData() {
  try {
    const [infoResp, overviewResp, aboutResp] = await Promise.all([
      getSystemInfo(),
      getSystemOverview(),
      getSystemAbout(),
    ])
    if (infoResp.code === 0) sysinfo.value = infoResp.data || {}
    if (overviewResp.code === 0) overview.value = overviewResp.data || {}
    if (aboutResp.code === 0) about.value = aboutResp.data || {}
  } finally {
    loading.value = false
  }
}

onMounted(async () => {
  await loadStaticData()
})
</script>

<style scoped>
.admin-dashboard {
  padding-bottom: 24px;
}

.shortcut-row {
  margin-bottom: 16px;
}

.shortcut-col {
  margin-bottom: 16px;
}

.shortcut-card {
  cursor: pointer;
  transition: transform 0.15s ease, box-shadow 0.15s ease;
}

.shortcut-card:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
}

.shortcut-body {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 4px 0;
}

.shortcut-icon {
  display: flex;
  align-items: center;
  justify-content: center;
}

.shortcut-title {
  font-size: 14px;
  font-weight: 500;
  color: var(--el-text-color-primary);
}

.section-row {
  margin-bottom: 16px;
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-weight: 600;
}

.refresh-hint {
  font-size: 12px;
  font-weight: normal;
  color: var(--el-text-color-secondary);
}

.info-card :deep(.el-card__body) {
  padding: 12px;
}

.about-actions {
  display: flex;
  gap: 16px;
  margin-top: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--el-border-color-lighter);
}

.resource-tabs :deep(.el-tabs__header) {
  margin: 0 0 10px;
}

.section-title-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin: 0 2px 12px;
}

.section-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}

.metric-col {
  margin-bottom: 16px;
}

.metric-card {
  border-radius: 10px;
  transition: box-shadow 0.2s ease;
}

.metric-card:hover {
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.08);
}

.metric-card :deep(.el-card__body) {
  padding: 14px 16px;
}

.metric-body {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.metric-left {
  flex: 1;
  min-width: 0;
}

.metric-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

.metric-icon {
  display: flex;
  align-items: center;
  justify-content: center;
}

.metric-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}

.metric-desc {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.metric-desc-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
  line-height: 1.6;
}

.metric-desc-label {
  color: var(--el-text-color-secondary);
}

.metric-desc-value {
  color: var(--el-text-color-primary);
  font-variant-numeric: tabular-nums;
}

.metric-right {
  flex-shrink: 0;
}
</style>
