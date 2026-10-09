<!-- SPDX-License-Identifier: AGPL-3.0-only -->
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

    <!-- 客户数上限提示 -->
    <div v-if="account.max_users > 0" class="cap-hint">
      <el-icon><Icon icon="material-symbols:group" /></el-icon>
      <span>{{ t('dashboardReseller.customerCap', { used: counts.users, max: account.max_users }) }}</span>
    </div>

    <!-- 常规信息 + 使用情况 -->
    <el-row :gutter="16" class="info-row">
      <!-- 常规信息 -->
      <el-col :xs="24" :sm="10">
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
            <el-descriptions-item :label="t('dashboardCpanel.os')">{{
              server.os_name_version || server.os_name || '—'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.webserver')">{{
              server.webserver || '—'
            }}</el-descriptions-item>
            <el-descriptions-item :label="t('dashboardCpanel.server')">{{
              server.host_name || '—'
            }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <!-- 使用情况 -->
      <el-col :xs="24" :sm="14">
        <el-card shadow="hover" class="info-card usage-card">
          <template #header>
            <div class="card-header">
              <span>{{ t('dashboardCpanel.usage') }}</span>
              <el-tag v-if="pkg.name && !packageBound" size="small" type="info" effect="plain">
                {{ t('dashboardCpanel.packageUnboundGlobal') }}
              </el-tag>
            </div>
          </template>

          <!-- 规格表：标签 / 数值 / 进度条 / 百分比 四列对齐，仅磁盘·流量带进度条 -->
          <div class="usage-list">
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.package') }}</span>
              <span class="u-value">{{ pkg.name || t('dashboardCpanel.packageUnbound') }}</span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.diskQuota') }}</span>
              <span class="u-value">
                {{ account.disk_used_bytes ? formatBytes(account.disk_used_bytes) : '0 B' }}
                <span class="muted"> / {{ fmtMb(pkg.disk_quota_mb) }}</span>
              </span>
              <div class="u-bar">
                <el-progress
                  :percentage="diskPct"
                  :color="diskColor"
                  :stroke-width="7"
                  :show-text="false"
                />
              </div>
              <span class="u-pct" :style="{ color: diskColor }">{{
                pkg.disk_quota_mb > 0 ? diskPct + '%' : t('dashboardCpanel.noLimit')
              }}</span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.bandwidth') }}</span>
              <span class="u-value">
                {{
                  account.bandwidth_used_bytes
                    ? formatBytes(account.bandwidth_used_bytes)
                    : '0 B'
                }}
                <span class="muted"> / {{ fmtMb(pkg.max_bandwidth_mb) }}</span>
              </span>
              <div class="u-bar">
                <el-progress
                  :percentage="bwPct"
                  :color="bwColor"
                  :stroke-width="7"
                  :show-text="false"
                />
              </div>
              <span class="u-pct" :style="{ color: bwColor }">{{
                pkg.max_bandwidth_mb > 0 ? bwPct + '%' : t('dashboardCpanel.noLimit')
              }}</span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.siteCount') }}</span>
              <span class="u-value">
                {{ t('dashboardCpanel.countUnit', { n: counts.sites }) }}
                <span class="muted">{{
                  t('dashboardCpanel.siteLimit', { n: fmtLimit(pkg.max_sites) })
                }}</span>
              </span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.domainCount') }}</span>
              <span class="u-value">
                <span class="muted">{{
                  t('dashboardCpanel.domainLimit', { n: fmtLimit(pkg.max_domains) })
                }}</span>
              </span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.sshTerminal') }}</span>
              <span class="u-value">{{
                packageBound
                  ? pkg.allow_ssh
                    ? t('dashboardCpanel.allow')
                    : t('dashboardCpanel.deny')
                  : t('dashboardCpanel.allowUnbound')
              }}</span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.reverseProxy') }}</span>
              <span class="u-value">{{
                pkg.allow_proxy ? t('dashboardCpanel.allow') : t('dashboardCpanel.deny')
              }}</span>
            </div>
          </div>
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
            <el-tag :type="statusMeta(row.status).type" size="small" effect="light">
              {{ statusMeta(row.status).label }}
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
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { Icon, Plus, ArrowRight } from '@/icons'
import { formatBytes } from '@/utils/fmt'
import { useUserStore } from '@/stores/user'
import { useUserStatus } from '@/utils/userStatus'
import { getSystemInfo, getDashboardCounts } from '@/api/dashboard'
import type { DashboardCounts } from '@/api/dashboard'
import { getUserInfo, getUserList } from '@/api/user'
import type { UserListItem } from '@/api/user'

const { t } = useI18n()
const { meta: statusMeta } = useUserStatus()
const router = useRouter()
const userStore = useUserStore()

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
  // reseller 的客户管理路由（父路由重定向到 /reseller/users/index）
  router.push('/reseller/users')
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

.stat-row {
  margin-bottom: 16px;
}

.cap-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: -6px 0 16px;
  padding: 8px 12px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  background: var(--el-color-warning-light-9);
  border: 1px solid var(--el-color-warning-light-5);
  border-radius: 8px;
}
.cap-hint .el-icon {
  font-size: 16px;
  color: var(--el-color-warning);
}

.stat-card {
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.06) !important;
}
.stat-card :deep(.el-card__body) {
  padding: 12px 16px;
}

.stat-body {
  display: flex;
  align-items: center;
  gap: 12px;
}

.stat-icon {
  font-size: 22px;
  padding: 8px;
  border-radius: 8px;
}

.stat-value {
  font-size: 22px;
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

/* 使用情况：规格表网格（标签/数值/进度条/百分比 四列对齐） */
.usage-list {
  display: flex;
  flex-direction: column;
}
.usage-item {
  display: grid;
  grid-template-columns: 84px minmax(0, 1fr) 180px 40px;
  align-items: center;
  column-gap: 14px;
  padding: 11px 2px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.usage-item:last-child {
  border-bottom: none;
}
.u-label {
  font-size: 13px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
}
.u-value {
  font-size: 13px;
  color: var(--el-text-color-primary);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.u-bar {
  min-width: 0;
}
/* 进度条本体：细圆角轨道 + 圆角端点 + 轻微内阴影，更精致 */
.u-bar :deep(.el-progress-bar__outer) {
  background-color: var(--el-fill-color-light);
  border-radius: 999px;
  overflow: hidden;
}
.u-bar :deep(.el-progress-bar__inner) {
  border-radius: 999px;
  box-shadow: 0 0 4px rgba(0, 0, 0, 0.08) inset;
}
.u-pct {
  font-size: 12px;
  font-weight: 600;
  text-align: right;
  font-variant-numeric: tabular-nums;
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
</style>
