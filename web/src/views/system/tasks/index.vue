<template>
  <div class="task-page">
    <header class="task-header">
      <div class="task-header__title">
        <el-icon :size="18"><List /></el-icon>
        <h3>{{ t('task.title') }}</h3>
        <el-text size="small" type="info">{{ t('task.subtitle') }}</el-text>
      </div>
      <div class="task-header__actions">
        <el-checkbox v-model="autoRefresh" size="small">{{ t('task.autoRefresh') }}</el-checkbox>
        <el-button size="small" :icon="Refresh" @click="loadAll">{{ t('task.refresh') }}</el-button>
      </div>
    </header>

    <!-- 状态统计：一眼看出还有多少在跑 / 卡住 -->
    <div class="task-stats">
      <div v-for="s in statCards" :key="s.key" class="task-stat" @click="pickStatus(s.key)">
        <span class="task-stat__value">{{ stats[s.key] ?? 0 }}</span>
        <span class="task-stat__label">{{ s.label }}</span>
      </div>
    </div>

    <div class="task-toolbar">
      <el-select
        v-model="filters.kind"
        size="small"
        clearable
        :placeholder="t('task.filterKind')"
        style="width: 150px"
        @change="reload"
      >
        <el-option v-for="k in kindOptions" :key="k.value" :label="k.label" :value="k.value" />
      </el-select>
      <el-select
        v-model="filters.status"
        size="small"
        clearable
        :placeholder="t('task.filterStatus')"
        style="width: 130px"
        @change="reload"
      >
        <el-option v-for="s in statusOptions" :key="s.value" :label="s.label" :value="s.value" />
      </el-select>
      <el-input
        v-model="filters.username"
        size="small"
        clearable
        :placeholder="t('task.filterUser')"
        style="width: 140px"
        @keyup.enter="reload"
        @clear="reload"
      />
      <el-input
        v-model="filters.keyword"
        size="small"
        clearable
        :placeholder="t('task.keyword')"
        style="width: 200px"
        @keyup.enter="reload"
        @clear="reload"
      />
      <el-button size="small" :icon="Search" type="primary" @click="reload">
        {{ t('task.search') }}
      </el-button>
      <el-button size="small" @click="resetFilters">{{ t('task.reset') }}</el-button>
    </div>

    <el-table v-loading="loading" :data="rows" size="small" stripe>
      <el-table-column :label="t('task.columns.task')" min-width="200" show-overflow-tooltip>
        <template #default="{ row }">
          <span>{{ row.title || row.pkg || row.action }}</span>
          <el-text v-if="row.control" size="small" type="warning" class="task-control">
            {{ row.control === 'pause' ? t('task.paused') : t('task.cancelRequested') }}
          </el-text>
        </template>
      </el-table-column>
      <el-table-column :label="t('task.columns.kind')" width="120">
        <template #default="{ row }">{{ kindText(row.kind) }}</template>
      </el-table-column>
      <el-table-column :label="t('task.columns.action')" width="150" prop="action" />
      <el-table-column :label="t('task.columns.user')" width="120" prop="username" />
      <el-table-column :label="t('task.columns.status')" width="110">
        <template #default="{ row }">
          <el-tag size="small" effect="dark" :type="statusType(row.status)">
            {{ statusText(row.status) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('task.columns.started')" width="170">
        <template #default="{ row }">{{ fmtTime(row.started_at) }}</template>
      </el-table-column>
      <el-table-column :label="t('task.columns.duration')" width="100">
        <template #default="{ row }">{{ fmtDuration(row) }}</template>
      </el-table-column>
      <el-table-column :label="t('task.columns.ops')" width="290" fixed="right">
        <template #default="{ row }">
          <el-button link type="primary" size="small" @click="openLog(row)">
            {{ t('task.ops.log') }}
          </el-button>
          <!-- 应用商店任务失败后：改快照脚本再重跑（管理员） -->
          <template v-if="row.kind === 'appstore' && canOperate(row)">
            <el-button link type="warning" size="small" @click="openEditor(row)">
              {{ t('task.ops.edit') }}
            </el-button>
            <el-button
              link
              type="danger"
              size="small"
              :loading="retryId === row.task_id"
              @click="retry(row)"
            >
              {{ t('task.ops.retry') }}
            </el-button>
          </template>
          <el-button
            link
            type="danger"
            size="small"
            :disabled="!isActive(row.status)"
            @click="handleCancel(row)"
          >
            {{ t('task.ops.cancel') }}
          </el-button>
          <el-button
            v-if="row.status === 'running'"
            link
            type="warning"
            size="small"
            @click="handlePause(row)"
          >
            {{ row.control === 'pause' ? t('task.ops.resume') : t('task.ops.pause') }}
          </el-button>
        </template>
      </el-table-column>
      <template #empty>
        <el-empty :description="t('task.empty')" :image-size="60" />
      </template>
    </el-table>

    <div class="task-pager">
      <el-pagination
        v-model:current-page="page"
        :page-size="pageSize"
        :total="total"
        layout="total, prev, pager, next"
        small
        background
        @current-change="load"
      />
    </div>

    <AppStoreLogDrawer ref="drawerRef" :simple="drawerSimple" @retried="loadAll" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { List, Refresh, Search } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import { useAppstoreRunOps } from '@/composables/useAppstoreRunOps'
import {
  cancelTask,
  getTasks,
  getTaskStats,
  pauseTask,
  resumeTask,
  type TaskItem,
  type TaskStats,
  type TaskStatus,
} from '@/api/task'

const { t } = useI18n()
const drawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
/** 抽屉的默认模式：只有应用商店任务带可编辑快照与重跑，其余只展示日志 */
const drawerSimple = ref(true)

const { retryId, canOperate, openEditor, retry } = useAppstoreRunOps(drawerRef, loadAll)

const rows = ref<TaskItem[]>([])
const loading = ref(false)
const page = ref(1)
const pageSize = 20
const total = ref(0)
const autoRefresh = ref(false)
const stats = reactive<TaskStats>({
  pending: 0,
  running: 0,
  success: 0,
  failed: 0,
  canceled: 0,
  total: 0,
})

const filters = reactive<{ kind: string; status: string; username: string; keyword: string }>({
  kind: '',
  status: '',
  username: '',
  keyword: '',
})

const kindOptions = [
  { value: 'appstore', label: t('task.kind.appstore') },
  { value: 'docker', label: t('task.kind.docker') },
  { value: 'backup', label: t('task.kind.backup') },
  { value: 'system', label: t('task.kind.system') },
  { value: 'cron', label: t('task.kind.cron') },
  { value: 'crontab', label: t('task.kind.crontab') },
  { value: 'site', label: t('task.kind.site') },
]

const statusOptions = [
  { value: 'pending', label: t('task.status.pending') },
  { value: 'running', label: t('task.status.running') },
  { value: 'success', label: t('task.status.success') },
  { value: 'failed', label: t('task.status.failed') },
  { value: 'canceled', label: t('task.status.canceled') },
]

const statCards = computed<Array<{ key: keyof TaskStats; label: string }>>(() => [
  { key: 'running', label: t('task.status.running') },
  { key: 'pending', label: t('task.status.pending') },
  { key: 'success', label: t('task.status.success') },
  { key: 'failed', label: t('task.status.failed') },
  { key: 'canceled', label: t('task.status.canceled') },
])

function statusText(s: string) {
  return t(`task.status.${s}` as never, s)
}

function kindText(k: string) {
  return t(`task.kind.${k}` as never, k)
}

function statusType(s: string): 'primary' | 'success' | 'danger' | 'info' | 'warning' {
  if (s === 'running') return 'primary'
  if (s === 'success') return 'success'
  if (s === 'failed') return 'danger'
  if (s === 'canceled') return 'info'
  return 'warning'
}

function isActive(s: TaskStatus) {
  return s === 'running' || s === 'pending'
}

function fmtTime(ts: number) {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

function fmtDuration(row: TaskItem) {
  if (!row.started_at) return '-'
  const end = row.finished_at || Math.floor(Date.now() / 1000)
  const sec = Math.max(0, end - row.started_at)
  if (sec < 60) return `${sec}s`
  if (sec < 3600) return `${Math.floor(sec / 60)}m${sec % 60}s`
  return `${Math.floor(sec / 3600)}h${Math.floor((sec % 3600) / 60)}m`
}

async function load() {
  loading.value = true
  try {
    const res = await getTasks({
      page: page.value,
      page_size: pageSize,
      kind: filters.kind || undefined,
      status: filters.status || undefined,
      username: filters.username.trim() || undefined,
      keyword: filters.keyword.trim() || undefined,
    })
    const data = res.data ?? {}
    rows.value = data.items ?? []
    total.value = data.total ?? 0
  } catch (e: any) {
    ElMessage.error(e?.message || t('task.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function loadStats() {
  try {
    const res = await getTaskStats()
    Object.assign(stats, res.data ?? {})
  } catch {
    // 统计只是顶部概览，失败不打扰列表
  }
}

function loadAll() {
  load()
  loadStats()
}

function reload() {
  page.value = 1
  loadAll()
}

function resetFilters() {
  filters.kind = ''
  filters.status = ''
  filters.username = ''
  filters.keyword = ''
  reload()
}

/** 点统计卡片即按该状态筛选 */
function pickStatus(key: keyof TaskStats) {
  filters.status = filters.status === key ? '' : key
  reload()
}

function openLog(row: TaskItem) {
  drawerSimple.value = row.kind !== 'appstore'
  drawerRef.value?.openDrawer(row.task_id, row.title || t('task.logTitle'))
}

async function handleCancel(row: TaskItem) {
  try {
    await ElMessageBox.confirm(t('task.confirmCancel'), t('task.ops.cancel'), { type: 'warning' })
  } catch {
    return
  }
  try {
    await cancelTask(row.task_id)
    ElMessage.success(t('task.cancelRequested'))
    loadAll()
  } catch (e: any) {
    ElMessage.error(e?.message || t('task.opFailed'))
  }
}

async function handlePause(row: TaskItem) {
  const resume = row.control === 'pause'
  try {
    if (resume) await resumeTask(row.task_id)
    else await pauseTask(row.task_id)
    ElMessage.success(resume ? t('task.resumeRequested') : t('task.pauseRequested'))
    loadAll()
  } catch (e: any) {
    ElMessage.error(e?.message || t('task.opFailed'))
  }
}

let timer: ReturnType<typeof setInterval> | undefined
watch(autoRefresh, (v) => {
  if (timer) clearInterval(timer)
  timer = v ? setInterval(loadAll, 5000) : undefined
})

onMounted(loadAll)
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.task-page {
  padding: 16px;
}

.task-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.task-header__title {
  display: flex;
  align-items: center;
  gap: 8px;
}

.task-header__title h3 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}

.task-header__actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.task-stats {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.task-stat {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 92px;
  padding: 8px 14px;
  border-radius: 8px;
  background: var(--el-fill-color-light);
  cursor: pointer;
  transition: background 0.2s;
}

.task-stat:hover {
  background: var(--el-fill-color);
}

.task-stat__value {
  font-size: 18px;
  font-weight: 600;
}

.task-stat__label {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.task-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.task-control {
  margin-left: 8px;
}

.task-pager {
  display: flex;
  justify-content: flex-end;
  margin-top: 12px;
}
</style>
