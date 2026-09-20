<template>
  <div class="cron-page">
    <el-card shadow="never" class="base-card">
      <template #header>
        <div class="card-header">
          <span>{{ t('automationCron.title') }}</span>
          <el-button type="primary" :icon="Plus" @click="openCreate">{{
            t('automationCron.newJob')
          }}</el-button>
        </div>
      </template>

      <el-alert
        type="info"
        :closable="false"
        class="cron-alert"
        :title="t('automationCron.alert')"
      />

      <el-table :data="jobs" v-loading="loading" style="width: 100%">
        <el-table-column
          prop="name"
          :label="t('automationCron.colName')"
          min-width="140"
          show-overflow-tooltip
        />
        <el-table-column :label="t('automationCron.colScript')" min-width="200">
          <template #default="{ row }">
            <span class="script-path">{{ row.script_path }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('automationCron.colSchedule')" min-width="150">
          <template #default="{ row }">
            <div>
              <span class="cron-schedule">{{ row.schedule }}</span>
              <div class="cron-desc">{{ describeCron(row.schedule) }}</div>
            </div>
          </template>
        </el-table-column>
        <el-table-column :label="t('automationCron.colStatus')" width="90">
          <template #default="{ row }">
            <el-switch
              :model-value="row.enabled"
              :disabled="switching"
              @change="(v: boolean) => handleToggle(row, v)"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('automationCron.colLastRun')" width="160">
          <template #default="{ row }">
            <span v-if="row.last_run_at > 0" class="link-like" @click="openLog(row)">
              {{ fmt(row.last_run_at) }}
            </span>
            <span v-else>—</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('automationCron.colNextRun')" width="160">
          <template #default="{ row }">
            {{ row.enabled && row.next_run_at > 0 ? fmt(row.next_run_at) : '—' }}
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="240" fixed="right">
          <template #default="{ row }">
            <el-button
              link
              type="primary"
              :disabled="runningId === row.id"
              @click="handleRunNow(row)"
            >
              {{ runningId === row.id ? t('automationCron.running') : t('automationCron.runNow') }}
            </el-button>
            <el-button link type="primary" @click="openHistory(row)">
              {{ t('cronHistory.history') }}
            </el-button>
            <el-button link type="primary" @click="openEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button link type="danger" @click="handleDelete(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新建 / 编辑 -->
    <el-dialog
      v-model="dialogVisible"
      :title="editing ? t('automationCron.editTitle') : t('automationCron.newJob')"
      width="620px"
      :close-on-click-modal="false"
    >
      <el-form ref="formRef" :model="form" :rules="rules" label-width="90px" @submit.prevent>
        <el-form-item :label="t('automationCron.nameLabel')" prop="name">
          <el-input
            v-model="form.name"
            :placeholder="t('automationCron.namePlaceholder')"
            maxlength="60"
          />
        </el-form-item>
        <el-form-item :label="t('automationCron.scriptLabel')" prop="script_path">
          <el-select
            v-model="form.script_path"
            filterable
            allow-create
            default-first-option
            style="width: 100%"
            :placeholder="t('automationCron.scriptPlaceholder')"
          >
            <el-option v-for="s in scriptFiles" :key="s" :label="s" :value="s" />
          </el-select>
          <div class="field-tip">{{ t('automationCron.scriptTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('automationCron.presetLabel')">
          <el-select v-model="preset" style="width: 100%" @change="applyPreset">
            <el-option v-for="p in presets" :key="p.value" :label="p.label" :value="p.value" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('automationCron.cronLabel')" prop="schedule">
          <el-input
            v-model="form.schedule"
            :placeholder="t('automationCron.cronPlaceholder')"
            @input="preset = 'custom'"
          />
          <div class="field-tip">
            <span v-if="describeCron(form.schedule) !== form.schedule">
              {{ t('automationCron.parsedDesc', { desc: describeCron(form.schedule) }) }}
            </span>
            <span v-else>{{ t('automationCron.cronHint') }}</span>
          </div>
        </el-form-item>
        <el-form-item :label="t('automationCron.remarkLabel')">
          <el-input
            v-model="form.remark"
            type="textarea"
            :rows="2"
            :placeholder="t('automationCron.remarkPlaceholder')"
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

    <AppStoreLogDrawer ref="logDrawerRef" />
    <CronRunHistory ref="historyRef" variant="system" @view="handleViewRun" @cleared="load" />
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus } from '@/icons'
import dayjs from 'dayjs'
import type { FormInstance, FormRules } from 'element-plus'
import { getScriptsTree } from '@/api/appstore'
import {
  listCronJobs,
  addCronJob,
  updateCronJob,
  deleteCronJob,
  toggleCronJob,
  runCronJobNow,
  type CronJob,
  type CronRunItem,
} from '@/api/cron'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import CronRunHistory from '@/components/CronRunHistory.vue'

const { t } = useI18n()

const loading = ref(false)
const jobs = ref<CronJob[]>([])
const switching = ref(false)
const runningId = ref('')

function fmt(ts: number) {
  return dayjs(ts * 1000).format('YYYY-MM-DD HH:mm')
}

// ── 频率预设 ────────────────────────────────────────────────
const presets = computed(() => [
  { value: 'custom', label: t('automationCron.presetCustom') },
  { value: '* * * * *', label: t('automationCron.presetEveryMinute') },
  { value: '*/5 * * * *', label: t('automationCron.presetEvery5Min') },
  { value: '0 * * * *', label: t('automationCron.presetHourly') },
  { value: '0 2 * * *', label: t('automationCron.presetDaily') },
  { value: '0 3 * * 1', label: t('automationCron.presetWeekly') },
  { value: '0 4 1 * *', label: t('automationCron.presetMonthly') },
])
const preset = ref('custom')

const dowNames = computed(() => [
  t('automationCron.dow0'),
  t('automationCron.dow1'),
  t('automationCron.dow2'),
  t('automationCron.dow3'),
  t('automationCron.dow4'),
  t('automationCron.dow5'),
  t('automationCron.dow6'),
])

/** 把常见 cron 表达式转中文；无法识别时原样返回（调用处回退为帮助文案） */
function describeCron(s: string): string {
  const p = s.trim().split(/\s+/)
  if (p.length !== 5) return s
  const [m, h, dom, mon, dow] = p
  if (m === '*' && h === '*' && dom === '*' && mon === '*' && dow === '*')
    return t('automationCron.descEveryMinute')
  if (m.startsWith('*/') && h === '*' && dom === '*' && mon === '*' && dow === '*')
    return t('automationCron.descEveryNMinutes', { n: m.slice(2) })
  if (m === '0' && h.startsWith('*/') && dom === '*' && mon === '*' && dow === '*')
    return t('automationCron.descEveryNHours', { n: h.slice(2) })
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && dom === '*' && mon === '*' && dow === '*')
    return t('automationCron.descDaily', { time: `${h.padStart(2, '0')}:${m.padStart(2, '0')}` })
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && dom === '*' && mon === '*' && /^\d$/.test(dow))
    return t('automationCron.descWeekly', {
      day: dowNames.value[Number(dow) % 7],
      time: `${h.padStart(2, '0')}:${m.padStart(2, '0')}`,
    })
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && /^\d+$/.test(dom) && mon === '*' && dow === '*')
    return t('automationCron.descMonthly', {
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
  script_path: '',
  schedule: '* * * * *',
  remark: '',
  enabled: true,
})

const rules = computed<FormRules>(() => ({
  name: [{ required: true, message: t('automationCron.nameRequired'), trigger: 'blur' }],
  script_path: [{ required: true, message: t('automationCron.scriptRequired'), trigger: 'change' }],
  schedule: [{ required: true, message: t('automationCron.scheduleRequired'), trigger: 'blur' }],
}))

function openCreate() {
  editing.value = null
  Object.assign(form, {
    name: '',
    script_path: '',
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
    script_path: row.script_path,
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
    if (editing.value) {
      await updateCronJob({
        id: editing.value.id,
        name: form.name.trim(),
        script_path: form.script_path.trim(),
        schedule: form.schedule.trim(),
        remark: form.remark.trim(),
        enabled: form.enabled,
      })
      ElMessage.success(t('automationCron.saved'))
    } else {
      await addCronJob({
        name: form.name.trim(),
        script_path: form.script_path.trim(),
        schedule: form.schedule.trim(),
        remark: form.remark.trim(),
      })
      ElMessage.success(t('automationCron.created'))
    }
    dialogVisible.value = false
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('automationCron.saveFailed'))
  } finally {
    saving.value = false
  }
}

async function handleToggle(row: CronJob, v: boolean) {
  switching.value = true
  try {
    await toggleCronJob(row.id, v)
    row.enabled = v
    ElMessage.success(v ? t('automationCron.enabled') : t('automationCron.disabled'))
  } catch (e: any) {
    ElMessage.error(e.message || t('automationCron.opFailed'))
  } finally {
    switching.value = false
  }
}

async function handleRunNow(row: CronJob) {
  runningId.value = row.id
  try {
    const resp = await runCronJobNow(row.id)
    ElMessage.success(t('automationCron.runTriggered'))
    row.last_run_id = resp.data.run_id
    row.last_run_at = Math.floor(Date.now() / 1000)
    logDrawerRef.value?.openDrawer(
      resp.data.run_id,
      t('automationCron.runLogTitle', { name: row.name }),
    )
  } catch (e: any) {
    ElMessage.error(e.message || t('automationCron.runFailed'))
  } finally {
    runningId.value = ''
  }
}

async function handleDelete(row: CronJob) {
  try {
    await ElMessageBox.confirm(
      t('automationCron.deleteConfirm', { name: row.name }),
      t('automationCron.deleteTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteCronJob(row.id)
    ElMessage.success(t('automationCron.deleted'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('automationCron.deleteFailed'))
  }
}

function openLog(row: CronJob) {
  if (!row.last_run_id) return
  logDrawerRef.value?.openDrawer(
    row.last_run_id,
    t('automationCron.runLogTitle', { name: row.name }),
  )
}

const logDrawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
const historyRef = ref<InstanceType<typeof CronRunHistory> | null>(null)
/** 当前历史抽屉所属任务名，仅用于给日志抽屉凑标题 */
const historyJobName = ref('')

function openHistory(row: CronJob) {
  historyJobName.value = row.name
  historyRef.value?.open({ id: row.id, name: row.name })
}

function handleViewRun(row: CronRunItem) {
  logDrawerRef.value?.openDrawer(
    row.run_id,
    t('automationCron.runLogTitle', { name: historyJobName.value }),
  )
}

// ── 脚本候选（扁平化 tree） ─────────────────────────────────
const scriptFiles = ref<string[]>([])

interface TreeNode {
  type: 'dir' | 'file'
  name: string
  path: string
  children?: TreeNode[]
}

function flatten(nodes: TreeNode[], prefix = ''): string[] {
  const out: string[] = []
  for (const n of nodes) {
    const path = n.path || `${prefix}/${n.name}`.replace(/^\/+/, '')
    if (n.type === 'file') out.push(path)
    else if (n.children) out.push(...flatten(n.children, path))
  }
  return out
}

async function loadScriptFiles() {
  try {
    const resp = await getScriptsTree()
    const tree = resp.data?.tree
    if (tree && tree.children) scriptFiles.value = flatten(tree.children)
  } catch {
    scriptFiles.value = []
  }
}

async function load() {
  loading.value = true
  try {
    const resp = await listCronJobs()
    jobs.value = resp.data.jobs || []
  } catch (e: any) {
    ElMessage.error(e.message || t('automationCron.loadFailed'))
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  load()
  loadScriptFiles()
})
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}

.cron-alert {
  margin-bottom: 14px;
}

.script-path {
  font-family: monospace;
  font-size: 12px;
  color: #409eff;
}

.cron-schedule {
  font-family: monospace;
  font-size: 12px;
  color: var(--el-text-color-primary);
}

.cron-desc {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}

.link-like {
  color: #409eff;
  cursor: pointer;
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
</style>
