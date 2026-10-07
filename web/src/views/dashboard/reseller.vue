<template>
  <div class="cpanel-home">
    <!-- 欢迎横幅 -->
    <el-card shadow="never" class="welcome-card">
      <div class="welcome">
        <el-avatar :size="56" :src="account.avatar || undefined" class="welcome-avatar">
          {{ (account.nickname || account.username || '?').slice(0, 1).toUpperCase() }}
        </el-avatar>
        <div class="welcome-main">
          <div class="welcome-greet">
            {{ greeting }}，{{ account.nickname || account.username || t('dashboardCpanel.guest') }}
          </div>
          <div class="welcome-sub">
            <el-tag size="small" type="warning" effect="light">{{ t('dashboardReseller.roleTag') }}</el-tag>
            <el-tag v-if="pkg.name" size="small" type="primary" effect="light">{{ pkg.name }}</el-tag>
            <span class="muted">{{ todayText }}</span>
          </div>
        </div>
        <el-button type="primary" :icon="Plus" @click="goCustomers">
          {{ t('dashboardReseller.manageCustomers') }}
        </el-button>
      </div>
    </el-card>

    <!-- 搜索 -->
    <div class="search-bar">
      <el-input
        v-model="keyword"
        :placeholder="t('dashboardCpanel.searchPlaceholder')"
        clearable
        class="search-input"
      >
        <template #prefix>
          <el-icon><Icon icon="material-symbols:search" /></el-icon>
        </template>
      </el-input>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="16" class="stat-row">
      <el-col v-for="c in statCards" :key="c.key" :xs="12" :sm="8" :lg="c.span">
        <el-card shadow="hover" class="stat-card">
          <div class="stat-body">
            <el-icon class="stat-icon" :style="{ color: c.color, background: c.bg }">
              <Icon :icon="c.icon" />
            </el-icon>
            <div class="stat-meta">
              <div class="stat-value">{{ c.value }}</div>
              <div class="stat-title">{{ c.title }}</div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 常规信息 + 使用情况 -->
    <el-row :gutter="16" class="info-row">
      <!-- 常规信息 -->
      <el-col :xs="24" :sm="12">
        <el-card shadow="hover" class="info-card">
          <template #header>
            <div class="card-header">
              <span>{{ t('dashboardCpanel.generalInfo') }}</span>
            </div>
          </template>
          <el-descriptions :column="1" label-width="96px">
            <el-descriptions-item :label="t('dashboardCpanel.currentUser')">
              {{ account.nickname || account.username || '—' }}
              <span v-if="account.username" class="muted">{{
                t('dashboardCpanel.usernameParen', { name: account.username })
              }}</span>
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.loginEmail')">{{
              account.email || '—'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.homeDir')">{{
              account.home_dir || '—'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.lastLoginIp')">{{
              account.last_login_ip || '—'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.lastLoginTime')">{{
              fmtTime(account.last_login_time)
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.sharedIp')">{{
              server.public_ip || '—'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.server')">{{
              server.host_name || '—'
            }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <!-- 使用情况 -->
      <el-col :xs="24" :sm="12">
        <el-card shadow="hover" class="info-card usage-card">
          <template #header>
            <div class="card-header">
              <span>{{ t('dashboardCpanel.usage') }}</span>
              <el-tag v-if="pkg.name && !packageBound" size="small" type="info" effect="plain">
                {{ t('dashboardCpanel.packageUnboundGlobal') }}
              </el-tag>
            </div>
          </template>

          <!-- 用量仪表 -->
          <div class="gauges">
            <div class="gauge">
              <el-progress
                type="dashboard"
                :width="104"
                :percentage="diskPct"
                :color="diskColor"
                :stroke-width="9"
              >
                <template #default>
                  <span class="gauge-pct">{{
                    pkg.disk_quota_mb > 0 ? diskPct + '%' : t('dashboardCpanel.noLimit')
                  }}</span>
                  <span class="gauge-cap">{{ t('dashboardCpanel.diskQuota') }}</span>
                </template>
              </el-progress>
              <div class="gauge-text">
                {{ account.disk_used_bytes ? formatBytes(account.disk_used_bytes) : '0 B' }}
                <span class="muted"> / {{ fmtMb(pkg.disk_quota_mb) }}</span>
              </div>
            </div>
            <div class="gauge">
              <el-progress
                type="dashboard"
                :width="104"
                :percentage="bwPct"
                :color="bwColor"
                :stroke-width="9"
              >
                <template #default>
                  <span class="gauge-pct">{{
                    pkg.max_bandwidth_mb > 0 ? bwPct + '%' : t('dashboardCpanel.noLimit')
                  }}</span>
                  <span class="gauge-cap">{{ t('dashboardCpanel.bandwidth') }}</span>
                </template>
              </el-progress>
              <div class="gauge-text">
                {{ account.bandwidth_used_bytes ? formatBytes(account.bandwidth_used_bytes) : '0 B' }}
                <span class="muted"> / {{ fmtMb(pkg.max_bandwidth_mb) }}</span>
              </div>
            </div>
          </div>

          <!-- 其余指标 -->
          <el-descriptions :column="1" label-width="96px" class="usage-meta">
            <el-descriptions-item :label="t('dashboardCpanel.package')">
              {{ pkg.name || t('dashboardCpanel.packageUnbound') }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.siteCount')">
              {{ t('dashboardCpanel.countUnit', { n: counts.sites }) }}
              <span class="muted">{{
                t('dashboardCpanel.siteLimit', { n: fmtLimit(pkg.max_sites) })
              }}</span>
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.domainCount')">
              <span class="muted">{{
                t('dashboardCpanel.domainLimit', { n: fmtLimit(pkg.max_domains) })
              }}</span>
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.fpmSpec')">
              <el-tag v-if="!pkg.fpm_spec_ref" size="small" type="info" effect="plain">
                {{ t('dashboardCpanel.panelDefault') }}
              </el-tag>
              <span v-else>{{ pkg.fpm_spec_ref }}</span>
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.sshTerminal')">
              {{
                packageBound
                  ? pkg.allow_ssh
                    ? t('dashboardCpanel.allow')
                    : t('dashboardCpanel.deny')
                  : t('dashboardCpanel.allowUnbound')
              }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.reverseProxy')">{{
              pkg.allow_proxy ? t('dashboardCpanel.allow') : t('dashboardCpanel.deny')
            }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>
    </el-row>

    <!-- 客户概览 -->
    <el-card shadow="hover" class="customer-card">
      <template #header>
        <div class="card-header">
          <span>{{ t('dashboardReseller.customerOverview') }}</span>
          <el-button link type="primary" :icon="ArrowRight" @click="goCustomers">
            {{ t('dashboardCpanel.viewAll') }}
          </el-button>
        </div>
      </template>
      <el-table :data="customers" size="default" :empty-text="t('dashboardReseller.customerEmpty')">
        <el-table-column :label="t('dashboardReseller.colUser')" min-width="160">
          <template #default="{ row }">
            <div class="cell-user">
              <span class="cell-name">{{ row.nickname || row.username }}</span>
              <span class="muted">@{{ row.username }}</span>
            </div>
          </template>
        </el-table-column>
        <el-table-column :label="t('dashboardReseller.colPackage')" min-width="120">
          <template #default="{ row }">
            <el-tag v-if="row.package_name" size="small" effect="plain">{{
              row.package_name
            }}</el-tag>
            <span v-else class="muted">{{ t('dashboardReseller.noPackage') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('dashboardReseller.colStatus')" width="100">
          <template #default="{ row }">
            <el-tag :type="row.status === 0 ? 'danger' : 'success'" size="small" effect="light">
              {{ row.status === 0 ? t('dashboardReseller.statusDisabled') : t('dashboardReseller.statusEnabled') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('dashboardReseller.colDisk')" min-width="140">
          <template #default="{ row }">
            {{ row.disk_used_bytes ? formatBytes(row.disk_used_bytes) : '0 B' }}
          </template>
        </el-table-column>
        <el-table-column :label="t('dashboardReseller.colLastLogin')" min-width="160">
          <template #default="{ row }">
            <span class="muted">{{ fmtTime(row.last_login_time) }}</span>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 功能分组 -->
    <div v-for="group in visibleGroups" :key="group.title" class="group">
      <div class="group-title">{{ group.title }}</div>
      <el-row :gutter="16">
        <el-col :xs="12" :sm="8" :md="6" :lg="4" v-for="item in group.items" :key="item.title">
          <div class="app-tile" @click="handleClick(item)">
            <el-icon class="app-icon"><Icon :icon="item.icon" /></el-icon>
            <div class="app-title">{{ item.title }}</div>
          </div>
        </el-col>
      </el-row>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { Icon, Plus, ArrowRight } from '@/icons'
import { formatBytes } from '@/utils/fmt'
import { useUserStore } from '@/stores/user'
import { getSystemInfo, getDashboardCounts } from '@/api/dashboard'
import type { DashboardCounts } from '@/api/dashboard'
import { getUserInfo, getUserList } from '@/api/user'
import type { UserListItem } from '@/api/user'

interface AppEntry {
  title: string
  icon: string
  path?: string
  roles: string[]
  coming?: boolean
}

interface AppGroup {
  title: string
  items: AppEntry[]
}

const { t } = useI18n()
const router = useRouter()
const userStore = useUserStore()
const roles = userStore.roles

const keyword = ref('')

// 功能入口
const groups = computed<AppGroup[]>(() => [
  {
    title: t('dashboardCpanel.groupCommon'),
    items: [
      {
        title: t('dashboardCpanel.itemFiles'),
        icon: 'material-symbols:folder',
        path: '/files',
        roles: ['reseller'],
      },
      {
        title: t('dashboardCpanel.itemSite'),
        icon: 'material-symbols:public',
        path: '/site',
        roles: ['reseller'],
      },
      {
        title: t('dashboardCpanel.itemTerminal'),
        icon: 'material-symbols:monitor',
        path: '/terminal',
        roles: ['reseller'],
      },
      {
        title: t('dashboardCpanel.itemProfile'),
        icon: 'material-symbols:person',
        path: '/profile',
        roles: ['reseller'],
      },
    ],
  },
  {
    title: t('dashboardReseller.groupReseller'),
    items: [
      {
        title: t('dashboardReseller.itemCustomers'),
        icon: 'material-symbols:account-circle',
        path: '/system/access',
        roles: ['reseller'],
      },
      {
        title: t('dashboardReseller.itemQuota'),
        icon: 'material-symbols:speed',
        roles: ['reseller'],
        coming: true,
      },
      {
        title: t('dashboardReseller.itemAllocation'),
        icon: 'material-symbols:tune',
        roles: ['reseller'],
        coming: true,
      },
    ],
  },
])

const hasRole = (entryRoles: string[]) => entryRoles.some((r) => roles.includes(r))

const visibleGroups = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return groups.value
    .map((g) => ({
      ...g,
      items: g.items.filter(
        (it) => hasRole(it.roles) && (!kw || it.title.toLowerCase().includes(kw)),
      ),
    }))
    .filter((g) => g.items.length > 0)
})

function handleClick(item: AppEntry) {
  if (item.coming) {
    ElMessage.info(t('dashboardCpanel.comingSoon'))
    return
  }
  if (item.path) {
    router.push(item.path)
  }
}

// ── 统计卡片（经销商：名下客户 / 站点总数 / 数据库总数） ──
const counts = ref<DashboardCounts>({ users: 0, sites: 0, databases: 0 })

const statCards = computed(() => [
  {
    key: 'customers',
    title: t('dashboardReseller.statCustomers'),
    value: counts.value.users,
    icon: 'material-symbols:group',
    color: '#9254de',
    bg: '#f9f0ff',
    span: 8,
  },
  {
    key: 'sites',
    title: t('dashboardCpanel.statSites'),
    value: counts.value.sites,
    icon: 'material-symbols:public',
    color: '#409eff',
    bg: '#ecf5ff',
    span: 8,
  },
  {
    key: 'databases',
    title: t('dashboardCpanel.statDatabases'),
    value: counts.value.databases,
    icon: 'material-symbols:database',
    color: '#67c23a',
    bg: '#f0f9eb',
    span: 8,
  },
])

// ── 客户概览 ─────────────────────────────────────────────
const customers = ref<UserListItem[]>([])

function goCustomers() {
  router.push('/system/access')
}

async function loadCustomers() {
  try {
    const res = await getUserList({})
    const list = (res?.data as UserListItem[]) || []
    // 取最近创建的 6 个客户展示
    customers.value = [...list]
      .sort((a, b) => (b.created_at || 0) - (a.created_at || 0))
      .slice(0, 6)
  } catch {
    /* ignore */
  }
}

// ── 欢迎横幅 ─────────────────────────────────────────────
const greeting = computed(() => {
  const h = new Date().getHours()
  if (h < 6 || h >= 22) return t('dashboardCpanel.greetNight')
  if (h < 12) return t('dashboardCpanel.greetMorning')
  if (h < 14) return t('dashboardCpanel.greetNoon')
  if (h < 18) return t('dashboardCpanel.greetAfternoon')
  return t('dashboardCpanel.greetEvening')
})

const todayText = computed(() =>
  new Date().toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    weekday: 'long',
  }),
)

// ── 常规信息 + 使用情况 ─────────────────────────────────────
const account = ref<Record<string, any>>({})
const server = ref<Record<string, any>>({})
const packageBound = ref(false)
const pkg = ref<Record<string, any>>({})

const fmtTime = (ts: number) => (ts ? new Date(ts * 1000).toLocaleString() : '—')
/** 数值上限展示：<=0 表示不限 */
const fmtLimit = (v?: number) =>
  !Number(v) ? t('dashboardCpanel.noLimit') : t('dashboardCpanel.countUnit', { n: v })
/** MB 容量展示：>= 1024 换算成 GB */
const fmtMb = (v?: number) => {
  const mb = Number(v) || 0
  if (mb <= 0) return t('dashboardCpanel.noLimit')
  return mb >= 1024 ? `${(mb / 1024).toFixed(1).replace(/\.0$/, '')} GB` : `${mb} MB`
}

/** 磁盘用量百分比（配额 0 = 不限，按 0 处理） */
const diskPct = computed(() => {
  const quotaMb = Number(pkg.value.disk_quota_mb) || 0
  const used = Number(account.value.disk_used_bytes) || 0
  if (!quotaMb) return 0
  const pct = (used / (quotaMb * 1024 * 1024)) * 100
  return Math.max(0, Math.min(100, Math.round(pct)))
})
/** 本月流量百分比 */
const bwPct = computed(() => {
  const quotaMb = Number(pkg.value.max_bandwidth_mb) || 0
  const used = Number(account.value.bandwidth_used_bytes) || 0
  if (!quotaMb) return 0
  const pct = (used / (quotaMb * 1024 * 1024)) * 100
  return Math.max(0, Math.min(100, Math.round(pct)))
})
/** 用量着色：>=90 危险 / >=75 警告 / 其余正常 */
function usageColor(pct: number): string {
  if (pct >= 90) return '#f56c6c'
  if (pct >= 75) return '#e6a23c'
  return '#67c23a'
}
const diskColor = computed(() => usageColor(diskPct.value))
const bwColor = computed(() => usageColor(bwPct.value))

async function loadAccount() {
  account.value = { ...userStore.userInfo }
  try {
    const res = await getUserInfo()
    if (res?.data) {
      const d = res.data as any
      account.value = { ...account.value, ...d }
      packageBound.value = !!d.package_bound
      pkg.value = d.package || {}
    }
  } catch {
    /* 保留既有信息 */
  }
}

async function loadServer() {
  try {
    const res = await getSystemInfo()
    if (res?.data) server.value = res.data
  } catch {
    /* ignore */
  }
}

async function loadCounts() {
  try {
    const res = await getDashboardCounts()
    if (res?.data) counts.value = { ...counts.value, ...res.data }
  } catch {
    /* ignore */
  }
}

onMounted(async () => {
  await Promise.all([loadAccount(), loadServer(), loadCounts(), loadCustomers()])
})
</script>

<style scoped>
.cpanel-home {
  padding: 20px;
}

/* 欢迎横幅 */
.welcome-card {
  margin-bottom: 16px;
  border: none;
  background: linear-gradient(120deg, var(--el-color-warning-light-9), var(--el-bg-color));
}
.welcome {
  display: flex;
  align-items: center;
  gap: 16px;
}
.welcome-avatar {
  flex: none;
  background: var(--el-color-warning);
  color: #fff;
  font-weight: 600;
}
.welcome-main {
  flex: 1;
  min-width: 0;
}
.welcome-greet {
  font-size: 20px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.welcome-sub {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 6px;
  flex-wrap: wrap;
}
.muted {
  color: var(--el-text-color-secondary);
}

.search-bar {
  margin-bottom: 16px;
}

.search-input {
  max-width: 480px;
}

.stat-row {
  margin-bottom: 16px;
}

.stat-card :deep(.el-card__body) {
  padding: 16px;
}

.stat-body {
  display: flex;
  align-items: center;
  gap: 14px;
}

.stat-icon {
  font-size: 26px;
  padding: 10px;
  border-radius: 10px;
}

.stat-value {
  font-size: 26px;
  font-weight: 600;
  line-height: 1.2;
}

.stat-title {
  font-size: 13px;
  color: var(--el-text-color-secondary);
}

.info-row {
  margin-bottom: 8px;
}

.info-card {
  margin-bottom: 16px;
}

.info-card :deep(.el-card__header) {
  padding: 12px 16px;
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-weight: 600;
}

/* 用量仪表 */
.gauges {
  display: flex;
  justify-content: space-around;
  gap: 12px;
  flex-wrap: wrap;
  padding: 8px 0 4px;
}
.gauge {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}
.gauge :deep(.el-progress__text) {
  display: flex;
  flex-direction: column;
  align-items: center;
}
.gauge-pct {
  font-size: 20px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  line-height: 1.1;
}
.gauge-cap {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}
.gauge-text {
  font-size: 13px;
  font-weight: 500;
  color: var(--el-text-color-primary);
  text-align: center;
}
.usage-meta {
  margin-top: 6px;
  border-top: 1px solid var(--el-border-color-lighter);
  padding-top: 8px;
}

/* 客户概览 */
.customer-card {
  margin-bottom: 16px;
}
.cell-user {
  display: flex;
  flex-direction: column;
  line-height: 1.3;
}
.cell-name {
  font-weight: 500;
  color: var(--el-text-color-primary);
}

.group {
  margin-bottom: 24px;
}

.group-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  margin-bottom: 12px;
  border-left: 3px solid #e6a23c;
  padding-left: 10px;
}

.app-tile {
  background: var(--el-bg-color);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  padding: 20px 8px;
  text-align: center;
  cursor: pointer;
  transition: all 0.2s;
  margin-bottom: 16px;
}

.app-tile:hover {
  border-color: #e6a23c;
  box-shadow: 0 2px 12px rgba(230, 162, 60, 0.2);
  transform: translateY(-2px);
}

.app-icon {
  font-size: 32px;
  color: var(--el-color-warning);
  margin-bottom: 8px;
}

.app-title {
  font-size: 14px;
  color: var(--el-text-color-primary);
}
</style>
