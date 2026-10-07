<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="crontab-page">
    <el-card shadow="never" class="base-card">
      <template #header>
        <div class="card-header">
          <span>{{ t('crontab.title') }}</span>
          <div class="header-actions">
            <el-button :icon="Delete" :disabled="readonly" @click="handlePurge">
              {{ t('crontab.purgeOldLogs') }}
            </el-button>
            <el-button type="primary" :icon="Plus" @click="openCreate">{{
              t('crontab.newJob')
            }}</el-button>
          </div>
        </div>
      </template>

      <el-alert type="info" :closable="false" class="cron-alert">
        <template #title>
          {{ t('crontab.alertPrefix')
          }}<strong>{{ canChooseExec ? t('crontab.alertChosen') : t('crontab.alertSelf') }}</strong
          >{{ t('crontab.alertSuffix') }}
          <template v-if="canChooseExec">{{ t('crontab.alertAdmin') }}</template>
          <template v-else>{{ t('crontab.alertUser') }}</template>
        </template>
      </el-alert>

      <el-table :data="jobs" v-loading="loading" style="width: 100%">
        <el-table-column
          prop="name"
          :label="t('crontab.colName')"
          min-width="130"
          show-overflow-tooltip
        />
        <el-table-column :label="t('crontab.colCommand')" min-width="200">
          <template #default="{ row }">
            <el-tag
              size="small"
              :type="row.kind === 'script' ? 'success' : 'info'"
              class="kind-tag"
            >
              {{ row.kind === 'script' ? t('crontab.kindScript') : t('crontab.kindCommand') }}
            </el-tag>
            <span class="mono">{{ row.command }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('crontab.colSchedule')" min-width="140">
          <template #default="{ row }">
            <div class="mono">{{ row.schedule }}</div>
            <div class="cron-desc">{{ describeCron(row.schedule) }}</div>
          </template>
        </el-table-column>
        <el-table-column :label="t('crontab.colExecUser')" width="120">
          <template #default="{ row }">
            <span class="mono">{{ row.exec_user || '—' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('crontab.colStatus')" width="90">
          <template #default="{ row }">
            <el-switch
              :model-value="row.enabled"
              :disabled="switching || readonly"
              @change="(v: boolean) => handleToggle(row, v)"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('crontab.colLastRun')" width="170">
          <template #default="{ row }">
            <span v-if="row.last_run_at > 0" class="link-like" @click="openLog(row)">
              {{ fmt(row.last_run_at) }}
            </span>
            <span v-else>—</span>
            <el-tag v-if="row.last_run_at > 0" size="small" :type="statusType(row.last_status)">
              {{ statusText(row.last_status) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('crontab.colNextRun')" width="150">
          <template #default="{ row }">
            {{ row.enabled && row.next_run_at > 0 ? fmt(row.next_run_at) : '—' }}
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="250" fixed="right">
          <template #default="{ row }">
            <el-button
              link
              type="primary"
              :disabled="readonly || runningId === row.id"
              @click="handleRunNow(row)"
            >
              {{ runningId === row.id ? t('crontab.running') : t('crontab.runNow') }}
            </el-button>
            <el-button link type="primary" @click="openHistory(row)">
              {{ t('cronHistory.history') }}
            </el-button>
            <el-button link type="primary" :disabled="readonly" @click="openEdit(row)">{{
              t('common.edit')
            }}</el-button>
            <el-button link type="danger" :disabled="readonly" @click="handleDelete(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新建 / 编辑 -->
    <el-dialog
      v-model="dialogVisible"
      :title="editing ? t('crontab.editTitle') : t('crontab.newJob')"
      width="620px"
      :close-on-click-modal="false"
    >
      <el-form ref="formRef" :model="form" :rules="rules" label-width="90px" @submit.prevent>
        <el-form-item :label="t('crontab.nameLabel')" prop="name">
          <el-input
            v-model="form.name"
            :placeholder="t('crontab.namePlaceholder')"
            maxlength="60"
          />
        </el-form-item>

        <el-form-item :label="t('crontab.execUserLabel')" prop="exec_user">
          <el-select
            v-if="canChooseExec"
            v-model="form.exec_user"
            filterable
            style="width: 100%"
            :placeholder="t('crontab.execUserPlaceholder')"
          >
            <el-option v-for="u in execUsers" :key="u" :label="u" :value="u" />
          </el-select>
          <el-input v-else :model-value="myExecUser" disabled />
          <div class="field-tip">
            <template v-if="canChooseExec">{{ t('crontab.execAdminTip') }}</template>
            <template v-else>{{ t('crontab.execUserTip') }}</template>
          </div>
        </el-form-item>

        <el-form-item :label="t('crontab.kindLabel')" prop="kind">
          <el-radio-group v-model="form.kind">
            <el-radio-button value="command">{{ t('crontab.kindCommand') }}</el-radio-button>
            <el-radio-button value="script">{{ t('crontab.kindScript') }}</el-radio-button>
          </el-radio-group>
        </el-form-item>

        <el-form-item :label="t('crontab.commandLabel')" prop="command">
          <el-input
            v-model="form.command"
            :type="form.kind === 'command' ? 'textarea' : 'text'"
            :rows="3"
            :placeholder="
              form.kind === 'command'
                ? t('crontab.commandPlaceholder')
                : t('crontab.scriptPlaceholder')
            "
          />
          <div class="field-tip">
            <template v-if="form.kind === 'command'">
              {{ t('crontab.commandTip1') }}<span class="mono">/bin/bash -c</span
              >{{ t('crontab.commandTip2') }}
            </template>
            <template v-else>
              {{ t('crontab.scriptTip1') }}<span class="mono">{{ t('crontab.scriptCode') }}</span
              >{{ t('crontab.scriptTip2') }}
            </template>
          </div>
        </el-form-item>

        <el-form-item :label="t('crontab.presetLabel')">
          <el-select v-model="preset" style="width: 100%" @change="applyPreset">
            <el-option v-for="p in presets" :key="p.value" :label="p.label" :value="p.value" />
          </el-select>
        </el-form-item>

        <el-form-item :label="t('crontab.cronLabel')" prop="schedule">
          <el-input
            v-model="form.schedule"
            :placeholder="t('crontab.cronPlaceholder')"
            @input="preset = 'custom'"
          />
          <div class="field-tip">
            <span v-if="describeCron(form.schedule) !== form.schedule">
              {{ t('crontab.parsedDesc', { desc: describeCron(form.schedule) }) }}
            </span>
            <span v-else>{{ t('crontab.cronHint') }}</span>
          </div>
        </el-form-item>

        <el-form-item :label="t('crontab.enabledLabel')">
          <el-switch v-model="form.enabled" />
        </el-form-item>

        <el-form-item :label="t('crontab.remarkLabel')">
          <el-input
            v-model="form.remark"
            type="textarea"
            :rows="2"
            :placeholder="t('crontab.remarkPlaceholder')"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="handleSave">{{
          t('common.save')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 运行日志 -->
    <el-dialog v-model="logVisible" :title="logTitle" width="760px" @closed="stopPoll">
      <pre class="log-box">{{ logText || t('crontab.logEmpty') }}</pre>
      <template #footer>
        <span class="log-status">{{
          logDone ? t('crontab.logDone') : t('crontab.logRunning')
        }}</span>
        <el-button @click="logVisible = false">{{ t('crontab.close') }}</el-button>
      </template>
    </el-dialog>

    <CronRunHistory ref="historyRef" variant="crontab" @view="handleViewRun" @cleared="load" />
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Delete } from '@/icons'
import dayjs from 'dayjs'
import type { FormInstance, FormRules } from 'element-plus'
import {
  listCrontab,
  addCrontab,
  updateCrontab,
  deleteCrontab,
  toggleCrontab,
  runCrontabNow,
  readCrontabLog,
  listCrontabExecUsers,
  purgeCrontabLogs,
  type CronJob,
  type CrontabRunItem,
} from '@/api/crontab'
import { useUserStore } from '@/stores/user'
import CronRunHistory from '@/components/CronRunHistory.vue'

const { t } = useI18n()
const userStore = useUserStore()

const loading = ref(false)
const jobs = ref<CronJob[]>([])
const switching = ref(false)
const runningId = ref('')
const canChooseExec = ref(false)
const myExecUser = ref('')
const execUsers = ref<string[]>([])

/** 演示账号只读（与后端 demo_readonly_guard 一致） */
const readonly = computed(() => (userStore.roles || []).includes('demo'))

function fmt(ts: number) {
  return dayjs(ts * 1000).format('YYYY-MM-DD HH:mm')
}

function statusType(s: string) {
  if (s === 'success') return 'success'
  if (s === 'failed') return 'danger'
  if (s === 'running') return 'warning'
  return 'info'
}

function statusText(s: string) {
  return (
    {
      success: t('crontab.statusSuccess'),
      failed: t('crontab.statusFailed'),
      running: t('crontab.statusRunning'),
    }[s] || t('crontab.statusUnknown')
  )
}

// ── 频率预设 ────────────────────────────────────────────────
const presets = computed(() => [
  { value: 'custom', label: t('crontab.presetCustom') },
  { value: '* * * * *', label: t('crontab.presetEveryMinute') },
  { value: '*/5 * * * *', label: t('crontab.presetEvery5Min') },
  { value: '0 * * * *', label: t('crontab.presetHourly') },
  { value: '0 2 * * *', label: t('crontab.presetDaily') },
  { value: '0 3 * * 1', label: t('crontab.presetWeekly') },
  { value: '0 4 1 * *', label: t('crontab.presetMonthly') },
])
const preset = ref('custom')

const dowNames = computed(() => [
  t('crontab.dow0'),
  t('crontab.dow1'),
  t('crontab.dow2'),
  t('crontab.dow3'),
  t('crontab.dow4'),
  t('crontab.dow5'),
  t('crontab.dow6'),
])

function describeCron(s: string): string {
  const p = s.trim().split(/\s+/)
  if (p.length !== 5) return s
  const [m, h, dom, mon, dow] = p
  if (m === '*' && h === '*' && dom === '*' && mon === '*' && dow === '*')
    return t('crontab.descEveryMinute')
  if (m.startsWith('*/') && h === '*' && dom === '*' && mon === '*' && dow === '*')
    return t('crontab.descEveryNMinutes', { n: m.slice(2) })
  if (m === '0' && h.startsWith('*/') && dom === '*' && mon === '*' && dow === '*')
    return t('crontab.descEveryNHours', { n: h.slice(2) })
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && dom === '*' && mon === '*' && dow === '*')
    return t('crontab.descDaily', { time: `${h.padStart(2, '0')}:${m.padStart(2, '0')}` })
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && dom === '*' && mon === '*' && /^\d$/.test(dow))
    return t('crontab.descWeekly', {
      day: dowNames.value[Number(dow) % 7],
      time: `${h.padStart(2, '0')}:${m.padStart(2, '0')}`,
    })
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && /^\d+$/.test(dom) && mon === '*' && dow === '*')
    return t('crontab.descMonthly', {
      day: dom,
      time: `${h.padStart(2, '0')}:${m.padStart(2, '0')}`,
    })
  return s
}

function applyPreset(v: string) {
  if (v !== 'custom') form.schedule = v
}

// ── 表单 ────────────────────────────────────────────────────
const dialogVisible = ref(false)
const editing = ref<CronJob | null>(null)
const saving = ref(false)
const formRef = ref<FormInstance>()

const form = reactive({
  name: '',
  kind: 'command' as 'command' | 'script',
  command: '',
  exec_user: '',
  schedule: '* * * * *',
  remark: '',
  enabled: true,
})

const rules = computed<FormRules>(() => ({
  name: [{ required: true, message: t('crontab.nameRequired'), trigger: 'blur' }],
  exec_user: [{ required: true, message: t('crontab.execUserRequired'), trigger: 'change' }],
  command: [{ required: true, message: t('crontab.commandRequired'), trigger: 'blur' }],
  schedule: [{ required: true, message: t('crontab.scheduleRequired'), trigger: 'blur' }],
}))

function openCreate() {
  editing.value = null
  Object.assign(form, {
    name: '',
    kind: 'command',
    command: '',
    exec_user: canChooseExec.value ? execUsers.value[0] || '' : myExecUser.value,
    schedule: '* * * * *',
    remark: '',
    enabled: true,
  })
  preset.value = '* * * * *'
  dialogVisible.value = true
}

function openEdit(row: CronJob) {
  editing.value = row
  Object.assign(form, {
    name: row.name,
    kind: row.kind === 'script' ? 'script' : 'command',
    command: row.command,
    exec_user: row.exec_user,
    schedule: row.schedule,
    remark: row.remark,
    enabled: row.enabled,
  })
  preset.value = presets.value.some((p) => p.value === row.schedule && p.value !== 'custom')
    ? row.schedule
    : 'custom'
  dialogVisible.value = true
}

async function handleSave() {
  try {
    await formRef.value?.validate()
  } catch {
    return
  }
  saving.value = true
  try {
    const payload = {
      name: form.name.trim(),
      kind: form.kind,
      command: form.command.trim(),
      exec_user: form.exec_user,
      schedule: form.schedule.trim(),
      remark: form.remark.trim(),
    }
    if (editing.value) {
      await updateCrontab({ ...payload, id: editing.value.id, enabled: form.enabled })
      ElMessage.success(t('crontab.saved'))
    } else {
      await addCrontab(payload)
      ElMessage.success(t('crontab.created'))
    }
    dialogVisible.value = false
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('crontab.saveFailed'))
  } finally {
    saving.value = false
  }
}

async function handleToggle(row: CronJob, v: boolean) {
  switching.value = true
  try {
    await toggleCrontab(row.id, v)
    row.enabled = v
    ElMessage.success(v ? t('crontab.enabled') : t('crontab.disabled'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('crontab.opFailed'))
  } finally {
    switching.value = false
  }
}

async function handleRunNow(row: CronJob) {
  runningId.value = row.id
  try {
    const resp = await runCrontabNow(row.id)
    ElMessage.success(t('crontab.runTriggered'))
    row.last_run_id = resp.data.run_id
    row.last_run_at = Math.floor(Date.now() / 1000)
    row.last_status = 'running'
    openLog(row)
  } catch (e: any) {
    ElMessage.error(e.message || t('crontab.runFailed'))
  } finally {
    runningId.value = ''
  }
}

async function handleDelete(row: CronJob) {
  try {
    await ElMessageBox.confirm(
      t('crontab.deleteConfirm', { name: row.name }),
      t('crontab.deleteTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteCrontab(row.id)
    ElMessage.success(t('crontab.deleted'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('crontab.deleteFailed'))
  }
}

// ── 日志 / 运行历史 ─────────────────────────────────────────
const logVisible = ref(false)
const logTitle = ref(t('crontab.logTitle'))
const historyRef = ref<InstanceType<typeof CronRunHistory> | null>(null)
/** 当前历史抽屉所属任务名，用于日志弹窗标题 */
const historyJobName = ref('')
const logText = ref('')
const logDone = ref(false)
let logTimer: number | undefined

function stopPoll() {
  if (logTimer) {
    window.clearInterval(logTimer)
    logTimer = undefined
  }
}

async function fetchLog(runId: string) {
  try {
    const resp = await readCrontabLog(runId)
    logText.value = resp.data.log || ''
    logDone.value = resp.data.done
    if (logDone.value) stopPoll()
  } catch {
    /* 日志尚未生成时忽略 */
  }
}

/** 按 run_id 打开日志弹窗（历史列表点「查看日志」也走这里） */
function openLogByRunId(runId: string) {
  if (!runId) return
  stopPoll()
  logText.value = ''
  logDone.value = false
  logTitle.value = t('crontab.logTitleWithName', { name: historyJobName.value })
  logVisible.value = true
  fetchLog(runId)
  logTimer = window.setInterval(() => fetchLog(runId), 1000)
}

function openLog(row: CronJob) {
  historyJobName.value = row.name
  openLogByRunId(row.last_run_id)
}

function openHistory(row: CronJob) {
  historyJobName.value = row.name
  historyRef.value?.open({ id: row.id, name: row.name })
}

function handleViewRun(row: CrontabRunItem) {
  openLogByRunId(row.run_id)
}

/** 清理「开始登记运行记录之前」遗留的、无法归属到任务的日志 */
async function handlePurge() {
  try {
    await ElMessageBox.confirm(t('crontab.purgeConfirm'), t('crontab.purgeTitle'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    const resp = await purgeCrontabLogs()
    ElMessage.success(t('crontab.purged', { n: resp.data?.deleted ?? 0 }))
  } catch (e: any) {
    ElMessage.error(e.message || t('crontab.purgeFailed'))
  }
}

// ── 数据加载 ────────────────────────────────────────────────
async function load() {
  loading.value = true
  try {
    const resp = await listCrontab()
    jobs.value = resp.data.jobs || []
    canChooseExec.value = !!resp.data.can_choose_exec
    myExecUser.value = resp.data.exec_user || ''
  } catch (e: any) {
    ElMessage.error(e.message || t('crontab.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function loadExecUsers() {
  try {
    const resp = await listCrontabExecUsers()
    execUsers.value = resp.data.users || []
  } catch {
    execUsers.value = []
  }
}

onMounted(async () => {
  await load()
  if (canChooseExec.value) await loadExecUsers()
})

onBeforeUnmount(stopPoll)
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}

.header-actions {
  display: flex;
  gap: 8px;
}

.cron-alert {
  margin-bottom: 14px;
}

.mono {
  font-family: monospace;
  font-size: 12px;
}

.kind-tag {
  margin-right: 6px;
}

.cron-desc {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}

.link-like {
  color: #409eff;
  cursor: pointer;
  margin-right: 6px;
}

.link-like:hover {
  text-decoration: underline;
}

.field-tip {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
  margin-top: 2px;
}

.log-box {
  margin: 0;
  max-height: 420px;
  overflow: auto;
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 12px;
  border-radius: 4px;
  font-family: monospace;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
}

.log-status {
  float: left;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 32px;
}
</style>
