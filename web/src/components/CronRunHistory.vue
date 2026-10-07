<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-drawer v-model="visible" :title="title" size="62%">
    <div class="hist-wrap">
      <div class="hist-toolbar">
        <span class="hist-tip">
          <el-icon><InfoFilled /></el-icon>
          {{ t('cronHistory.keepTip', { n: keep }) }}
        </span>
        <div class="toolbar-spacer" />
        <el-button size="small" plain :loading="loading" @click="load">
          {{ t('cronHistory.refresh') }}
        </el-button>
        <el-button
          size="small"
          type="danger"
          plain
          :disabled="!runs.length"
          :loading="clearing"
          @click="handleClear"
        >
          {{ t('cronHistory.clear') }}
        </el-button>
      </div>

      <el-table :data="runs" v-loading="loading" size="small" style="width: 100%">
        <el-table-column :label="t('cronHistory.colTime')" min-width="150">
          <template #default="{ row }">{{ fmt(row.started_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('cronHistory.colStatus')" width="96">
          <template #default="{ row }">
            <el-tag size="small" :type="statusType(row)" effect="dark">
              {{ statusText(row) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('cronHistory.colExitCode')" width="90">
          <template #default="{ row }">
            {{ row.exit_code >= 0 ? row.exit_code : '—' }}
          </template>
        </el-table-column>
        <el-table-column :label="t('cronHistory.colDuration')" width="110">
          <template #default="{ row }">{{ duration(row) }}</template>
        </el-table-column>
        <el-table-column :label="t('cronHistory.colTrigger')" width="96">
          <template #default="{ row }">
            {{
              row.action === 'manual'
                ? t('cronHistory.triggerManual')
                : t('cronHistory.triggerCron')
            }}
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="110" fixed="right">
          <template #default="{ row }">
            <el-button link type="primary" @click="emit('view', row)">
              {{ t('cronHistory.viewLog') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('cronHistory.empty')" :image-size="60" />
        </template>
      </el-table>
    </div>
  </el-drawer>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { InfoFilled } from '@/icons'
import dayjs from 'dayjs'
import { listCronRuns, clearCronRuns, type CronRunItem } from '@/api/cron'
import {
  listCrontabRuns,
  clearCrontabRuns,
  type CrontabRunItem,
} from '@/api/crontab'

const props = defineProps<{
  /** system = 计划任务（/system/cron/*）；crontab = 用户计划任务（/terminal/crontab/*） */
  variant: 'system' | 'crontab'
}>()

const { t } = useI18n()

const visible = ref(false)
const loading = ref(false)
const clearing = ref(false)
const runs = ref<CronRunItem[]>([])
const keep = ref(50)
/** 任务 id：system 是数字、crontab 是字符串，统一按字符串存 */
const jobId = ref('')
const jobName = ref('')

const title = computed(() => t('cronHistory.title', { name: jobName.value }))

const emit = defineEmits<{ view: [run: CronRunItem | CrontabRunItem]; cleared: [] }>()

function open(job: { id: string; name: string }) {
  jobId.value = job.id
  jobName.value = job.name
  visible.value = true
  load()
}

defineExpose({ open })

async function load() {
  loading.value = true
  try {
    const resp =
      props.variant === 'crontab'
        ? await listCrontabRuns(jobId.value)
        : await listCronRuns(jobId.value)
    runs.value = resp.data?.runs || []
    keep.value = resp.data?.keep || 50
  } catch (e: any) {
    ElMessage.error(e.message || t('cronHistory.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function handleClear() {
  try {
    await ElMessageBox.confirm(t('cronHistory.clearConfirm'), t('cronHistory.clearTitle'), {
      type: 'warning',
    })
  } catch {
    return
  }
  clearing.value = true
  try {
    const resp =
      props.variant === 'crontab'
        ? await clearCrontabRuns(jobId.value)
        : await clearCronRuns(jobId.value)
    ElMessage.success(t('cronHistory.cleared', { n: resp.data?.deleted ?? 0 }))
    runs.value = []
    emit('cleared')
  } catch (e: any) {
    ElMessage.error(e.message || t('cronHistory.clearFailed'))
  } finally {
    clearing.value = false
  }
}

function fmt(ts: number) {
  return ts > 0 ? dayjs(ts * 1000).format('YYYY-MM-DD HH:mm:ss') : '—'
}

function duration(row: CronRunItem) {
  if (!row.finished_at || row.finished_at < row.started_at) return '—'
  const s = row.finished_at - row.started_at
  if (s < 60) return t('cronHistory.seconds', { n: s })
  return t('cronHistory.minutes', { n: Math.floor(s / 60), s: s % 60 })
}

function statusType(row: CronRunItem): 'success' | 'danger' | 'warning' | 'info' {
  if (row.status === 'success') return 'success'
  if (row.status === 'failed') return 'danger'
  return row.status === 'running' ? 'warning' : 'info'
}

function statusText(row: CronRunItem) {
  if (row.status === 'success') return t('cronHistory.statusSuccess')
  if (row.status === 'failed') return t('cronHistory.statusFailed')
  if (row.status === 'running') return t('cronHistory.statusRunning')
  return row.status
}
</script>

<style scoped>
.hist-wrap {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.hist-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-bottom: 10px;
  margin-bottom: 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.hist-tip {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.toolbar-spacer {
  flex: 1;
}
</style>
