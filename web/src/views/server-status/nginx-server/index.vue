<template>
  <div v-loading="loading" class="nginx-server">
    <!-- 未安装引导 -->
    <el-result
      v-if="!installed"
      icon="warning"
      :title="t('statusNginx.notInstalled')"
      :sub-title="t('statusNginx.installTip')"
    >
      <template #extra>
        <el-button type="primary" @click="goAppstore">{{ t('statusNginx.goAppstore') }}</el-button>
      </template>
    </el-result>

    <template v-else>
      <!-- 运行状态横幅 -->
      <div class="hero" :class="running ? 'hero-on' : 'hero-off'">
        <div class="hero-left">
          <span class="hero-dot"></span>
          <div>
            <div class="hero-title">
              {{ running ? t('statusNginx.titleRunning') : t('statusNginx.titleStopped') }}
            </div>
            <div class="hero-sub">
              {{ status.systemd ? t('statusNginx.systemdUnit') : t('statusNginx.binDaemon') }}
              <template v-if="status.pid"> · PID {{ status.pid }}</template>
              <template v-if="versionText && versionText !== '-'"> · {{ versionText }}</template>
            </div>
          </div>
        </div>
        <div class="hero-right">
          <span class="updated">
            <el-icon><Timer /></el-icon>
            {{ t('statusNginx.updatedAt', { time: lastUpdated }) }}
          </span>
          <el-button size="small" :loading="refreshing" circle @click="refresh">
            <el-icon><Refresh /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 基础信息（左） + 服务控制（右） -->
      <el-row :gutter="16" class="mt-3">
        <el-col :xs="24" :lg="16">
          <el-card shadow="never" class="h-full">
            <template #header>
              <div class="card-header">
                <span>{{ t('statusNginx.basicTitle') }}</span>
              </div>
            </template>
            <div class="info-grid">
              <div class="info-item">
                <div class="info-label">
                  <el-icon><Odometer /></el-icon>{{ t('statusNginx.runState') }}
                </div>
                <div class="info-value">
                  <el-tag :type="running ? 'success' : 'danger'">
                    {{ running ? t('statusNginx.stateRunning') : t('statusNginx.stateStopped') }}
                  </el-tag>
                </div>
              </div>
              <div class="info-item">
                <div class="info-label">
                  <el-icon><InfoFilled /></el-icon>{{ t('statusNginx.version') }}
                </div>
                <div class="info-value mono">{{ versionText }}</div>
              </div>
              <div class="info-item">
                <div class="info-label">
                  <el-icon><Document /></el-icon>{{ t('statusNginx.mainConf') }}
                </div>
                <div class="info-value mono sm">{{ status.conf_file || '-' }}</div>
              </div>
              <div class="info-item">
                <div class="info-label">
                  <el-icon><Cpu /></el-icon>{{ t('statusNginx.binary') }}
                </div>
                <div class="info-value mono sm">{{ status.bin || '-' }}</div>
              </div>
            </div>
          </el-card>
        </el-col>

        <!-- 服务控制 -->
        <el-col :xs="24" :lg="8">
          <el-card shadow="never" class="h-full">
            <template #header>
              <div class="card-header">
                <span>{{ t('statusNginx.ctrlTitle') }}</span>
              </div>
            </template>
            <div class="ctrl-row">
              <el-button
                type="primary"
                :disabled="!!(!running || acting)"
                :loading="acting === 'reload'"
                @click="control('reload')"
              >
                {{ t('statusNginx.reload') }}
              </el-button>
              <el-button
                type="warning"
                :disabled="!!(!running || acting)"
                :loading="acting === 'restart'"
                @click="control('restart')"
              >
                {{ t('statusNginx.restart') }}
              </el-button>
              <el-button
                type="success"
                :disabled="!!(running || acting)"
                :loading="acting === 'start'"
                @click="control('start')"
              >
                {{ t('statusNginx.start') }}
              </el-button>
              <el-button
                type="danger"
                plain
                :disabled="!!(!running || acting)"
                :loading="acting === 'stop'"
                @click="control('stop')"
              >
                {{ t('statusNginx.stop') }}
              </el-button>
            </div>
            <div class="ctrl-tip">{{ t('statusNginx.ctrlTip') }}</div>
          </el-card>
        </el-col>
      </el-row>

      <!-- 性能监控（stub_status） -->
      <el-card shadow="never" class="mt-3">
        <template #header>
          <div class="card-header">
            <span>{{ t('statusNginx.perfTitle') }}</span>
            <span class="header-tip">{{ t('statusNginx.perfSub') }}</span>
          </div>
        </template>

        <!-- 启用开关 -->
        <div class="stub-head">
          <div>
            <div class="stub-title">{{ t('statusNginx.stubTitle') }}</div>
            <div class="stub-desc">{{ t('statusNginx.stubDesc') }}</div>
            <div v-if="stub.port" class="stub-endpoint">
              {{
                t('statusNginx.stubEndpoint', {
                  url: `127.0.0.1:${stub.port}${stub.path ?? '/nginx_status'}`,
                })
              }}
            </div>
          </div>
          <el-switch v-model="stubEnabled" :loading="savingStub" @change="toggleStub" />
        </div>

        <template v-if="hasMetrics">
          <!-- 性能指标 -->
          <div class="metric-grid">
            <div class="metric">
              <div class="metric-label">{{ t('statusNginx.metricMaxRps') }}</div>
              <div class="metric-value">{{ fmt(maxRps) }}</div>
              <div class="metric-tip">{{ t('statusNginx.metricMaxRpsTip') }}</div>
            </div>
            <div class="metric">
              <div class="metric-label">{{ t('statusNginx.metricMaxConn') }}</div>
              <div class="metric-value">{{ fmt(maxConn) }}</div>
              <div class="metric-tip">worker_connections</div>
            </div>
            <div class="metric">
              <div class="metric-label">{{ t('statusNginx.metricPerConn') }}</div>
              <div class="metric-value">{{ perConn }}</div>
              <div class="metric-tip">{{ t('statusNginx.metricPerConnTip') }}</div>
            </div>
            <div class="metric">
              <div class="metric-label">{{ t('statusNginx.metricProcTotal') }}</div>
              <div class="metric-value">{{ processes.total }}</div>
              <div class="metric-tip">{{ t('statusNginx.metricProcTotalTip') }}</div>
            </div>
          </div>

          <!-- 活动连接 / 读写等待 / 工作进程 -->
          <el-row :gutter="16" class="mt-3">
            <el-col :xs="24" :sm="8">
              <div class="mini">
                <div class="mini-head">
                  <span>{{ t('statusNginx.activeConn') }}</span>
                  <span class="mini-num"
                    >{{ metrics.active }} <span class="mini-total">/ {{ fmt(maxConn) }}</span></span
                  >
                </div>
                <el-progress :percentage="activeRatio" :show-text="false" :stroke-width="8" />
              </div>
            </el-col>
            <el-col :xs="24" :sm="8">
              <div class="mini">
                <div class="mini-head">
                  <span>{{ t('statusNginx.workers') }}</span>
                  <span class="mini-num"
                    >{{ processes.workers }}
                    <span class="mini-total">/ {{ wpNum || '-' }}</span></span
                  >
                </div>
                <el-progress
                  :percentage="workersRatio"
                  :show-text="false"
                  :stroke-width="8"
                  status="success"
                />
              </div>
            </el-col>
            <el-col :xs="24" :sm="8">
              <div class="mini">
                <div class="mini-head">
                  <span>{{ t('statusNginx.rwTitle') }}</span>
                </div>
                <div class="rw-row">
                  <span class="rw"
                    ><span class="rw-dot rw-r"></span
                    >{{ t('statusNginx.reading', { n: metrics.reading }) }}</span
                  >
                  <span class="rw"
                    ><span class="rw-dot rw-w"></span
                    >{{ t('statusNginx.writing', { n: metrics.writing }) }}</span
                  >
                  <span class="rw"
                    ><span class="rw-dot rw-wa"></span
                    >{{ t('statusNginx.waiting', { n: metrics.waiting }) }}</span
                  >
                </div>
              </div>
            </el-col>
          </el-row>

          <!-- 详情 tabs -->
          <el-tabs type="border-card" class="mt-3 detail-tabs">
            <el-tab-pane :label="t('statusNginx.tabRequests')">
              <el-table :data="stubRows" size="small">
                <el-table-column prop="label" :label="t('statusNginx.colMetric')" min-width="160" />
                <el-table-column prop="value" :label="t('statusNginx.colValue')" min-width="160" />
              </el-table>
            </el-tab-pane>
            <el-tab-pane :label="t('statusNginx.tabProcesses')">
              <div class="proc-list">
                <div v-for="p in procBars" :key="p.label" class="proc-item">
                  <span class="proc-name"
                    ><span class="proc-dot" :style="{ background: p.color }"></span
                    >{{ p.label }}</span
                  >
                  <el-progress
                    :percentage="p.pct"
                    :show-text="false"
                    :stroke-width="10"
                    :color="p.color"
                    class="proc-bar"
                  />
                  <span class="proc-num">{{ p.value }}</span>
                </div>
                <div class="proc-note">
                  {{ t('statusNginx.procTotal', { n: processes.total }) }}
                </div>
              </div>
            </el-tab-pane>
            <el-tab-pane :label="t('statusNginx.tabConfig')">
              <el-table :data="confRows" size="small">
                <el-table-column prop="label" :label="t('statusNginx.colMetric')" min-width="160" />
                <el-table-column prop="value" :label="t('statusNginx.colValue')" min-width="160" />
              </el-table>
              <div class="ideal">
                <div class="ideal-title">{{ t('statusNginx.idealTitle') }}</div>
                <div class="ideal-row">
                  {{ t('statusNginx.idealMaxConn') }}<b>{{ fmt(maxConn) }}</b>
                </div>
                <div class="ideal-row">
                  {{ t('statusNginx.idealMaxRps') }}<b>{{ fmt(maxRps) }}</b>
                </div>
                <div class="ideal-row">
                  {{ t('statusNginx.idealMaxWorkers') }}<b>{{ wpNum }}</b
                  >（{{ workerProcessesText }}）
                </div>
                <div class="ideal-tip">{{ t('statusNginx.idealTip') }}</div>
              </div>
            </el-tab-pane>
          </el-tabs>
        </template>

        <template v-else>
          <el-empty
            :description="running ? t('statusNginx.emptyRunning') : t('statusNginx.emptyStopped')"
            :image-size="80"
          />
        </template>
      </el-card>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Odometer, InfoFilled, Document, Cpu, Refresh, Timer } from '@/icons'
import {
  controlNginx,
  getNginxStatus,
  getNginxStubStatus,
  setNginxStubStatus,
  type NginxStatus,
  type NginxStubMetrics,
  type NginxStubStatus,
} from '@/api/serverNginx.ts'

const EMPTY_METRICS: NginxStubMetrics = {
  active: 0,
  accepts: 0,
  handled: 0,
  requests: 0,
  reading: 0,
  writing: 0,
  waiting: 0,
}

const { t } = useI18n()
const router = useRouter()
/** 只有首屏与手动刷新才遮罩：定时轮询静默更新，避免整页每 5 秒抖一下 */
const loading = ref(false)
const refreshing = ref(false)
const acting = ref('')
const savingStub = ref(false)
const status = ref<NginxStatus>({ installed: false })
const stub = ref<NginxStubStatus>({ enabled: false })
/** 上一次采集成功的指标：本次没采到但 nginx 还在跑时沿用，避免内容整块塌陷 */
const lastMetrics = ref<NginxStubMetrics | null>(null)
const lastUpdated = ref('—')
let inited = false

let timer: ReturnType<typeof setInterval> | undefined
let destroyed = false

const installed = computed(() => !!status.value.installed)
const running = computed(() => !!status.value.running)
const versionText = computed(() => {
  const v = status.value.version || ''
  const m = v.match(/nginx\/([\d.]+)/)
  return m ? m[1] : v || '-'
})

const stubEnabled = computed<boolean>({
  get: () => stub.value.enabled ?? false,
  set: (v: boolean) => {
    stub.value.enabled = v
  },
})
const metrics = computed<NginxStubMetrics>(
  () => stub.value.metrics ?? lastMetrics.value ?? EMPTY_METRICS,
)
const hasMetrics = computed(() => !!(stub.value.metrics ?? lastMetrics.value))
const processes = computed(
  () => stub.value.processes ?? { master: 0, workers: 0, cache: 0, total: 0 },
)

// worker_processes：数字直接取值；auto 用实际工作进程数；未知为 0
const wpNum = computed<number>(() => {
  const wp = stub.value.worker_processes || ''
  if (/^\d+$/.test(wp)) return Number(wp)
  if (wp.toLowerCase() === 'auto') return processes.value.workers || 0
  return 0
})
const wcNum = computed<number>(() => Number(stub.value.worker_connections) || 0)
const workerProcessesText = computed(() => {
  const wp = stub.value.worker_processes || ''
  return wp ? (wp.toLowerCase() === 'auto' ? t('statusNginx.workerAuto') : wp) : '—'
})
const maxRps = computed(() => wpNum.value * wcNum.value)
const maxConn = computed(() => wcNum.value)
const perConn = computed(() => {
  const m = metrics.value
  if (!m || !m.handled) return '0'
  return (m.requests / m.handled).toFixed(2)
})
const activeRatio = computed(() => {
  const m = metrics.value
  if (!m || !maxConn.value) return 0
  return Math.min(100, (m.active / maxConn.value) * 100)
})
const workersRatio = computed(() => {
  if (!wpNum.value || !processes.value.workers) return 0
  return Math.min(100, (processes.value.workers / wpNum.value) * 100)
})

const stubRows = computed(() => {
  const m = metrics.value
  const fmt = (n?: number) => (n == null ? '-' : n.toLocaleString())
  return [
    { label: t('statusNginx.stubRowActive'), value: fmt(m?.active) },
    { label: t('statusNginx.stubRowAccepts'), value: fmt(m?.accepts) },
    { label: t('statusNginx.stubRowHandled'), value: fmt(m?.handled) },
    { label: t('statusNginx.stubRowRequests'), value: fmt(m?.requests) },
    { label: t('statusNginx.stubRowReading'), value: fmt(m?.reading) },
    { label: t('statusNginx.stubRowWriting'), value: fmt(m?.writing) },
    { label: t('statusNginx.stubRowWaiting'), value: fmt(m?.waiting) },
  ]
})

const confRows = computed(() => [
  { label: t('statusNginx.confWorkerProcesses'), value: workerProcessesText.value },
  { label: t('statusNginx.confWorkerConn'), value: wcNum.value ? String(wcNum.value) : '—' },
])

const procBars = computed(() => {
  const scale = Math.max(processes.value.total, 1)
  const pct = (n: number) => Math.round((n / scale) * 100)
  return [
    {
      label: t('statusNginx.procMaster'),
      value: processes.value.master,
      color: '#409EFF',
      pct: pct(processes.value.master),
    },
    {
      label: t('statusNginx.procWorkers'),
      value: processes.value.workers,
      color: '#67C23A',
      pct: pct(processes.value.workers),
    },
    {
      label: t('statusNginx.procCache'),
      value: processes.value.cache,
      color: '#E6A23C',
      pct: pct(processes.value.cache),
    },
  ]
})

function fmt(n: number): string {
  return n.toLocaleString()
}

function formatTime(d: Date): string {
  const p = (x: number) => String(x).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

/** 静默刷新：定时轮询走这里，不触发遮罩 */
async function load() {
  const [sr, st] = await Promise.all([
    getNginxStatus().catch(() => null),
    getNginxStubStatus().catch(() => null),
  ])
  if (!destroyed) {
    if (sr && sr.code === 0) status.value = sr.data
    if (st && st.code === 0) {
      stub.value = st.data
      if (st.data.metrics) lastMetrics.value = st.data.metrics
      // nginx 明确已停 / 状态页已关才丢掉旧数据；运行中偶发采集失败时保留上一次，避免页面塌陷
      else if (!status.value.running || !st.data.enabled) lastMetrics.value = null
    }
    lastUpdated.value = formatTime(new Date())
  }
}

/** 手动刷新：带遮罩，走完整加载态 */
async function refresh() {
  if (!inited) loading.value = true
  refreshing.value = true
  try {
    await load()
  } finally {
    inited = true
    loading.value = false
    refreshing.value = false
  }
}

const controlLabels = computed<Record<string, string>>(() => ({
  reload: t('statusNginx.reload'),
  restart: t('statusNginx.restart'),
  start: t('statusNginx.start'),
  stop: t('statusNginx.stop'),
}))

async function control(action: 'reload' | 'restart' | 'start' | 'stop') {
  const tip = controlLabels.value[action]
  const warn = action === 'stop'
  try {
    await ElMessageBox.confirm(t('statusNginx.confirmAction', { action: tip }), t('common.tip'), {
      type: warn ? 'warning' : 'info',
    })
  } catch {
    return
  }
  acting.value = action
  try {
    const res = await controlNginx(action)
    ElMessage.success(res.message ?? t('statusNginx.actionOk', { action: tip }))
    await load()
  } catch {
    /* handled by interceptor */
  } finally {
    acting.value = ''
  }
}

async function toggleStub(v: boolean | string | number) {
  const enable = !!v
  savingStub.value = true
  try {
    await setNginxStubStatus(enable)
    ElMessage.success(t(enable ? 'statusNginx.stubOn' : 'statusNginx.stubOff'))
    await load()
  } catch {
    /* handled by interceptor */
    await load()
  } finally {
    savingStub.value = false
  }
}

function goAppstore() {
  router.push('/appstore')
}

onMounted(async () => {
  await refresh()
  if (destroyed) return
  timer = setInterval(load, 5000)
})

onUnmounted(() => {
  destroyed = true
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.nginx-server {
  min-height: 200px;
}

/* 状态横幅 */
.hero {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 18px 20px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  background: var(--el-bg-color);
}
.hero-on {
  border-left: 4px solid #67c23a;
}
.hero-off {
  border-left: 4px solid #f56c6c;
}
.hero-left {
  display: flex;
  align-items: center;
  gap: 12px;
}
.hero-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: #67c23a;
  box-shadow: 0 0 0 4px rgba(103, 194, 58, 0.15);
}
.hero-off .hero-dot {
  background: #f56c6c;
  box-shadow: 0 0 0 4px rgba(245, 108, 108, 0.15);
}
.hero-title {
  font-size: 17px;
  font-weight: 600;
}
.hero-sub {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.hero-right {
  display: flex;
  align-items: center;
  gap: 12px;
}
.updated {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

/* 基础信息卡（四项合并进一张卡） */
.h-full {
  height: 100%;
}
.info-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 16px 24px;
}
.info-item {
  min-width: 0;
}
.info-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 10px;
}
.info-value {
  font-size: 15px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  word-break: break-all;
}
.info-value.sm {
  font-size: 13px;
  font-weight: 500;
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.header-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  font-weight: normal;
}
.ctrl-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
}
.ctrl-tip {
  margin-top: 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}

/* stub_status */
.stub-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 4px 0 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.stub-title {
  font-size: 14px;
  font-weight: 600;
}
.stub-desc {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}
.stub-endpoint {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
}

.metric-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 14px 28px;
  margin-top: 18px;
}
.metric {
  padding: 4px 0;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.metric-label {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.metric-value {
  margin: 6px 0 2px;
  font-size: 24px;
  font-weight: 700;
  color: var(--el-color-primary);
  /* 等宽数字：轮询刷新时数字跳动不会带着宽度一起抖 */
  font-variant-numeric: tabular-nums;
}
.metric-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.mini {
  height: 100%;
  padding: 14px 16px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
}
.mini-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
  font-size: 13px;
  color: var(--el-text-color-primary);
}
.mini-num {
  font-size: 20px;
  font-weight: 700;
  color: var(--el-color-primary);
  font-variant-numeric: tabular-nums;
}
.mini-total {
  font-size: 12px;
  font-weight: 500;
  color: var(--el-text-color-secondary);
}
.rw-row {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  align-items: center;
  padding-top: 6px;
}
.rw {
  font-size: 13px;
  color: var(--el-text-color-regular);
}
.rw-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 6px;
}
.rw-r {
  background: #409eff;
}
.rw-w {
  background: #e6a23c;
}
.rw-wa {
  background: #909399;
}

.detail-tabs {
  margin-top: 16px;
}

.proc-list {
  padding: 6px 0;
}
.proc-item {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
}
.proc-name {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 100px;
  font-size: 13px;
  color: var(--el-text-color-regular);
}
.proc-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.proc-bar {
  flex: 1;
}
.proc-num {
  min-width: 40px;
  text-align: right;
  font-size: 13px;
  color: var(--el-text-color-primary);
}
.proc-note {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.ideal {
  margin-top: 14px;
  padding: 14px 18px;
  background: var(--el-color-primary-light-9);
  border-radius: 6px;
}
.ideal-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 10px;
  color: var(--el-text-color-primary);
}
.ideal-row {
  font-size: 13px;
  color: var(--el-text-color-regular);
  line-height: 2;
}
.ideal-row b {
  color: var(--el-color-primary);
}
.ideal-tip {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.mt-3 {
  margin-top: 12px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
}
</style>
