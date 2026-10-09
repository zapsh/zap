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
            <el-tag v-if="pkg.name" size="small" type="primary" effect="light">{{ pkg.name }}</el-tag>
            <span class="muted">{{ todayText }}</span>
          </div>
        </div>
        <el-button type="primary" :icon="Plus" @click="goCreateSite">
          {{ t('dashboardCpanel.createSite') }}
        </el-button>
      </div>
    </el-card>

    <!-- 统计卡片（压缩高度 + 微阴影） -->
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

    <!-- 常规信息 + 使用情况（左右 4:6） -->
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
                {{ t('dashboardCpanel.countUnit', { n: stats.total }) }}
                <span class="muted">{{
                  t('dashboardCpanel.siteLimit', { n: fmtLimit(pkg.max_sites) })
                }}</span>
              </span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.domainCount') }}</span>
              <span class="u-value">
                {{ t('dashboardCpanel.countUnit', { n: stats.domains }) }}
                <span class="muted">{{
                  t('dashboardCpanel.domainLimit', { n: fmtLimit(pkg.max_domains) })
                }}</span>
              </span>
            </div>
            <div class="usage-item">
              <span class="u-label">{{ t('dashboardCpanel.fpmSpec') }}</span>
              <span class="u-value">
                <el-tag v-if="!pkg.fpm_spec_ref" size="small" type="info" effect="plain">
                  {{ t('dashboardCpanel.panelDefault') }}
                </el-tag>
                <template v-else>{{ pkg.fpm_spec_ref }}</template>
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

    <!-- 最近动态（固定高度，内部滚动） -->
    <el-card shadow="hover" class="activity-card">
      <template #header>
        <div class="card-header">
          <span>{{ t('dashboardCpanel.recentActivity') }}</span>
          <el-link type="primary" :underline="false" @click="router.push('/profile')">
            {{ t('dashboardCpanel.viewAllLogs') }}
            <el-icon class="link-arrow"><Icon icon="material-symbols:arrow-forward" /></el-icon>
          </el-link>
        </div>
      </template>
      <div class="act-scroll">
        <el-timeline v-if="activities.length">
          <el-timeline-item
            v-for="(a, i) in activities"
            :key="i"
            :timestamp="fmtTs(a.time)"
            :color="a.color"
            placement="top"
          >
            <div class="act-title">
              <el-icon class="act-icon"><Icon :icon="a.icon" /></el-icon>
              <span>{{ a.title }}</span>
            </div>
            <div v-if="a.sub" class="act-sub muted">{{ a.sub }}</div>
          </el-timeline-item>
        </el-timeline>
        <el-empty v-else :description="t('dashboardCpanel.recentEmpty')" :image-size="60" />
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { Icon, Plus } from '@/icons'
import { formatBytes } from '@/utils/fmt'
import { useUserStore } from '@/stores/user'
import { getSystemInfo } from '@/api/dashboard'
import { getUserInfo, getMyLoginHistory } from '@/api/user'
import type { LoginRecordItem } from '@/api/user'
import { getNotices } from '@/api/notice'
import type { NoticeMessage } from '@/api/notice'
import { getCertList } from '@/api/ssl'
import { listAllApps } from '@/api/site'
import { http } from '@/utils/request'

interface ActivityItem {
  time: number
  icon: string
  color: string
  title: string
  sub?: string
}

const { t } = useI18n()
const router = useRouter()
const userStore = useUserStore()

// ── 统计卡片（普通用户：站点 / 域名 / SSL / 应用） ──────────────
const stats = ref({ total: 0, running: 0, domains: 0, ssl: 0, apps: 0 })

const statCards = computed(() => [
  {
    key: 'sites',
    title: t('dashboardCpanel.statSites'),
    value: stats.value.total,
    icon: 'material-symbols:public',
    color: '#409eff',
    bg: '#ecf5ff',
    span: 6,
  },
  {
    key: 'domains',
    title: t('dashboardCpanel.statDomains'),
    value: stats.value.domains,
    icon: 'material-symbols:language',
    color: '#e6a23c',
    bg: '#fdf6ec',
    span: 6,
  },
  {
    key: 'ssl',
    title: t('dashboardCpanel.statSsl'),
    value: stats.value.ssl,
    icon: 'material-symbols:lock',
    color: '#f56c6c',
    bg: '#fef0f0',
    span: 6,
  },
  {
    key: 'apps',
    title: t('dashboardCpanel.statApps'),
    value: stats.value.apps,
    icon: 'material-symbols:apps',
    color: '#67c23a',
    bg: '#f0f9eb',
    span: 6,
  },
])

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

function goCreateSite() {
  router.push('/site')
}

// ── 常规信息 + 使用情况 ─────────────────────────────────────
const account = ref<Record<string, any>>({})
const server = ref<Record<string, any>>({})
const packageBound = ref(false)
const pkg = ref<Record<string, any>>({})

const fmtTime = (ts: number) => (ts ? new Date(ts * 1000).toLocaleString() : '—')
const fmtTs = (ts: number) => (ts ? new Date(ts * 1000).toLocaleString() : '—')
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

async function loadStats() {
  // 站点数与运行数、域名总数（接口按角色返回可见范围）
  try {
    const res = await http.get<{ code: number; data: any }>('/site/list')
    const d = res.data as any
    let domains = 0
    if (Array.isArray(d?.rows)) {
      d.rows.forEach((s: any) => {
        domains += (s.domains || []).length
      })
    }
    stats.value.total = d?.total || 0
    stats.value.running = d?.running || 0
    stats.value.domains = domains
  } catch {
    /* ignore */
  }
  // SSL 证书数
  try {
    const res = await getCertList()
    const arr = Array.isArray(res?.data) ? (res.data as any[]) : []
    stats.value.ssl = arr.length
  } catch {
    /* ignore */
  }
  // 站点应用数（跨站点，按角色返回可见范围）
  try {
    const payload = (await listAllApps()) as any
    const d = payload?.data
    if (Array.isArray(d)) stats.value.apps = d.length
    else if (d && Array.isArray(d.rows)) stats.value.apps = d.rows.length
    else if (d && typeof d.total === 'number') stats.value.apps = d.total
  } catch {
    /* ignore */
  }
}

// ── 最近动态（登录记录 + 站内信，合并时间线） ──────────────
const activities = ref<ActivityItem[]>([])

async function loadActivity() {
  const [lh, nt] = await Promise.allSettled([
    getMyLoginHistory({ page: 1, page_size: 10 }),
    getNotices({ page: 1, page_size: 10 }),
  ])
  const items: ActivityItem[] = []
  if (lh.status === 'fulfilled' && lh.value?.data) {
    ;(lh.value.data as LoginRecordItem[]).forEach((r) => {
      const ok = r.status === 'success'
      items.push({
        time: r.created_at,
        icon: ok ? 'material-symbols:login' : 'material-symbols:gpp-maybe',
        color: ok ? '#67c23a' : '#f56c6c',
        title:
          r.status === 'success'
            ? t('dashboardCpanel.recentLoginSuccess')
            : r.status === '2fa_failed'
              ? t('dashboardCpanel.recentLogin2faFailed')
              : t('dashboardCpanel.recentLoginFailed'),
        sub: `IP: ${r.ip || '-'}${r.user_agent ? ' · ' + r.user_agent : ''}`,
      })
    })
  }
  if (nt.status === 'fulfilled' && nt.value?.data?.list) {
    ;(nt.value.data.list as NoticeMessage[]).forEach((n) => {
      items.push({
        time: n.created_at,
        icon: 'material-symbols:notifications',
        color: n.is_read ? '#909399' : '#409eff',
        title: n.title,
        sub: n.body,
      })
    })
  }
  items.sort((a, b) => b.time - a.time)
  activities.value = items.slice(0, 10)
}

onMounted(async () => {
  await Promise.all([loadAccount(), loadServer(), loadStats(), loadActivity()])
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
  background: linear-gradient(120deg, var(--el-color-primary-light-9), var(--el-bg-color));
}
.welcome {
  display: flex;
  align-items: center;
  gap: 16px;
}
.welcome-avatar {
  flex: none;
  background: var(--el-color-primary);
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
}
.muted {
  color: var(--el-text-color-secondary);
}

.stat-row {
  margin-bottom: 16px;
}

/* 高度压缩 + 微阴影 */
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

/* 最近动态：固定高度，内部滚动 */
.activity-card {
  margin-bottom: 16px;
}
.act-scroll {
  max-height: 320px;
  overflow-y: auto;
  padding-right: 4px;
}
.link-arrow {
  margin-left: 2px;
  font-size: 14px;
}
.act-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 500;
  color: var(--el-text-color-primary);
}
.act-icon {
  font-size: 18px;
}
.act-sub {
  margin-top: 2px;
  font-size: 12px;
  word-break: break-all;
}
</style>
