<template>
  <div class="tasks-panel">
    <div class="tasks-toolbar">
      <el-select
        v-model="status"
        size="small"
        clearable
        :placeholder="t('task.filterStatus')"
        style="width: 140px"
        @change="load"
      >
        <el-option
          v-for="s in statusOptions"
          :key="s.value"
          :label="s.label"
          :value="s.value"
        />
      </el-select>
      <el-input
        v-model="keyword"
        size="small"
        clearable
        :placeholder="t('task.keyword')"
        style="width: 200px"
        @keyup.enter="load"
        @clear="load"
      />
      <el-button size="small" :icon="Refresh" @click="load">{{ t('task.refresh') }}</el-button>
      <div class="tasks-toolbar__spacer" />
      <el-text v-if="total" size="small" type="info">
        {{ t('task.total', { n: total }) }}
      </el-text>
    </div>

    <el-table v-loading="loading" :data="rows" size="small" stripe>
      <el-table-column :label="t('task.columns.task')" min-width="220" show-overflow-tooltip>
        <template #default="{ row }">
          <span>{{ row.title || row.pkg || row.action }}</span>
          <el-text v-if="row.status === 'pending'" size="small" type="warning" class="tasks-badge">
            {{ t('task.queuePosition', { n: queueIndex(row) }) }}
          </el-text>
        </template>
      </el-table-column>
      <el-table-column :label="t('task.columns.status')" width="110">
        <template #default="{ row }">
          <el-tag size="small" effect="dark" :type="statusType(row.status)">
            {{ statusText(row.status) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column
        v-if="isAdmin"
        :label="t('task.columns.user')"
        width="120"
        prop="username"
      />
      <el-table-column :label="t('task.columns.started')" width="170">
        <template #default="{ row }">{{ fmtTime(row.started_at) }}</template>
      </el-table-column>
      <el-table-column :label="t('task.columns.ops')" width="150" fixed="right">
        <template #default="{ row }">
          <el-button link type="primary" size="small" @click="openLog(row)">
            {{ t('task.ops.log') }}
          </el-button>
          <el-button
            link
            type="danger"
            size="small"
            :disabled="!isActive(row.status)"
            @click="handleCancel(row)"
          >
            {{ t('task.ops.cancel') }}
          </el-button>
        </template>
      </el-table-column>
      <template #empty>
        <el-empty :description="t('task.empty')" :image-size="60" />
      </template>
    </el-table>

    <div class="tasks-pager">
      <el-pagination
        v-model:current-page="page"
        :page-size="pageSize"
        :total="total"
        layout="prev, pager, next"
        small
        background
        @current-change="load"
      />
    </div>

    <!-- 日志：队列里的任务未必有可停止句柄，用 simple 模式只展示日志 -->
    <AppStoreLogDrawer ref="drawerRef" simple />
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useUserStore } from '@/stores/user'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import { cancelTask, getTasks, type TaskItem, type TaskStatus } from '@/api/task'

const props = withDefaults(
  defineProps<{
    /** 只看某一类任务（appstore / docker ...）；留空 = 全部 */
    kind?: string
    refreshToken?: number
  }>(),
  { kind: '', refreshToken: 0 },
)
const emit = defineEmits<{ (e: 'count', n: number): void }>()

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = !!userStore.roles?.includes('admin')

const drawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)

const rows = ref<TaskItem[]>([])
const loading = ref(false)
const page = ref(1)
const pageSize = 20
const total = ref(0)
const status = ref('')
const keyword = ref('')

const statusOptions = [
  { value: 'pending', label: t('task.status.pending') },
  { value: 'running', label: t('task.status.running') },
  { value: 'success', label: t('task.status.success') },
  { value: 'failed', label: t('task.status.failed') },
  { value: 'canceled', label: t('task.status.canceled') },
]

function statusText(s: string) {
  return t(`task.status.${s}` as never, s)
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

/** 排队位次：接口按 id 倒序返回，pending 的倒着数就是「第几位」 */
function queueIndex(row: TaskItem) {
  if (row.status !== 'pending') return 0
  const queued = rows.value.filter((r) => r.status === 'pending')
  return queued.length - queued.findIndex((r) => r.task_id === row.task_id)
}

function fmtTime(ts: number) {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

async function load() {
  loading.value = true
  try {
    const res = await getTasks({
      kind: props.kind || undefined,
      page: page.value,
      page_size: pageSize,
      status: status.value || undefined,
      keyword: keyword.value.trim() || undefined,
    })
    const data = res.data ?? {}
    rows.value = data.items ?? []
    total.value = data.total ?? 0
    emit('count', total.value)
  } catch (e: any) {
    ElMessage.error(e?.message || t('task.loadFailed'))
  } finally {
    loading.value = false
  }
}

function openLog(row: TaskItem) {
  drawerRef.value?.openDrawer(row.task_id, row.title || t('task.logTitle'))
}

async function handleCancel(row: TaskItem) {
  try {
    await ElMessageBox.confirm(t('task.confirmCancel'), t('task.ops.cancel'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await cancelTask(row.task_id)
    ElMessage.success(t('task.cancelRequested'))
    load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('task.opFailed'))
  }
}

/** 队列里有活儿时才自动刷新：静止的列表不必每 5 秒打一次接口 */
let timer: ReturnType<typeof setInterval> | undefined
function syncTimer() {
  const busy = rows.value.some((r) => isActive(r.status))
  if (busy && !timer) timer = setInterval(load, 5000)
  else if (!busy && timer) {
    clearInterval(timer)
    timer = undefined
  }
}

watch(rows, syncTimer)
watch(() => props.refreshToken, load)
onMounted(load)
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})

defineExpose({ load })
</script>

<style scoped>
.tasks-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.tasks-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.tasks-toolbar__spacer {
  flex: 1;
}

.tasks-badge {
  margin-left: 8px;
}

.tasks-pager {
  display: flex;
  justify-content: flex-end;
}
</style>
