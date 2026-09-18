<template>
  <div class="docker-pane">
    <div class="pane-toolbar">
      <div class="pane-toolbar__left">
        <el-input
          v-model="keyword"
          :prefix-icon="Search"
          :placeholder="t('docker.common.search')"
          clearable
          :style="{ width: '240px' }"
        />
        <el-select v-model="stateFilter" :style="{ width: '150px' }">
          <el-option :label="t('docker.container.filterAll')" value="all" />
          <el-option :label="t('docker.container.filterRunning')" value="running" />
          <el-option :label="t('docker.container.filterStopped')" value="stopped" />
        </el-select>
        <el-select
          v-model="projectFilter"
          clearable
          :placeholder="t('docker.container.project')"
          :style="{ width: '170px' }"
        >
          <el-option v-for="p in projects" :key="p" :label="p" :value="p" />
        </el-select>
        <template v-if="selection.length">
          <el-divider direction="vertical" />
          <span class="pane-selected">{{ t('docker.common.selected', { n: selection.length }) }}</span>
          <el-button size="small" @click="bulk('start', false)">{{ t('docker.container.start') }}</el-button>
          <el-button size="small" @click="bulk('stop', true)">{{ t('docker.container.stop') }}</el-button>
          <el-button size="small" @click="bulk('restart', true)">{{ t('docker.container.restart') }}</el-button>
          <el-button size="small" type="danger" plain @click="bulk('remove', true)">
            {{ t('docker.common.remove') }}
          </el-button>
        </template>
      </div>
      <div class="pane-toolbar__right">
        <el-button size="small" :icon="Refresh" @click="emit('refresh')">{{ t('docker.refresh') }}</el-button>
      </div>
    </div>

    <el-table
      v-loading="loading"
      :data="filtered"
      row-key="ID"
      size="small"
      @selection-change="(rows: DockerContainer[]) => (selection = rows)"
    >
      <el-table-column type="selection" width="42" />

      <el-table-column :label="t('docker.container.name')" min-width="210">
        <template #default="{ row }">
          <div class="cell-primary">
            <span class="cell-name" :title="displayName(row)">{{ displayName(row) }}</span>
            <el-tag v-if="row.project" size="small" effect="plain" round>{{ row.project }}</el-tag>
          </div>
          <div class="cell-sub mono">{{ shortId(row.ID) }}</div>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.status')" width="130">
        <template #default="{ row }">
          <el-tooltip :content="row.Status" placement="top" :show-after="400">
            <el-tag size="small" :type="stateType(row.State)" effect="dark">
              {{ stateText(row.State) }}
            </el-tag>
          </el-tooltip>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.container.image')" min-width="180" show-overflow-tooltip>
        <template #default="{ row }">
          <span class="mono">{{ row.Image }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.container.ports')" min-width="170" show-overflow-tooltip>
        <template #default="{ row }">
          <span class="mono">{{ row.Ports || '—' }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.container.cpu')" width="90">
        <template #default="{ row }">{{ cpu(row) }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.container.memory')" width="150">
        <template #default="{ row }">{{ mem(row) }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.common.created')" width="170">
        <template #default="{ row }">{{ row.CreatedAt || '—' }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.common.actions')" width="240" fixed="right">
        <template #default="{ row }">
          <el-button v-if="row.State !== 'running'" link type="success" @click="single(row, 'start', false)">
            {{ t('docker.container.start') }}
          </el-button>
          <el-button v-else link type="warning" @click="single(row, 'stop', true)">
            {{ t('docker.container.stop') }}
          </el-button>
          <el-button link type="primary" @click="openExec(row)">{{ t('docker.exec.title') }}</el-button>
          <el-button link type="primary" @click="openLogs(row)">{{ t('docker.container.logs') }}</el-button>
          <el-button link type="primary" @click="openInspect(row)">{{ t('docker.container.inspect') }}</el-button>
          <el-dropdown trigger="click" @command="(cmd: string) => more(row, cmd)">
            <el-button link type="primary">
              {{ t('docker.common.actions') }}<el-icon class="el-icon--right"><ArrowDown /></el-icon>
            </el-button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item command="restart">{{ t('docker.container.restart') }}</el-dropdown-item>
                <el-dropdown-item v-if="row.State !== 'paused'" command="pause">
                  {{ t('docker.container.pause') }}
                </el-dropdown-item>
                <el-dropdown-item v-else command="unpause">{{ t('docker.container.unpause') }}</el-dropdown-item>
                <el-dropdown-item command="kill" divided>{{ t('docker.container.kill') }}</el-dropdown-item>
                <el-dropdown-item command="remove">
                  <span class="danger-text">{{ t('docker.common.remove') }}</span>
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </template>
      </el-table-column>

      <template #empty>
        <el-empty :description="t('docker.common.empty')" :image-size="60" />
      </template>
    </el-table>

    <LogsDrawer v-model="logsVisible" :container-id="activeId" :container-name="activeName" />
    <InspectDrawer v-model="inspectVisible" :container-id="activeId" :container-name="activeName" />
    <ExecTerminal v-model="execVisible" :container-id="activeId" :container-name="activeName" />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { ArrowDown, Refresh, Search } from '@/icons'
import LogsDrawer from '../components/LogsDrawer.vue'
import InspectDrawer from '../components/InspectDrawer.vue'
import ExecTerminal from '../components/ExecTerminal.vue'
import {
  containerAction,
  containerStats,
  listContainers,
  type DockerContainer,
  type DockerStat,
} from '@/api/docker'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()

const loading = ref(false)
const rows = ref<DockerContainer[]>([])
const stats = ref<Record<string, DockerStat>>({})
const selection = ref<DockerContainer[]>([])
const keyword = ref('')
const stateFilter = ref('all')
const projectFilter = ref('')

const logsVisible = ref(false)
const inspectVisible = ref(false)
const execVisible = ref(false)
const activeId = ref('')
const activeName = ref('')

/** compose 项目下拉：直接从容器自带的 project 字段取，无需再请求。 */
const projects = computed(() =>
  Array.from(new Set(rows.value.map((r) => r.project).filter((p): p is string => !!p))).sort(),
)

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return rows.value.filter((r) => {
    if (stateFilter.value === 'running' && r.State !== 'running') return false
    if (stateFilter.value === 'stopped' && r.State === 'running') return false
    if (projectFilter.value && r.project !== projectFilter.value) return false
    if (!kw) return true
    return `${r.ID} ${r.Names} ${r.Image} ${r.project}`.toLowerCase().includes(kw)
  })
})

function displayName(row: DockerContainer): string {
  return row.Names?.split(',')[0]?.trim() || row.ID
}

function shortId(id: string): string {
  return id?.slice(0, 12) || ''
}

function stateType(state: string) {
  switch (state) {
    case 'running':
      return 'success'
    case 'paused':
    case 'restarting':
      return 'warning'
    case 'dead':
      return 'danger'
    case 'exited':
    case 'created':
      return 'info'
    default:
      return 'info'
  }
}

function stateText(state: string): string {
  const map: Record<string, string> = {
    running: t('docker.common.running'),
    exited: t('docker.common.exited'),
    paused: t('docker.common.pausedState'),
    created: t('docker.common.createdState'),
    restarting: t('docker.common.restarting'),
    dead: t('docker.common.dead'),
  }
  return map[state] || state || t('docker.common.unknownState')
}

/** stats 按容器名索引（`docker stats` 输出的是容器名，非 ID）。 */
function cpu(row: DockerContainer): string {
  return stats.value[row.Names]?.CPUPerc || '—'
}

function mem(row: DockerContainer): string {
  return stats.value[row.Names]?.MemUsage || '—'
}

/** 动作成功提示（放在函数里取 t，切换语言后文案同步更新）。 */
function successText(action: string): string {
  const map: Record<string, string> = {
    start: t('docker.common.started'),
    stop: t('docker.common.stopped'),
    restart: t('docker.common.restarted'),
    pause: t('docker.common.paused'),
    unpause: t('docker.common.unpaused'),
    kill: t('docker.common.killed'),
    remove: t('docker.common.deleted'),
  }
  return map[action] || t('docker.common.deleted')
}

function confirmText(action: string, name: string, n: number): string {
  if (action === 'remove') {
    return n > 1 ? t('docker.common.bulkRemoveConfirm', { n }) : t('docker.common.removeConfirm', { name })
  }
  if (action === 'stop') {
    return n > 1 ? t('docker.container.bulkStopConfirm', { n }) : t('docker.container.stopConfirm', { name })
  }
  if (action === 'kill') {
    return n > 1 ? t('docker.container.bulkKillConfirm', { n }) : t('docker.container.killConfirm', { name })
  }
  return ''
}

async function runAction(ids: string[], action: string, confirmIt: boolean, n: number, name: string) {
  if (confirmIt) {
    const msg = confirmText(action, name, n)
    if (msg) {
      try {
        await ElMessageBox.confirm(msg, t('docker.common.remove'), { type: 'warning' })
      } catch {
        return
      }
    }
  }
  try {
    const resp = await containerAction(ids, action)
    const failed = resp.data.failed ?? []
    if (failed.length) ElMessage.warning(t('docker.common.partialFailed', { n: failed.length }))
    else ElMessage.success(successText(action))
    selection.value = []
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

function single(row: DockerContainer, action: string, confirmIt: boolean) {
  runAction([row.ID], action, confirmIt, 1, displayName(row))
}

function more(row: DockerContainer, action: string) {
  single(row, action, action !== 'restart' && action !== 'pause' && action !== 'unpause')
}

function bulk(action: string, confirmIt: boolean) {
  if (!selection.value.length) {
    ElMessage.warning(t('docker.common.noSelection'))
    return
  }
  runAction(
    selection.value.map((r) => r.ID),
    action,
    confirmIt,
    selection.value.length,
    '',
  )
}

function openLogs(row: DockerContainer) {
  activeId.value = row.ID
  activeName.value = displayName(row)
  logsVisible.value = true
}

/** exec 需要容器处于运行状态：停止的容器 attach 上去只会立刻报错 */
function openExec(row: DockerContainer) {
  if (row.State !== 'running') {
    ElMessage.warning(t('docker.exec.needRunning'))
    return
  }
  activeId.value = row.ID
  activeName.value = displayName(row)
  execVisible.value = true
}

function openInspect(row: DockerContainer) {
  activeId.value = row.ID
  activeName.value = displayName(row)
  inspectVisible.value = true
}

async function load() {
  loading.value = true
  try {
    const resp = await listContainers(true)
    rows.value = resp.data.items ?? []
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.loadFailed'))
  } finally {
    loading.value = false
  }

  // 资源占用是增强列，失败不影响主列表（例如 daemon 不支持 stats）
  try {
    const s = await containerStats()
    const map: Record<string, DockerStat> = {}
    for (const item of s.data.items ?? []) {
      map[item.Name] = item
    }
    stats.value = map
  } catch {
    /* 忽略：统计列显示 — 即可 */
  }
}

watch(() => props.refreshToken, load)
watch(() => rows.value.length, (n) => emit('count', n), { immediate: true })
onMounted(load)
</script>

<style scoped>
.pane-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.pane-toolbar__left,
.pane-toolbar__right {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.pane-selected {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.cell-primary {
  display: flex;
  align-items: center;
  gap: 6px;
}

.cell-name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cell-sub {
  font-size: 11px;
  color: var(--el-text-color-secondary);
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}

.danger-text {
  color: var(--el-color-danger);
}
</style>
