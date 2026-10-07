<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="job-tasks">
    <!-- 开关关闭时的提示（仅普通用户会命中） -->
    <el-alert
      v-if="!isAdmin && !policyAllowJob"
      type="info"
      :closable="false"
      show-icon
      :title="t('backup.jobDisabledTitle')"
      :description="t('backup.jobDisabledDesc')"
      style="margin-bottom: 16px"
    />

    <template v-else>
      <div class="card-head">
        <span>{{ t('backup.jobsTitle') }}</span>
        <el-button type="primary" @click="openJobDialog(null)">{{ t('backup.newJob') }}</el-button>
      </div>
      <el-table :data="jobs" v-loading="loadingJobs" size="small">
        <el-table-column prop="name" :label="t('backup.colName')" min-width="160" />
        <el-table-column prop="target_type" :label="t('backup.colType')" width="80" />
        <el-table-column :label="t('backup.colSchedule')" min-width="160">
          <template #default="{ row }">
            <div>{{ describeCron(row.schedule) }}</div>
            <div class="muted">{{ row.schedule }}</div>
          </template>
        </el-table-column>
        <el-table-column :label="t('backup.colRetain')" width="80">
          <template #default="{ row }">{{ row.retain_count }}</template>
        </el-table-column>
        <el-table-column :label="t('backup.colEnabled')" width="90">
          <template #default="{ row }">
            <el-switch v-model="row.enabled" :active-value="1" :inactive-value="0"
              @change="onToggleJob(row)" />
          </template>
        </el-table-column>
        <el-table-column :label="t('backup.colLastRun')" width="200">
          <template #default="{ row }">
            <span v-if="row.last_run_at">{{ formatTime(row.last_run_at) }}</span>
            <el-tag v-if="row.last_status === 1" size="small" type="success">{{ t('backup.statusOk') }}</el-tag>
            <el-tag v-else-if="row.last_status === -1" size="small" type="danger">{{ t('backup.statusFail') }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('backup.colAction')" width="220">
          <template #default="{ row }">
            <el-button text type="primary" @click="onRunJob(row)">{{ t('backup.runNow') }}</el-button>
            <el-button text type="primary" @click="openJobDialog(row)">{{ t('backup.edit') }}</el-button>
            <el-button text type="danger" @click="onDeleteJob(row)">{{ t('backup.delete') }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </template>

    <!-- 任务编辑对话框 -->
    <el-dialog v-model="jobDialog" :title="jobForm.id ? t('backup.editTitle') : t('backup.newTitle')" width="560px">
      <el-form :model="jobForm" label-width="96px">
        <el-form-item :label="t('backup.nameLabel')">
          <el-input v-model="jobForm.name" />
        </el-form-item>
        <el-form-item :label="t('backup.kindLabel')">
          <el-radio-group v-model="jobForm.target_type">
            <el-radio value="dir">{{ t('backup.kindDirFile') }}</el-radio>
            <el-radio value="db">{{ t('backup.kindDb') }}</el-radio>
          </el-radio-group>
        </el-form-item>

        <template v-if="jobForm.target_type === 'dir'">
          <el-form-item :label="t('backup.pathLabel')">
            <el-input v-model="jobForm.dirPaths" type="textarea" :rows="3" :placeholder="t('backup.pathPh')" />
            <span v-if="!isAdmin" class="tip">{{ t('backup.pathUserTip') }}</span>
          </el-form-item>
        </template>
        <template v-else>
          <el-form-item :label="t('backup.engineLabel')">
            <el-select v-model="jobForm.engine" style="width:160px">
              <el-option label="MySQL / MariaDB" value="mysql" />
              <el-option label="SQLite" value="sqlite" />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('backup.dbNameLabel')">
            <el-input v-model="jobForm.dbName" />
            <span v-if="!isAdmin" class="tip">{{ t('backup.dbNameUserTip') }}</span>
          </el-form-item>
          <el-collapse class="more-collapse">
            <el-collapse-item :title="t('backup.moreConn')">
              <el-form-item :label="t('backup.dbUser')">
                <el-input v-model="jobForm.dbUser" :placeholder="t('backup.dbUserPh')" />
              </el-form-item>
              <el-form-item :label="t('backup.dbPass')">
                <el-input v-model="jobForm.dbPass" type="password" show-password :placeholder="t('backup.dbPassPh')" />
              </el-form-item>
              <el-form-item :label="t('backup.host')">
                <el-input v-model="jobForm.dbHost" placeholder="127.0.0.1" />
              </el-form-item>
              <el-form-item :label="t('backup.port')">
                <el-input v-model="jobForm.dbPort" placeholder="3306" />
              </el-form-item>
              <el-form-item v-if="jobForm.engine === 'sqlite'" :label="t('backup.dbFile')">
                <el-input v-model="jobForm.dbPath" />
              </el-form-item>
            </el-collapse-item>
          </el-collapse>
        </template>

        <el-form-item :label="t('backup.scheduleLabel')">
          <el-select
            v-model="jobForm.schedule"
            filterable
            allow-create
            default-first-option
            :placeholder="t('backup.schedulePh')"
            style="width: 100%"
          >
            <el-option :label="t('backup.cronDaily3')" value="0 3 * * *" />
            <el-option :label="t('backup.cronDaily4')" value="0 4 * * *" />
            <el-option :label="t('backup.cronWeekly')" value="0 3 * * 1" />
            <el-option :label="t('backup.cronMonthly')" value="0 3 1 * *" />
            <el-option :label="t('backup.cronHourly')" value="0 * * * *" />
          </el-select>
          <span class="tip">{{ t('backup.scheduleTip', { desc: describeCron(jobForm.schedule) }) }}</span>
        </el-form-item>
        <el-form-item :label="t('backup.retainLabel')">
          <el-input-number v-model="jobForm.retain_count" :min="0" :max="999" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="jobDialog = false">{{ t('backup.cancel') }}</el-button>
        <el-button type="primary" :loading="savingJob" @click="onSaveJob">{{ t('backup.save') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { http } from '@/utils/request'
import { useUserStore } from '@/stores/user'

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => (userStore.roles || []).includes('admin'))

/** 普通用户侧：开关关闭时不展示任务列表与新建入口 */
const policyAllowJob = ref(true)

const loadingJobs = ref(false)
const jobs = ref<any[]>([])

const jobDialog = ref(false)
const savingJob = ref(false)
const jobForm = ref({
  id: 0,
  name: '',
  target_type: 'dir',
  dirPaths: '',
  engine: 'mysql',
  dbName: '',
  dbUser: '',
  dbPass: '',
  dbHost: '',
  dbPort: '',
  dbPath: '',
  schedule: '0 3 * * *',
  retain_count: 7,
})

async function loadPolicyLight() {
  if (isAdmin.value) return // 管理员始终可见
  try {
    const d: any = await http.get('/system/backup/policy')
    policyAllowJob.value = d.data?.allow_user_job !== false
  } catch {
    // 读不到策略默认放行，交给后端按 scope 兜底
    policyAllowJob.value = true
  }
}

async function loadJobs() {
  loadingJobs.value = true
  try {
    const d: any = await http.get('/system/backup/jobs')
    jobs.value = d.data?.jobs || []
  } catch (e: any) {
    jobs.value = []
    // 普通用户被开关拦截时已在页面上给出提示，不再重复报错
    if (!(e?.message || '').includes('已关闭')) {
      ElMessage.error(e?.message || t('backup.loadFailed'))
    }
  } finally {
    loadingJobs.value = false
  }
}

function openJobDialog(row: any) {
  if (row) {
    let t2: any = {}
    try { t2 = JSON.parse(row.target || '{}') } catch {}
    jobForm.value = {
      id: row.id,
      name: row.name,
      target_type: row.target_type,
      dirPaths: (t2.paths || []).join('\n'),
      engine: t2.engine || 'mysql',
      dbName: t2.db_name || '',
      dbUser: t2.user || '',
      dbPass: t2.password || '',
      dbHost: t2.host || '',
      dbPort: t2.port ? String(t2.port) : '',
      dbPath: t2.db_path || '',
      schedule: row.schedule,
      retain_count: row.retain_count,
    }
  } else {
    jobForm.value = {
      id: 0, name: '', target_type: 'dir', dirPaths: '', engine: 'mysql',
      dbName: '', dbUser: '', dbPass: '', dbHost: '', dbPort: '', dbPath: '',
      schedule: '0 3 * * *', retain_count: 7,
    }
  }
  jobDialog.value = true
}

async function onSaveJob() {
  const f = jobForm.value
  if (!f.name.trim()) return ElMessage.warning(t('backup.nameRequired'))
  let target: any
  if (f.target_type === 'dir') {
    const paths = f.dirPaths.split('\n').map((s) => s.trim()).filter(Boolean)
    if (!paths.length) return ElMessage.warning(t('backup.pathsRequired'))
    target = { paths }
  } else {
    if (!f.dbName.trim()) return ElMessage.warning(t('backup.dbNameNameRequired'))
    target = {
      engine: f.engine,
      db_name: f.dbName,
      user: f.dbUser,
      password: f.dbPass,
      host: f.dbHost,
      port: parseInt(f.dbPort || '0', 10) || 0,
      db_path: f.engine === 'sqlite' ? f.dbPath : undefined,
    }
  }
  savingJob.value = true
  try {
    await http.post('/system/backup/job/save', {
      id: f.id || undefined,
      name: f.name.trim(),
      target_type: f.target_type,
      target: JSON.stringify(target),
      schedule: f.schedule,
      retain_count: f.retain_count,
    })
    ElMessage.success(t('backup.saved'))
    jobDialog.value = false
    loadJobs()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.saveFailed'))
  } finally {
    savingJob.value = false
  }
}

async function onRunJob(row: any) {
  try {
    await http.post('/system/backup/job/run', { id: row.id })
    ElMessage.success(t('backup.triggered'))
    setTimeout(loadJobs, 1500)
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.runFailed'))
  }
}

async function onToggleJob(row: any) {
  try {
    await http.post('/system/backup/job/save', {
      id: row.id,
      name: row.name,
      target_type: row.target_type,
      target: row.target,
      schedule: row.schedule,
      enabled: row.enabled,
      retain_count: row.retain_count,
    })
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.updateFailed'))
    loadJobs()
  }
}

async function onDeleteJob(row: any) {
  try {
    await ElMessageBox.confirm(t('backup.deleteJobConfirm', { name: row.name }), t('backup.notice'), { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.post('/system/backup/job/delete', { id: row.id })
    ElMessage.success(t('backup.deleted'))
    loadJobs()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.deleteFailed'))
  }
}

function formatBytes(n: number) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let v = n
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++ }
  return `${(v).toFixed(1)} ${u[i]}`
}
function formatTime(ts: number) {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const p = (x: number) => String(x).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

/** cron 表达式 -> 人类可读说明（命中预设走 i18n，未命中回退原始表达式） */
const CRON_DESC_KEY: Record<string, string> = {
  '0 3 * * *': 'backup.cronDaily3',
  '0 4 * * *': 'backup.cronDaily4',
  '0 3 * * 1': 'backup.cronWeekly',
  '0 3 1 * *': 'backup.cronMonthly',
  '0 * * * *': 'backup.cronHourly',
  '* * * * *': 'backup.cronMinute',
}
function describeCron(expr?: string) {
  const e = (expr || '').trim()
  if (!e) return t('backup.cronUnset')
  if (CRON_DESC_KEY[e]) return t(CRON_DESC_KEY[e])
  const m = e.match(/^\d+ (\d+) \* \* \*$/)
  if (m) return t('backup.cronDailyAt', { h: String(m[1]).padStart(2, '0') })
  const h = e.match(/^\d+ (\d+) \* \* (\d+)$/)
  if (h) {
    const wd = t('backup.weekdays').split(',')
    return t('backup.cronWeeklyAt', { d: wd[Number(h[2])] || h[2], h: String(h[1]).padStart(2, '0') })
  }
  return t('backup.cronCustom', { expr: e })
}

onMounted(() => {
  loadPolicyLight()
  loadJobs()
})
</script>

<style scoped>
.job-tasks { padding: 4px 0; }
.card-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
.muted { color: var(--el-text-color-secondary); font-size: 12px; }
.tip { color: #909399; font-size: 12px; }
</style>
