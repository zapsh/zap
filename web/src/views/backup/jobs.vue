<template>
  <div class="job-tasks">
    <!-- 开关关闭时的提示（仅普通用户会命中） -->
    <el-alert
      v-if="!isAdmin && !policyAllowJob"
      type="info"
      :closable="false"
      show-icon
      title="定时备份功能未开启"
      description="管理员已在「备份策略」中关闭「允许用户定时备份」，暂无法创建或管理定时备份任务。如需开启请联系管理员。"
      style="margin-bottom: 16px"
    />

    <template v-else>
      <div class="card-head">
        <span>备份任务</span>
        <el-button type="primary" @click="openJobDialog(null)">新建任务</el-button>
      </div>
      <el-table :data="jobs" v-loading="loadingJobs" size="small">
        <el-table-column prop="name" label="名称" min-width="160" />
        <el-table-column prop="target_type" label="类型" width="80" />
        <el-table-column label="计划(cron)" min-width="160">
          <template #default="{ row }">
            <div>{{ describeCron(row.schedule) }}</div>
            <div class="muted">{{ row.schedule }}</div>
          </template>
        </el-table-column>
        <el-table-column label="保留份数" width="80">
          <template #default="{ row }">{{ row.retain_count }}</template>
        </el-table-column>
        <el-table-column label="启用" width="90">
          <template #default="{ row }">
            <el-switch v-model="row.enabled" :active-value="1" :inactive-value="0"
              @change="onToggleJob(row)" />
          </template>
        </el-table-column>
        <el-table-column label="上次运行" width="200">
          <template #default="{ row }">
            <span v-if="row.last_run_at">{{ formatTime(row.last_run_at) }}</span>
            <el-tag v-if="row.last_status === 1" size="small" type="success">成功</el-tag>
            <el-tag v-else-if="row.last_status === -1" size="small" type="danger">失败</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="220">
          <template #default="{ row }">
            <el-button text type="primary" @click="onRunJob(row)">立即执行</el-button>
            <el-button text type="primary" @click="openJobDialog(row)">编辑</el-button>
            <el-button text type="danger" @click="onDeleteJob(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </template>

    <!-- 任务编辑对话框 -->
    <el-dialog v-model="jobDialog" :title="jobForm.id ? '编辑任务' : '新建任务'" width="560px">
      <el-form :model="jobForm" label-width="96px">
        <el-form-item label="任务名">
          <el-input v-model="jobForm.name" />
        </el-form-item>
        <el-form-item label="备份类型">
          <el-radio-group v-model="jobForm.target_type">
            <el-radio value="dir">目录 / 文件</el-radio>
            <el-radio value="db">数据库</el-radio>
          </el-radio-group>
        </el-form-item>

        <template v-if="jobForm.target_type === 'dir'">
          <el-form-item label="备份路径">
            <el-input v-model="jobForm.dirPaths" type="textarea" :rows="3" placeholder="每行一个绝对路径" />
            <span v-if="!isAdmin" class="tip">仅能填写你自己家目录内的绝对路径（后端会校验）</span>
          </el-form-item>
        </template>
        <template v-else>
          <el-form-item label="数据库类型">
            <el-select v-model="jobForm.engine" style="width:160px">
              <el-option label="MySQL / MariaDB" value="mysql" />
              <el-option label="SQLite" value="sqlite" />
            </el-select>
          </el-form-item>
          <el-form-item label="数据库名">
            <el-input v-model="jobForm.dbName" />
            <span v-if="!isAdmin" class="tip">仅能填写你自己名下的数据库（库名需以你的账号前缀开头，后端会校验）</span>
          </el-form-item>
          <el-collapse class="more-collapse">
            <el-collapse-item title="更多连接信息（留空则用本机 zapadm 直连，填写后备份远程数据库）">
              <el-form-item label="用户名">
                <el-input v-model="jobForm.dbUser" placeholder="留空 = zapadm" />
              </el-form-item>
              <el-form-item label="密码">
                <el-input v-model="jobForm.dbPass" type="password" show-password placeholder="留空 = 面板凭据" />
              </el-form-item>
              <el-form-item label="主机">
                <el-input v-model="jobForm.dbHost" placeholder="127.0.0.1" />
              </el-form-item>
              <el-form-item label="端口">
                <el-input v-model="jobForm.dbPort" placeholder="3306" />
              </el-form-item>
              <el-form-item v-if="jobForm.engine === 'sqlite'" label="数据库文件">
                <el-input v-model="jobForm.dbPath" />
              </el-form-item>
            </el-collapse-item>
          </el-collapse>
        </template>

        <el-form-item label="计划(cron)">
          <el-select
            v-model="jobForm.schedule"
            filterable
            allow-create
            default-first-option
            placeholder="如 0 3 * * *（每天 3 点）"
            style="width: 100%"
          >
            <el-option label="每天 03:00" value="0 3 * * *" />
            <el-option label="每天 04:00" value="0 4 * * *" />
            <el-option label="每周一 03:00" value="0 3 * * 1" />
            <el-option label="每月 1 号 03:00" value="0 3 1 * *" />
            <el-option label="每小时" value="0 * * * *" />
          </el-select>
          <span class="tip">当前执行：{{ describeCron(jobForm.schedule) }}（标准 5 段 cron，可直接输入自定义表达式）</span>
        </el-form-item>
        <el-form-item label="保留份数">
          <el-input-number v-model="jobForm.retain_count" :min="0" :max="999" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="jobDialog = false">取消</el-button>
        <el-button type="primary" :loading="savingJob" @click="onSaveJob">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { http } from '@/utils/request'
import { useUserStore } from '@/stores/user'

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
      ElMessage.error(e?.message || '加载失败')
    }
  } finally {
    loadingJobs.value = false
  }
}

function openJobDialog(row: any) {
  if (row) {
    let t: any = {}
    try { t = JSON.parse(row.target || '{}') } catch {}
    jobForm.value = {
      id: row.id,
      name: row.name,
      target_type: row.target_type,
      dirPaths: (t.paths || []).join('\n'),
      engine: t.engine || 'mysql',
      dbName: t.db_name || '',
      dbUser: t.user || '',
      dbPass: t.password || '',
      dbHost: t.host || '',
      dbPort: t.port ? String(t.port) : '',
      dbPath: t.db_path || '',
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
  if (!f.name.trim()) return ElMessage.warning('请填写任务名')
  let target: any
  if (f.target_type === 'dir') {
    const paths = f.dirPaths.split('\n').map((s) => s.trim()).filter(Boolean)
    if (!paths.length) return ElMessage.warning('请填写备份路径')
    target = { paths }
  } else {
    if (!f.dbName.trim()) return ElMessage.warning('请填写数据库名')
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
    ElMessage.success('已保存')
    jobDialog.value = false
    loadJobs()
  } catch (e: any) {
    ElMessage.error(e?.message || '保存失败')
  } finally {
    savingJob.value = false
  }
}

async function onRunJob(row: any) {
  try {
    await http.post('/system/backup/job/run', { id: row.id })
    ElMessage.success('已触发执行')
    setTimeout(loadJobs, 1500)
  } catch (e: any) {
    ElMessage.error(e?.message || '执行失败')
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
    ElMessage.error(e?.message || '更新失败')
    loadJobs()
  }
}

async function onDeleteJob(row: any) {
  try {
    await ElMessageBox.confirm(`确认删除任务 ${row.name}？`, '提示', { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.post('/system/backup/job/delete', { id: row.id })
    ElMessage.success('已删除')
    loadJobs()
  } catch (e: any) {
    ElMessage.error(e?.message || '删除失败')
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

const CRON_DESC: Record<string, string> = {
  '0 3 * * *': '每天 03:00',
  '0 4 * * *': '每天 04:00',
  '0 3 * * 1': '每周一 03:00',
  '0 3 1 * *': '每月 1 号 03:00',
  '0 * * * *': '每小时',
  '* * * * *': '每分钟',
}
function describeCron(expr?: string) {
  const e = (expr || '').trim()
  if (!e) return '未设置'
  if (CRON_DESC[e]) return CRON_DESC[e]
  const m = e.match(/^\d+ (\d+) \* \* \*$/)
  if (m) return `每天 ${String(m[1]).padStart(2, '0')}:00`
  const h = e.match(/^\d+ (\d+) \* \* (\d+)$/)
  if (h) return `每周 ${['日','一','二','三','四','五','六'][Number(h[2])] || h[2]} ${String(h[1]).padStart(2, '0')}:00`
  return `自定义：${e}`
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
