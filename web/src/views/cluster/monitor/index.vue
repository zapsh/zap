<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
/**
 * 多节点监控大屏。
 *
 * 只打一个接口 `/node/overview`：所有启用节点的最新采样值 + 集群汇总 + 近 30 分钟趋势，
 * 全部读主控自己的库，**不向节点发请求**——所以某台节点卡住 / 掉线，
 * 大屏照常秒开，只是那张卡片显示离线，不会拖垮整屏。
 *
 * 轮询间隔跟着后端采集间隔走（`summary.interval`），前端再快也拿不到新数据。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { formatBytes } from '@/utils/fmt'
import { nodeContext, nodeViewPath, setNodeContext } from '@/utils/base'
import { getNodesOverview, type NodeOverviewItem, type NodeSummary } from '@/api/node'
import NodeTrend from './NodeTrend.vue'

const { t } = useI18n()
const router = useRouter()

const loading = ref(true)
const nodes = ref<NodeOverviewItem[]>([])
const summary = ref<NodeSummary | null>(null)
const interval = ref(15)
const sort = ref<'id' | 'cpu' | 'load' | 'name'>('cpu')
const drawer = ref(false)
const active = ref<NodeOverviewItem | null>(null)
const countdown = ref(15)

let timer: ReturnType<typeof setInterval> | undefined
let destroyed = false

const sorted = computed(() => {
  const list = [...nodes.value]
  // 离线节点一律沉底：大屏上要看的是「活着且有问题」的机器
  const alive = list.filter((n) => n.online)
  const dead = list.filter((n) => !n.online)
  const cmp: Record<string, (a: NodeOverviewItem, b: NodeOverviewItem) => number> = {
    id: (a, b) => a.id - b.id,
    name: (a, b) => a.name.localeCompare(b.name),
    cpu: (a, b) => (b.cpu_usage ?? 0) - (a.cpu_usage ?? 0),
    load: (a, b) => (b.loadavg_one ?? 0) - (a.loadavg_one ?? 0),
  }
  const f = cmp[sort.value] ?? cmp.id
  return [...alive.sort(f), ...dead.sort(cmp.id)]
})

const memPct = computed(() => {
  const s = summary.value
  if (!s || !s.memory_total_b) return 0
  return Math.round((s.memory_used_b / s.memory_total_b) * 100)
})
const diskPct = computed(() => {
  const s = summary.value
  if (!s || !s.disk_total_b) return 0
  return Math.round((s.disk_used_b / s.disk_total_b) * 100)
})

/** 使用率配色：<70 正常，70~85 提醒，>85 危险 */
function level(v: number | null): 'ok' | 'warn' | 'bad' {
  const n = Number(v ?? 0)
  if (n >= 85) return 'bad'
  if (n >= 70) return 'warn'
  return 'ok'
}

/** 相对时间：大屏上「3 分钟前」比完整时间戳有用 */
function ago(ts: number | null): string {
  if (!ts) return t('cluster.never')
  const d = Math.max(0, Math.floor(Date.now() / 1000) - ts)
  if (d < 60) return t('cluster.secondsAgo', { n: d })
  if (d < 3600) return t('cluster.minutesAgo', { n: Math.floor(d / 60) })
  return t('cluster.hoursAgo', { n: Math.floor(d / 3600) })
}

/**
 * 迷你趋势线：直接画 SVG path，不给每张卡片建 Chart 实例。
 * 20 台节点 × Chart.js 的 canvas 开销在轮询下会明显掉帧，而 sparkline 只需要一条线。
 */
function sparkPath(trend: NodeOverviewItem['trend'], key: 'cpu' | 'mem'): string {
  if (!trend || trend.length < 2) return ''
  const w = 100
  const h = 28
  const vals = trend.map((p) => Math.max(0, Math.min(100, Number(p[key] ?? 0))))
  const step = w / (vals.length - 1)
  return vals
    .map(
      (v, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)},${(h - (v / 100) * h).toFixed(1)}`,
    )
    .join(' ')
}

function openNode(n: NodeOverviewItem) {
  active.value = n
  drawer.value = true
}

/**
 * 进入节点视图：这是一次**完整接管**，面板请求改走主控的 /api/nodes/{id}/** 代理。
 * 必须整页重载 —— 登录身份、菜单与权限矩阵都要改由该节点给出（代理会带上节点
 * API Token），而这些都在应用启动时拉取，SPA 内跳路由拿不到远端结果。
 */
function enterNodeView(n: NodeOverviewItem) {
  setNodeContext({ id: n.id, name: n.name, baseUrl: n.base_url })
  window.location.href = nodeViewPath(router, '/server-status/index')
}

async function load() {
  try {
    const res = await getNodesOverview()
    if (destroyed) return
    nodes.value = res.data?.nodes || []
    summary.value = res.data?.summary || null
    if (res.data?.interval) {
      interval.value = res.data.interval
      countdown.value = res.data.interval
    }
  } catch {
    /* 拦截器已弹窗；下一轮自动重试 */
  } finally {
    loading.value = false
  }
}

function startTicker() {
  if (timer) clearInterval(timer)
  timer = setInterval(() => {
    countdown.value -= 1
    if (countdown.value <= 0) {
      countdown.value = interval.value
      load()
    }
  }, 1000)
}

onMounted(async () => {
  // 大屏只读主控自己的注册表与采样库，属于**主控页面**。若正处在节点视图里打开它，
  // 就等于离开了那台机器的上下文：这里直接退出接管并整页重载，让身份/菜单回到主控。
  // 否则下钻的历史曲线（`/nodes/{id}/history` 由主控服务、不经代理）会被发到节点上报错。
  if (nodeContext.value) {
    setNodeContext(null)
    window.location.reload()
    return
  }
  await load()
  if (destroyed) return
  countdown.value = interval.value
  startTicker()
})

onUnmounted(() => {
  destroyed = true
  if (timer) clearInterval(timer)
})
</script>

<template>
  <div class="cluster-screen">
    <div class="screen-toolbar">
      <el-select v-model="sort" size="small" style="width: 150px">
        <el-option :label="t('cluster.sortCpu')" value="cpu" />
        <el-option :label="t('cluster.sortLoad')" value="load" />
        <el-option :label="t('cluster.sortName')" value="name" />
        <el-option :label="t('cluster.sortId')" value="id" />
      </el-select>
      <div class="toolbar-right">
        <span class="meta">{{ t('cluster.autoRefresh', { sec: countdown }) }}</span>
        <el-button size="small" :loading="loading" @click="load">
          {{ t('common.refresh') }}
        </el-button>
      </div>
    </div>

    <!-- 集群汇总 -->
    <el-row v-if="summary" :gutter="12" class="summary-row">
      <el-col :xs="12" :sm="8" :md="5">
        <div class="stat">
          <div class="stat-label">{{ t('cluster.statNodes') }}</div>
          <div class="stat-value">
            {{ summary.online }}<span class="stat-sub">/ {{ summary.total }}</span>
          </div>
          <div class="stat-foot">
            <span class="dot on"></span>{{ t('cluster.online') }} <span class="dot off"></span
            >{{ t('cluster.offline') }} {{ summary.offline }}
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="8" :md="5">
        <div class="stat">
          <div class="stat-label">{{ t('cluster.statAvgCpu') }}</div>
          <div class="stat-value">{{ summary.avg_cpu_usage.toFixed(1) }}%</div>
          <div class="stat-bar">
            <i
              :class="level(summary.avg_cpu_usage)"
              :style="{ width: `${Math.min(100, summary.avg_cpu_usage)}%` }"
            ></i>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="8" :md="5">
        <div class="stat">
          <div class="stat-label">{{ t('cluster.statAvgLoad') }}</div>
          <div class="stat-value">{{ summary.avg_load.toFixed(2) }}</div>
          <div class="stat-foot">
            {{ t('cluster.maxLoad', { v: summary.max_load.toFixed(2) }) }}
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="8" :md="5">
        <div class="stat">
          <div class="stat-label">{{ t('cluster.statMemory') }}</div>
          <div class="stat-value">{{ memPct }}%</div>
          <div class="stat-bar">
            <i :class="level(memPct)" :style="{ width: `${memPct}%` }"></i>
          </div>
          <div class="stat-foot">
            {{ formatBytes(summary.memory_used_b) }} / {{ formatBytes(summary.memory_total_b) }}
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="8" :md="4">
        <div class="stat">
          <div class="stat-label">{{ t('cluster.statDisk') }}</div>
          <div class="stat-value">{{ diskPct }}%</div>
          <div class="stat-bar">
            <i :class="level(diskPct)" :style="{ width: `${diskPct}%` }"></i>
          </div>
          <div class="stat-foot">
            {{ formatBytes(summary.disk_used_b) }} / {{ formatBytes(summary.disk_total_b) }}
          </div>
        </div>
      </el-col>
    </el-row>

    <el-empty v-if="!loading && !nodes.length" :description="t('cluster.noNodes')">
      <el-button type="primary" @click="router.push('/cluster/nodes')">
        {{ t('cluster.addNode') }}
      </el-button>
    </el-empty>

    <!-- 节点卡片网格 -->
    <div v-else v-loading="loading" class="grid">
      <div
        v-for="n in sorted"
        :key="n.id"
        class="card"
        :class="{ 'card-off': !n.online }"
        @click="openNode(n)"
      >
        <div class="card-head">
          <div class="card-title">
            <span class="dot" :class="n.online ? 'on' : 'off'"></span>
            <span class="card-name">{{ n.name }}</span>
          </div>
          <el-tag v-if="n.version" size="small" type="info">{{ n.version }}</el-tag>
        </div>
        <div class="card-sub">{{ n.host_name || n.base_url }}</div>

        <el-tooltip v-if="!n.online && n.last_error" :content="n.last_error" placement="top">
          <div class="card-error">{{ n.last_error }}</div>
        </el-tooltip>

        <template v-if="n.online">
          <div class="metrics">
            <div class="metric">
              <div class="metric-label">{{ t('systemMonitor.cpu') }}</div>
              <div class="metric-value" :class="level(n.cpu_usage)">
                {{ (n.cpu_usage ?? 0).toFixed(1) }}%
              </div>
              <div class="bar">
                <i
                  :class="level(n.cpu_usage)"
                  :style="{ width: `${Math.min(100, n.cpu_usage ?? 0)}%` }"
                ></i>
              </div>
            </div>
            <div class="metric">
              <div class="metric-label">{{ t('systemMonitor.memory') }}</div>
              <div class="metric-value" :class="level(n.memory_usage)">
                {{ (n.memory_usage ?? 0).toFixed(1) }}%
              </div>
              <div class="bar">
                <i
                  :class="level(n.memory_usage)"
                  :style="{ width: `${Math.min(100, n.memory_usage ?? 0)}%` }"
                ></i>
              </div>
            </div>
            <div class="metric">
              <div class="metric-label">{{ t('cluster.diskUsage') }}</div>
              <div class="metric-value" :class="level(n.disk_usage)">
                {{ (n.disk_usage ?? 0).toFixed(1) }}%
              </div>
              <div class="bar">
                <i
                  :class="level(n.disk_usage)"
                  :style="{ width: `${Math.min(100, n.disk_usage ?? 0)}%` }"
                ></i>
              </div>
            </div>
          </div>

          <div class="load-row">
            <span class="load-label">{{ t('systemMonitor.systemLoad') }}</span>
            <span class="load-value">
              {{ (n.loadavg_one ?? 0).toFixed(2) }} / {{ (n.loadavg_five ?? 0).toFixed(2) }} /
              {{ (n.loadavg_fifteen ?? 0).toFixed(2) }}
            </span>
            <span v-if="n.cpu_num" class="load-cores">× {{ n.cpu_num }}</span>
          </div>

          <svg class="spark" viewBox="0 0 100 28" preserveAspectRatio="none">
            <path
              :d="sparkPath(n.trend, 'cpu')"
              fill="none"
              stroke="var(--el-color-primary)"
              stroke-width="1.4"
              vector-effect="non-scaling-stroke"
            />
            <path
              :d="sparkPath(n.trend, 'mem')"
              fill="none"
              stroke="var(--el-color-success)"
              stroke-width="1.4"
              vector-effect="non-scaling-stroke"
            />
          </svg>
          <div class="spark-legend">
            <span><i class="lg lg-cpu"></i>{{ t('systemMonitor.cpu') }}</span>
            <span><i class="lg lg-mem"></i>{{ t('systemMonitor.memory') }}</span>
            <span class="spark-range">{{ t('cluster.trendRange') }}</span>
          </div>
        </template>

        <div class="card-foot">
          <span>{{ t('cluster.sampledAt', { time: ago(n.sampled_at) }) }}</span>
          <span v-if="n.uptime">{{ t('cluster.uptime') }} {{ n.uptime }}</span>
        </div>
      </div>
    </div>

    <!-- 下钻：单节点历史曲线 -->
    <el-drawer v-model="drawer" size="720px" destroy-on-close>
      <template #header>
        <div style="display: flex; align-items: center; gap: 12px">
          <span style="font-size: 16px; font-weight: 600">{{ active?.name }}</span>
          <el-button
            v-if="active && active.online"
            size="small"
            type="primary"
            plain
            @click="enterNodeView(active)"
          >
            {{ t('cluster.enterNodeView') }}
          </el-button>
        </div>
      </template>
      <NodeTrend v-if="active" :node-id="active.id" />
    </el-drawer>
  </div>
</template>

<style scoped>
.screen-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.toolbar-right {
  display: flex;
  align-items: center;
  gap: 10px;
}
.meta {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.summary-row {
  margin-bottom: 4px;
}
.stat {
  padding: 12px 14px;
  margin-bottom: 12px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 10px;
  background: var(--el-bg-color);
}
.stat-label {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.stat-value {
  font-size: 22px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  line-height: 1.4;
}
.stat-sub {
  font-size: 13px;
  font-weight: 500;
  color: var(--el-text-color-secondary);
  margin-left: 4px;
}
.stat-bar {
  height: 4px;
  border-radius: 2px;
  background: var(--el-fill-color);
  overflow: hidden;
  margin: 4px 0 6px;
}
.stat-bar i {
  display: block;
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s ease;
}
.stat-foot {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  display: flex;
  align-items: center;
  gap: 6px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 14px;
  min-height: 200px;
}
.card {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 12px;
  padding: 14px;
  background: var(--el-bg-color);
  cursor: pointer;
  transition:
    box-shadow 0.2s ease,
    border-color 0.2s ease;
}
.card:hover {
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.08);
  border-color: var(--el-color-primary-light-5);
}
.card-off {
  opacity: 0.75;
}
.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.card-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.card-name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.card-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin: 2px 0 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.card-error {
  font-size: 12px;
  color: var(--el-color-danger);
  margin-bottom: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
  display: inline-block;
}
.dot.on {
  background: var(--el-color-success);
}
.dot.off {
  background: var(--el-text-color-placeholder);
}
.metrics {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
}
.metric-label {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.metric-value {
  font-size: 17px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.bar {
  height: 4px;
  border-radius: 2px;
  background: var(--el-fill-color);
  overflow: hidden;
  margin-top: 4px;
}
.bar i {
  display: block;
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s ease;
}
/* 数值与进度条共用同一 level()：文字着色靠 .ok/.warn/.bad，条子靠 .bar i.xxx */
.ok {
  color: var(--el-color-success);
}
.bar i.ok {
  background: var(--el-color-success);
}
.warn {
  color: var(--el-color-warning);
}
.bar i.warn {
  background: var(--el-color-warning);
}
.bad {
  color: var(--el-color-danger);
}
.bar i.bad {
  background: var(--el-color-danger);
}
.stat-bar i.ok {
  background: var(--el-color-success);
}
.stat-bar i.warn {
  background: var(--el-color-warning);
}
.stat-bar i.bad {
  background: var(--el-color-danger);
}
.load-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-top: 12px;
  font-size: 13px;
}
.load-label {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.load-value {
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}
.load-cores {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.spark {
  width: 100%;
  height: 34px;
  margin-top: 8px;
  overflow: visible;
}
.spark-legend {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 11px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}
.spark-range {
  margin-left: auto;
}
.lg {
  display: inline-block;
  width: 8px;
  height: 2px;
  border-radius: 1px;
  margin-right: 4px;
  vertical-align: middle;
}
.lg-cpu {
  background: var(--el-color-primary);
}
.lg-mem {
  background: var(--el-color-success);
}
.card-foot {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px solid var(--el-border-color-lighter);
  font-size: 11px;
  color: var(--el-text-color-secondary);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
