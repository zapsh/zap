<template>
  <div class="backup-wrap">
    <!-- 备份存储目录（可改到其他磁盘 / 挂载点，缓解系统盘空间不足） -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-head">
          <span>备份存储目录</span>
          <el-button text type="primary" @click="loadSetting">刷新</el-button>
        </div>
      </template>
      <el-form label-width="96px" class="form">
        <el-form-item label="存储目录">
          <el-input v-model="setting.path" placeholder="如 /data/backups（绝对路径，留空=默认 {ZAP_PATH}/data/backup）" />
        </el-form-item>
        <el-form-item label="磁盘占用">
          <el-progress
            :percentage="diskPercent"
            :status="diskPercent > 90 ? 'exception' : ''"
            :stroke-width="14"
            style="width: 100%"
          />
          <span class="disk-tip">
            可用 {{ formatBytes(setting.disk_free) }} / 共 {{ formatBytes(setting.disk_total) }}
            <span v-if="diskPercent > 90" style="color:#f56c6c">（空间紧张，建议改到更大磁盘）</span>
          </span>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="savingSetting" @click="saveSetting">保存目录</el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <el-tabs v-model="tab">
      <el-tab-pane label="备份清单" name="archives">
        <!-- 立即备份 -->
        <el-card shadow="never" class="block">
          <template #header>
            <span>立即备份</span>
          </template>
          <el-form :model="form" label-width="90px" :inline="false" class="form">
            <el-form-item label="备份类型">
              <el-radio-group v-model="form.kind">
                <el-radio value="dir">目录 / 文件</el-radio>
                <el-radio value="db">数据库</el-radio>
              </el-radio-group>
            </el-form-item>

            <template v-if="form.kind === 'dir'">
              <el-form-item label="归档名">
                <el-input v-model="form.name" placeholder="如 web-backup" />
              </el-form-item>
              <el-form-item label="待备份路径">
                <el-input v-model="form.pathsText" type="textarea" :rows="3"
                  placeholder="每行一个绝对路径，如 /home/u/www/blog" />
              </el-form-item>
            </template>

            <template v-else>
              <el-form-item label="数据库类型">
                <el-select v-model="form.engine" style="width:160px">
                  <el-option label="MySQL / MariaDB" value="mysql" />
                  <el-option label="SQLite" value="sqlite" />
                </el-select>
              </el-form-item>
              <el-form-item label="归档名">
                <el-input v-model="form.name" placeholder="如 db-blog" />
              </el-form-item>
              <el-form-item label="数据库名">
                <el-input v-model="form.dbName" placeholder="要导出的数据库名" />
              </el-form-item>
              <el-form-item v-if="form.engine === 'mysql'" label="用户名">
                <el-input v-model="form.dbUser" />
              </el-form-item>
              <el-form-item v-if="form.engine === 'mysql'" label="密码">
                <el-input v-model="form.dbPass" type="password" show-password />
              </el-form-item>
              <el-form-item v-if="form.engine === 'mysql'" label="主机">
                <el-input v-model="form.dbHost" placeholder="127.0.0.1" />
              </el-form-item>
              <el-form-item v-if="form.engine === 'mysql'" label="端口">
                <el-input v-model="form.dbPort" placeholder="3306" />
              </el-form-item>
              <el-form-item v-if="form.engine === 'sqlite'" label="数据库文件">
                <el-input v-model="form.dbPath" placeholder="如 /usr/local/zap/data/zap.db" />
              </el-form-item>
            </template>

            <el-form-item>
              <el-button type="primary" :loading="running" @click="onManualBackup">
                立即备份
              </el-button>
            </el-form-item>
          </el-form>
        </el-card>

        <!-- 备份归档 -->
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>备份归档</span>
              <el-button text type="primary" @click="loadArchives">刷新</el-button>
            </div>
          </template>
          <el-table :data="archives" v-loading="loadingArchives" size="small">
            <el-table-column prop="name" label="文件名" min-width="200" />
            <el-table-column label="大小" width="120">
              <template #default="{ row }">{{ formatBytes(row.size) }}</template>
            </el-table-column>
            <el-table-column label="修改时间" width="180">
              <template #default="{ row }">{{ formatTime(row.mtime) }}</template>
            </el-table-column>
            <el-table-column label="操作" width="180">
              <template #default="{ row }">
                <el-button text type="primary" @click="onRestore(row)">还原</el-button>
                <el-button text type="danger" @click="onDeleteArchive(row)">删除</el-button>
              </template>
            </el-table-column>
          </el-table>
        </el-card>

        <!-- 备份历史 -->
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>备份历史</span>
              <el-button text type="primary" @click="loadRecords">刷新</el-button>
            </div>
          </template>
          <el-table :data="records" v-loading="loadingRecords" size="small">
            <el-table-column prop="name" label="名称" min-width="160" />
            <el-table-column prop="kind" label="类型" width="80" />
            <el-table-column label="状态" width="90">
              <template #default="{ row }">
                <el-tag size="small" :type="row.status === 1 ? 'success' : (row.status === -1 ? 'danger' : 'info')">
                  {{ row.status === 1 ? '成功' : (row.status === -1 ? '失败' : '进行中') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="大小" width="100">
              <template #default="{ row }">{{ formatBytes(row.size) }}</template>
            </el-table-column>
            <el-table-column label="时间" width="180">
              <template #default="{ row }">{{ formatTime(row.created_at) }}</template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-tab-pane>

      <el-tab-pane label="备份任务" name="jobs">
        <div class="card-head">
          <span>备份任务</span>
          <el-button type="primary" @click="openJobDialog(null)">新建任务</el-button>
        </div>
        <el-table :data="jobs" v-loading="loadingJobs" size="small">
          <el-table-column prop="name" label="名称" min-width="160" />
          <el-table-column prop="target_type" label="类型" width="80" />
          <el-table-column prop="schedule" label="计划(cron)" width="140" />
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
      </el-tab-pane>

      <el-tab-pane label="备份策略" name="policy">
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>备份策略</span>
              <el-button text type="primary" @click="loadPolicy">刷新</el-button>
            </div>
          </template>
          <el-form label-width="140px" class="form">
            <el-form-item label="允许用户自助备份">
              <el-switch v-model="policy.allow_user_backup" />
              <span class="tip">关闭后，普通用户无法自助备份自己的站点 / 数据库</span>
            </el-form-item>
            <el-form-item label="全局保留份数">
              <el-input-number v-model="policy.global_retain" :min="0" :max="999" />
              <span class="tip">用户未单独设置时的默认保留份数</span>
            </el-form-item>
            <el-form-item label="启用全量备份">
              <el-switch v-model="policy.all_enabled" />
              <span class="tip">开启后按下方计划自动备份全部用户数据（按主人打标，用户可从系统目录还原）</span>
            </el-form-item>
            <el-form-item label="全量备份计划(cron)">
              <el-input v-model="policy.all_schedule" placeholder="如 0 4 * * *（每天 4 点）" style="max-width:240px" />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingPolicy" @click="savePolicy">保存策略</el-button>
              <el-button :loading="runningFull" @click="runFull">立即全量备份</el-button>
            </el-form-item>
          </el-form>
        </el-card>
      </el-tab-pane>
    </el-tabs>

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
          </el-form-item>
          <el-form-item v-if="jobForm.engine === 'mysql'" label="用户名">
            <el-input v-model="jobForm.dbUser" />
          </el-form-item>
          <el-form-item v-if="jobForm.engine === 'mysql'" label="密码">
            <el-input v-model="jobForm.dbPass" type="password" show-password />
          </el-form-item>
          <el-form-item v-if="jobForm.engine === 'mysql'" label="主机">
            <el-input v-model="jobForm.dbHost" placeholder="127.0.0.1" />
          </el-form-item>
          <el-form-item v-if="jobForm.engine === 'mysql'" label="端口">
            <el-input v-model="jobForm.dbPort" placeholder="3306" />
          </el-form-item>
          <el-form-item v-if="jobForm.engine === 'sqlite'" label="数据库文件">
            <el-input v-model="jobForm.dbPath" />
          </el-form-item>
        </template>

        <el-form-item label="计划(cron)">
          <el-input v-model="jobForm.schedule" placeholder="如 0 3 * * *（每天 3 点）" />
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

    <!-- 还原对话框 -->
    <el-dialog v-model="restoreDialog" title="还原" width="520px">
      <el-form :model="restoreForm" label-width="96px">
        <el-form-item label="数据库类型">
          <el-select v-model="restoreForm.engine" style="width:160px" :disabled="restoreForm.kind === 'dir'">
            <el-option label="MySQL / MariaDB" value="mysql" />
            <el-option label="SQLite" value="sqlite" />
          </el-select>
        </el-form-item>
        <el-form-item label="目标数据库">
          <el-input v-model="restoreForm.dbName" :disabled="restoreForm.kind === 'dir'" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" label="用户名">
          <el-input v-model="restoreForm.dbUser" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" label="密码">
          <el-input v-model="restoreForm.dbPass" type="password" show-password />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" label="主机">
          <el-input v-model="restoreForm.dbHost" placeholder="127.0.0.1" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" label="端口">
          <el-input v-model="restoreForm.dbPort" placeholder="3306" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'sqlite'" label="数据库文件">
          <el-input v-model="restoreForm.dbPath" placeholder="如 /usr/local/zap/data/zap.db" />
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir'" label="解包目标目录">
          <el-input v-model="restoreForm.targetDir" placeholder="如 /home/u/www/restore" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="restoreDialog = false">取消</el-button>
        <el-button type="primary" :loading="restoring" @click="onConfirmRestore">还原</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { http } from '@/utils/request'

const tab = ref('archives')
const running = ref(false)
const savingSetting = ref(false)
const setting = ref({ path: '', disk_free: 0, disk_total: 0 })
const policy = ref({ allow_user_backup: true, global_retain: 7, all_enabled: false, all_schedule: '' })
const savingPolicy = ref(false)
const runningFull = ref(false)
const diskPercent = computed(() => {
  const t = setting.value.disk_total
  if (!t) return 0
  const used = t - setting.value.disk_free
  return Math.max(0, Math.min(100, Math.round((used / t) * 100)))
})

async function loadSetting() {
  try {
    const d: any = await http.get('/system/backup/setting')
    setting.value = d.data || { path: '', disk_free: 0, disk_total: 0 }
  } catch (e: any) {
    ElMessage.error(e?.message || '加载失败')
  }
}
async function saveSetting() {
  savingSetting.value = true
  try {
    await http.post('/system/backup/setting', { path: setting.value.path.trim() })
    ElMessage.success('已保存，后续备份将写入新目录')
    loadSetting()
    loadArchives()
  } catch (e: any) {
    ElMessage.error(e?.message || '保存失败')
  } finally {
    savingSetting.value = false
  }
}
const loadingArchives = ref(false)
const loadingRecords = ref(false)
const loadingJobs = ref(false)
const archives = ref<any[]>([])
const records = ref<any[]>([])
const jobs = ref<any[]>([])

const form = ref({
  kind: 'dir',
  name: '',
  pathsText: '',
  engine: 'mysql',
  dbName: '',
  dbUser: '',
  dbPass: '',
  dbHost: '',
  dbPort: '',
  dbPath: '',
})

// ── 备份策略 ──
async function loadPolicy() {
  try {
    const d: any = await http.get('/system/backup/policy')
    policy.value = {
      allow_user_backup: d.data?.allow_user_backup !== false,
      global_retain: d.data?.global_retain ?? 7,
      all_enabled: d.data?.all_enabled === true,
      all_schedule: d.data?.all_schedule || '',
    }
  } catch (e: any) {
    ElMessage.error(e?.message || '加载策略失败')
  }
}
async function savePolicy() {
  savingPolicy.value = true
  try {
    await http.post('/system/backup/policy', {
      allow_user: policy.value.allow_user_backup,
      global_retain: policy.value.global_retain,
      all_enabled: policy.value.all_enabled,
      all_schedule: policy.value.all_schedule.trim(),
    })
    ElMessage.success('策略已更新')
  } catch (e: any) {
    ElMessage.error(e?.message || '保存失败')
  } finally {
    savingPolicy.value = false
  }
}
async function runFull() {
  runningFull.value = true
  try {
    await http.post('/system/backup/all')
    ElMessage.success('已启动全量备份（后台执行）')
  } catch (e: any) {
    ElMessage.error(e?.message || '启动失败')
  } finally {
    runningFull.value = false
  }
}

// ── 归档 / 历史 / 任务 ──
async function loadArchives() {
  loadingArchives.value = true
  try {
    const d: any = await http.get('/system/backup/list')
    archives.value = (d.data?.items || []).filter((i: any) => /\.(tar\.gz|sql\.gz)$/.test(i.name))
  } catch (e: any) {
    ElMessage.error(e?.message || '加载失败')
  } finally {
    loadingArchives.value = false
  }
}
async function loadRecords() {
  loadingRecords.value = true
  try {
    const d: any = await http.get('/system/backup/records')
    records.value = d.data?.records || []
  } catch (e: any) {
    ElMessage.error(e?.message || '加载失败')
  } finally {
    loadingRecords.value = false
  }
}
async function loadJobs() {
  loadingJobs.value = true
  try {
    const d: any = await http.get('/system/backup/jobs')
    jobs.value = d.data?.jobs || []
  } catch (e: any) {
    ElMessage.error(e?.message || '加载失败')
  } finally {
    loadingJobs.value = false
  }
}

// ── 立即备份 ──
async function onManualBackup() {
  const f = form.value
  if (!f.name.trim()) return ElMessage.warning('请填写归档名')
  running.value = true
  try {
    if (f.kind === 'dir') {
      const paths = f.pathsText.split('\n').map((s) => s.trim()).filter(Boolean)
      if (!paths.length) return ElMessage.warning('请填写待备份路径')
      await http.post('/system/backup/create_dir', { name: f.name.trim(), paths })
      ElMessage.success('目录备份完成')
    } else {
      await http.post('/system/backup/create_db', {
        name: f.name.trim(),
        engine: f.engine,
        db_name: f.dbName,
        user: f.dbUser,
        password: f.dbPass,
        host: f.dbHost,
        port: parseInt(f.dbPort || '0', 10) || 0,
        db_path: f.engine === 'sqlite' ? f.dbPath : undefined,
      })
      ElMessage.success('数据库备份完成')
    }
    loadArchives()
    loadRecords()
  } catch (e: any) {
    ElMessage.error(e?.message || '备份失败')
  } finally {
    running.value = false
  }
}

// ── 删除归档 ──
async function onDeleteArchive(row: any) {
  try {
    await ElMessageBox.confirm(`确认删除 ${row.name}？`, '提示', { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.post('/system/backup/delete', { path: row.path })
    ElMessage.success('已删除')
    loadArchives()
  } catch (e: any) {
    ElMessage.error(e?.message || '删除失败')
  }
}

// ── 还原 ──
const restoreDialog = ref(false)
const restoring = ref(false)
const restoreForm = ref({ kind: 'dir', path: '', engine: 'mysql', dbName: '', dbUser: '', dbPass: '', dbHost: '', dbPort: '', dbPath: '', targetDir: '' })

function onRestore(row: any) {
  const kind = /\.sql\.gz$/.test(row.name) ? 'db' : 'dir'
  restoreForm.value = {
    kind,
    path: row.path,
    engine: 'mysql',
    dbName: '',
    dbUser: '',
    dbPass: '',
    dbHost: '127.0.0.1',
    dbPort: '3306',
    dbPath: '',
    targetDir: '',
  }
  restoreDialog.value = true
}
async function onConfirmRestore() {
  const r = restoreForm.value
  restoring.value = true
  try {
    if (r.kind === 'dir') {
      if (!r.targetDir.trim()) return ElMessage.warning('请填写解包目标目录')
      await http.post('/system/backup/restore_dir', { path: r.path, target_dir: r.targetDir.trim() })
    } else {
      if (!r.dbName.trim()) return ElMessage.warning('请填写目标数据库名')
      await http.post('/system/backup/restore_db', {
        path: r.path,
        engine: r.engine,
        db_name: r.dbName,
        user: r.dbUser,
        password: r.dbPass,
        host: r.dbHost,
        port: parseInt(r.dbPort || '0', 10) || 0,
        db_path: r.engine === 'sqlite' ? r.dbPath : undefined,
      })
    }
    ElMessage.success('还原完成')
    restoreDialog.value = false
  } catch (e: any) {
    ElMessage.error(e?.message || '还原失败')
  } finally {
    restoring.value = false
  }
}

// ── 任务 ──
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
    setTimeout(loadRecords, 1500)
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

// ── 格式化 ──
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

onMounted(() => {
  loadSetting()
  loadArchives()
  loadRecords()
  loadJobs()
  loadPolicy()
})
</script>

<style scoped>
.backup-wrap { padding: 12px; }
.block { margin-bottom: 16px; }
.card-head { display: flex; justify-content: space-between; align-items: center; }
.form { max-width: 560px; }
</style>
