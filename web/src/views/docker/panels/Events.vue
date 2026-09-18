<template>
  <div class="events">
    <div class="events__toolbar">
      <el-tag size="small" effect="dark" :type="statusType">{{ statusText }}</el-tag>
      <el-select
        v-model="typeFilter"
        size="small"
        clearable
        :style="{ width: '140px' }"
        :placeholder="t('docker.events.filterType')"
      >
        <el-option v-for="o in EVENT_TYPES" :key="o" :label="o" :value="o" />
      </el-select>
      <el-input
        v-model="keyword"
        size="small"
        clearable
        :style="{ width: '180px' }"
        :placeholder="t('docker.events.filterKeyword')"
      />
      <span class="events__hint">{{ t('docker.events.limit', { n: MAX }) }}</span>
      <div class="events__spacer" />
      <el-button size="small" @click="toggle">
        {{ live ? t('docker.events.pause') : t('docker.events.resume') }}
      </el-button>
      <el-button size="small" :icon="Delete" @click="clear">{{ t('docker.events.clear') }}</el-button>
    </div>

    <el-table :data="filtered" size="small" height="100%" stripe>
      <el-table-column :label="t('docker.events.time')" width="170">
        <template #default="{ row }">{{ fmtTime(row.time) }}</template>
      </el-table-column>
      <el-table-column :label="t('docker.events.type')" width="110">
        <template #default="{ row }">
          <el-tag size="small" effect="plain" :type="tagType(row.type)">{{ row.type }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('docker.events.action')" width="140" prop="action" />
      <el-table-column :label="t('docker.events.object')" min-width="180">
        <template #default="{ row }">{{ row.name || row.id }}</template>
      </el-table-column>
      <el-table-column :label="t('docker.events.image')" min-width="200" prop="image" show-overflow-tooltip />
      <el-table-column :label="t('docker.common.id')" width="130">
        <template #default="{ row }">
          <span class="events__mono">{{ row.id.slice(0, 12) }}</span>
        </template>
      </el-table-column>
      <el-table-column :label="t('docker.common.actions')" width="200" fixed="right">
        <template #default="{ row }">
          <!-- 只有容器有 exec / logs / inspect 接口；镜像、卷、网络没有对应动作 -->
          <template v-if="row.type === 'container'">
            <el-button link type="primary" @click="openExec(row)">{{ t('docker.exec.title') }}</el-button>
            <el-button link type="primary" @click="openLogs(row)">{{ t('docker.container.logs') }}</el-button>
            <el-button link type="primary" @click="openInspect(row)">
              {{ t('docker.container.inspect') }}
            </el-button>
          </template>
          <span v-else>—</span>
        </template>
      </el-table-column>
      <template #empty>
        <span class="events__empty">{{ t('docker.events.empty') }}</span>
      </template>
    </el-table>

    <!-- 直接复用容器面板的抽屉：事件里的对象可能已被删除，接口报错由抽屉自己提示 -->
    <LogsDrawer v-model="logsVisible" :container-id="activeId" :container-name="activeName" />
    <InspectDrawer v-model="inspectVisible" :container-id="activeId" :container-name="activeName" />
    <ExecTerminal v-model="execVisible" :container-id="activeId" :container-name="activeName" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Delete } from '@/icons'
import LogsDrawer from '../components/LogsDrawer.vue'
import InspectDrawer from '../components/InspectDrawer.vue'
import ExecTerminal from '../components/ExecTerminal.vue'
import { listContainers } from '@/api/docker'
import { getToken } from '@/utils/auth'
import { wsUrl } from '@/utils/base'

defineProps<{ refreshToken?: number }>()
const emit = defineEmits<{ count: [number] }>()

const { t } = useI18n()

/** 守护事件的对象类型（与 Engine API 的 `Type` 一致） */
const EVENT_TYPES = ['container', 'image', 'volume', 'network', 'daemon', 'plugin']

/** 只保留最近若干条：事件流可能很吵，不设上限内存会一直涨 */
const MAX = 500

type Status = 'idle' | 'connecting' | 'connected' | 'closed' | 'error'

interface DockerEvent {
  type: string
  action: string
  id: string
  name: string
  image: string
  time: number
  attrs?: Record<string, string>
}

const status = ref<Status>('idle')
/** 是否保持连接：暂停即断开（事件流没有"暂停但保持连接"的必要，断开最省资源） */
const live = ref(true)
const items = ref<DockerEvent[]>([])
const typeFilter = ref('')
const keyword = ref('')

// 日志 / 详情抽屉：事件里的对象可能已被删除（destroy / remove），这种时候接口会报错，由抽屉提示
const logsVisible = ref(false)
const inspectVisible = ref(false)
const execVisible = ref(false)
const activeId = ref('')
const activeName = ref('')

let ws: WebSocket | null = null

const statusType = computed(() => {
  switch (status.value) {
    case 'connected':
      return 'success'
    case 'connecting':
      return 'warning'
    case 'error':
      return 'danger'
    default:
      return 'info'
  }
})

const statusText = computed(() => {
  switch (status.value) {
    case 'connecting':
      return t('docker.events.connecting')
    case 'connected':
      return t('docker.events.connected')
    case 'error':
      return t('docker.events.closed')
    default:
      return t('docker.events.closed')
  }
})

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return items.value.filter((e) => {
    if (typeFilter.value && e.type !== typeFilter.value) return false
    if (!kw) return true
    return (
      e.action.toLowerCase().includes(kw) ||
      e.name.toLowerCase().includes(kw) ||
      e.image.toLowerCase().includes(kw) ||
      e.id.toLowerCase().includes(kw)
    )
  })
})

function connect() {
  disconnect()
  status.value = 'connecting'

  const sock = new WebSocket(wsUrl(`/api/docker/events/ws?token=${getToken()}`))
  ws = sock

  sock.onmessage = (ev) => {
    if (ws !== sock || typeof ev.data !== 'string') return
    let msg: { kind?: string; data?: DockerEvent; message?: string } = {}
    try {
      msg = JSON.parse(ev.data)
    } catch {
      return
    }
    if (msg.kind === 'ready') {
      status.value = 'connected'
    } else if (msg.kind === 'event' && msg.data) {
      push(msg.data)
    } else if (msg.kind === 'error') {
      status.value = 'error'
      live.value = false
      ElMessage.error(t('docker.events.error', { message: msg.message ?? '' }))
    } else if (msg.kind === 'end') {
      status.value = 'closed'
    }
  }

  sock.onerror = () => {
    if (ws !== sock) return
    status.value = 'error'
  }

  sock.onclose = () => {
    if (ws !== sock) return
    ws = null
    if (status.value === 'connecting' || status.value === 'connected') {
      status.value = 'closed'
    }
  }
}

function disconnect() {
  const sock = ws
  ws = null
  if (!sock) return
  try {
    if (sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify({ type: 'close' }))
  } catch {
    /* 已断开，忽略 */
  }
  sock.close()
}

function push(ev: DockerEvent) {
  // 最新在上：表格不需要自动滚动，新事件一眼可见
  items.value.unshift(ev)
  if (items.value.length > MAX) items.value.length = MAX
  emit('count', items.value.length)
}

/** actor 名字可能带前导 `/`（同 `docker ps` 的输出），展示前去掉 */
function eventName(ev: DockerEvent): string {
  return ev.name?.replace(/^\//, '') || ev.id.slice(0, 12)
}

/**
 * 事件里没有"是否运行中"这个字段，只能现查一次列表来判断。
 * 查不到（容器刚被删）或请求失败时返回空串，由调用方兜底放行 —— 宁可让终端自己报错，也别误拦。
 */
async function lookupState(id: string): Promise<string> {
  try {
    const resp = await listContainers(true)
    const hit = (resp.data.items ?? []).find((c) => c.ID.startsWith(id) || id.startsWith(c.ID))
    return hit?.State ?? ''
  } catch {
    return ''
  }
}

/** exec 只对运行中的容器有意义：停止的容器 attach 上去只会立刻报错 */
async function openExec(ev: DockerEvent) {
  const state = await lookupState(ev.id)
  if (state && state !== 'running') {
    ElMessage.warning(t('docker.exec.needRunning'))
    return
  }
  activeId.value = ev.id
  activeName.value = eventName(ev)
  execVisible.value = true
}

function openLogs(ev: DockerEvent) {
  activeId.value = ev.id
  activeName.value = eventName(ev)
  logsVisible.value = true
}

function openInspect(ev: DockerEvent) {
  activeId.value = ev.id
  activeName.value = eventName(ev)
  inspectVisible.value = true
}

function clear() {
  items.value = []
  emit('count', 0)
}

function toggle() {
  live.value = !live.value
  if (live.value) connect()
  else disconnect()
}

function fmtTime(sec: number): string {
  if (!sec) return '-'
  return new Date(sec * 1000).toLocaleString()
}

function tagType(type: string): 'success' | 'warning' | 'danger' | 'info' | 'primary' {
  switch (type) {
    case 'container':
      return 'primary'
    case 'image':
      return 'success'
    case 'volume':
      return 'warning'
    case 'network':
      return 'info'
    default:
      return 'info'
  }
}

onMounted(connect)
onBeforeUnmount(disconnect)
</script>

<style scoped>
.events {
  display: flex;
  flex-direction: column;
  gap: 10px;
  /* 与表格 height="100%" 配合：面板高度随视口，避免事件堆满整页 */
  height: calc(100vh - 300px);
  min-height: 360px;
}

.events__toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.events__spacer {
  flex: 1;
}

.events__hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.events__mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}

.events__empty {
  font-size: 13px;
  color: var(--el-text-color-secondary);
}
</style>
