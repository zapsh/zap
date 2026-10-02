<template>
  <div class="backup-settings">
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
          <div class="dp-row">
            <el-input v-model="setting.path" placeholder="如 /data/backups（绝对路径，留空=默认 {ZAP_PATH}/data/backup）" readonly />
            <el-button @click="dirPickerStore = true">选择目录</el-button>
          </div>
          <DirPicker v-model="dirPickerStore" :start-path="setting.path" confirm-text="选此目录" @confirm="(p) => setting.path = p" />
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
      <el-tab-pane v-if="isAdmin" label="备份清单" name="archives">
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
              <el-collapse class="more-collapse">
                <el-collapse-item title="更多连接信息（留空则用本机 zapadm 直连，填写后备份远程数据库）">
                  <el-form-item label="用户名">
                    <el-input v-model="form.dbUser" placeholder="留空 = zapadm" />
                  </el-form-item>
                  <el-form-item label="密码">
                    <el-input v-model="form.dbPass" type="password" show-password placeholder="留空 = 面板凭据" />
                  </el-form-item>
                  <el-form-item label="主机">
                    <el-input v-model="form.dbHost" placeholder="127.0.0.1" />
                  </el-form-item>
                  <el-form-item label="端口">
                    <el-input v-model="form.dbPort" placeholder="3306" />
                  </el-form-item>
                  <el-form-item v-if="form.engine === 'sqlite'" label="数据库文件">
                    <el-input v-model="form.dbPath" placeholder="如 /usr/local/zap/data/zap.db" />
                  </el-form-item>
                </el-collapse-item>
              </el-collapse>
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

      <el-tab-pane v-if="isAdmin" label="备份策略" name="policy">
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
            <el-form-item label="允许用户定时备份">
              <el-switch v-model="policy.allow_user_job" />
              <span class="tip">关闭后，普通用户无法创建 / 修改定时备份任务（已有任务对其不可见、不可执行）</span>
            </el-form-item>
            <el-form-item label="全量备份模式">
              <el-radio-group v-model="policy.mode">
                <el-radio value="home">按家目录</el-radio>
                <el-radio value="site">按站点+应用+库</el-radio>
              </el-radio-group>
              <span class="tip">home=遍历用户家目录；site=遍历站点文档根+应用工作目录+库（both 模式后端保留但暂不开放）</span>
            </el-form-item>
            <el-form-item label="落盘位置">
              <el-radio-group v-model="policy.dest">
                <el-radio value="home">用户家目录 backups</el-radio>
                <el-radio value="system">系统备份目录</el-radio>
              </el-radio-group>
              <span class="tip">home=各用户 &lt;home&gt;/backups（自己管理/还原）；system=集中系统目录</span>
            </el-form-item>
            <el-form-item label="默认排除列表">
              <el-input v-model="policy.exclude_default" type="textarea" :rows="4"
                placeholder="每行一个模式，如 node_modules、.cache、*.log" style="max-width:480px" />
              <span class="tip">管理员通用排除（家目录备份内置含 backups/.zap 防自我递归）；用户可在 &lt;home&gt;/.zap/backup_exclude.txt 追加</span>
            </el-form-item>
            <el-form-item label="全局保留份数">
              <el-input-number v-model="policy.global_retain" :min="0" :max="999" />
              <span class="tip">用户未单独设置时的默认保留份数</span>
            </el-form-item>
            <el-form-item label="启用全量备份">
              <el-switch v-model="policy.all_enabled" />
              <span class="tip">开启后按下方计划自动备份全部用户数据（按主人打标，用户可从系统目录还原）</span>
            </el-form-item>
            <el-form-item label="全量备份计划">
              <el-select
                v-model="policy.all_schedule"
                filterable
                allow-create
                default-first-option
                clearable
                placeholder="留空 = 不自动备份"
                style="max-width: 260px"
              >
                <el-option label="每天 03:00" value="0 3 * * *" />
                <el-option label="每天 04:00" value="0 4 * * *" />
                <el-option label="每周一 03:00" value="0 3 * * 1" />
                <el-option label="每月 1 号 03:00" value="0 3 1 * *" />
                <el-option label="每小时" value="0 * * * *" />
              </el-select>
              <span class="tip">当前执行：{{ describeCron(policy.all_schedule) }}（留空 = 不自动备份；标准 5 段 cron，可直接输入自定义表达式）</span>
            </el-form-item>
            <el-form-item label="上次全量结果" v-if="policy.last_report">
              <span class="tip">
                成功 {{ policy.last_report.ok }} / 失败 {{ policy.last_report.fail }}
                <template v-if="policy.last_report.finished_at">（{{ formatTime(policy.last_report.finished_at) }}）</template>
                <span v-if="policy.last_report.fail" style="color:#f56c6c">，详见审计日志</span>
              </span>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingPolicy" @click="savePolicy">保存策略</el-button>
              <el-button :loading="runningFull" @click="runFull">立即全量备份</el-button>
            </el-form-item>
          </el-form>
        </el-card>

        <!-- 额外备份目录（管理员） -->
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>额外备份目录</span>
              <el-button text type="primary" @click="loadPaths">刷新</el-button>
            </div>
          </template>
          <el-table :data="paths" v-loading="loadingPaths" size="small" empty-text="暂无额外目录">
            <el-table-column prop="owner_type" label="类型" width="90">
              <template #default="{ row }">{{ row.owner_type === 'site' ? '站点' : '用户' }}</template>
            </el-table-column>
            <el-table-column prop="owner_id" label="归属ID" width="90" />
            <el-table-column prop="path" label="目录" min-width="220" />
            <el-table-column prop="note" label="备注" min-width="120" />
            <el-table-column label="操作" width="100">
              <template #default="{ row }">
                <el-button text type="danger" @click="onDeletePath(row)">删除</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-form :inline="true" class="form" style="margin-top:12px">
            <el-form-item label="类型">
              <el-select v-model="pathForm.owner_type" style="width:110px">
                <el-option label="用户" value="user" />
                <el-option label="站点" value="site" />
              </el-select>
            </el-form-item>
            <el-form-item label="归属ID">
              <el-input v-model="pathForm.owner_id" type="number" style="width:110px" />
            </el-form-item>
            <el-form-item label="目录">
              <div class="dp-row">
                <el-input v-model="pathForm.path" placeholder="绝对路径" readonly style="width:260px" />
                <el-button @click="dirPickerExtra = true">选择目录</el-button>
              </div>
              <DirPicker v-model="dirPickerExtra" :start-path="pathForm.path" confirm-text="选此目录" @confirm="(p) => pathForm.path = p" />
            </el-form-item>
            <el-form-item label="备注">
              <el-input v-model="pathForm.note" style="width:160px" />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingPath" @click="onAddPath">添加</el-button>
            </el-form-item>
          </el-form>
          <div class="tip" style="margin-top:8px">
            家目录之外的目录只能由管理员在此添加：普通用户仅备份各自家目录（策略二整屋打包），站点附加目录也在此维护。
          </div>
        </el-card>
      </el-tab-pane>
    </el-tabs>

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
        <el-form-item v-if="restoreForm.kind === 'dir'" label="还原方式">
          <el-radio-group v-model="restoreForm.toOriginal">
            <el-radio :value="false">解包到指定目录</el-radio>
            <el-radio :value="true">还原到原路径</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir' && !restoreForm.toOriginal" label="解包目标目录">
          <el-input v-model="restoreForm.targetDir" placeholder="如 /home/u/www/restore" />
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir' && restoreForm.toOriginal">
          <span class="tip">将按归档内路径直接写回原绝对位置（需管理员或该备份所属用户）。</span>
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
import DirPicker from '@/components/DirPicker.vue'
import { useUserStore } from '@/stores/user'

const userStore = useUserStore()
const isAdmin = computed(() => (userStore.roles || []).includes('admin'))

const tab = ref('archives')
const dirPickerStore = ref(false)
const dirPickerExtra = ref(false)
const running = ref(false)
const savingSetting = ref(false)
const setting = ref({ path: '', disk_free: 0, disk_total: 0 })
const policy = ref<any>({ allow_user_backup: true, allow_user_job: true, global_retain: 7, all_enabled: false, all_schedule: '', mode: 'home', dest: 'system', exclude_default: '', last_report: null })
/** 全量备份计划：直接绑定 policy.all_schedule，可在下方 select 选预设或手输 cron */
const savingPolicy = ref(false)
const runningFull = ref(false)
const loadingPaths = ref(false)
const savingPath = ref(false)
const paths = ref<any[]>([])
const pathForm = ref({ owner_type: 'user', owner_id: 0, path: '', note: '' })
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
const archives = ref<any[]>([])
const records = ref<any[]>([])

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
      allow_user_job: d.data?.allow_user_job !== false,
      global_retain: d.data?.global_retain ?? 7,
      all_enabled: d.data?.all_enabled === true,
      all_schedule: d.data?.all_schedule || '',
      mode: d.data?.mode || 'home',
      dest: d.data?.dest || 'system',
      exclude_default: d.data?.exclude_default || '',
      last_report: d.data?.last_report ?? null,
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
      allow_user_job: policy.value.allow_user_job,
      global_retain: policy.value.global_retain,
      all_enabled: policy.value.all_enabled,
      all_schedule: policy.value.all_schedule.trim(),
      mode: policy.value.mode,
      dest: policy.value.dest,
      exclude_default: policy.value.exclude_default,
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

// ── 额外备份目录（管理员）──
async function loadPaths() {
  loadingPaths.value = true
  try {
    const d: any = await http.get('/system/backup/paths')
    paths.value = d.data?.items || []
  } catch (e: any) {
    ElMessage.error(e?.message || '加载失败')
  } finally {
    loadingPaths.value = false
  }
}
async function onAddPath() {
  const f = pathForm.value
  if (!f.path.trim()) return ElMessage.warning('请填写目录')
  if (!['site', 'user'].includes(f.owner_type)) return ElMessage.warning('类型有误')
  savingPath.value = true
  try {
    await http.post('/system/backup/paths', {
      owner_type: f.owner_type,
      owner_id: Number(f.owner_id) || 0,
      path: f.path.trim(),
      note: f.note.trim(),
    })
    ElMessage.success('已添加')
    pathForm.value = { owner_type: 'user', owner_id: 0, path: '', note: '' }
    loadPaths()
  } catch (e: any) {
    ElMessage.error(e?.message || '添加失败')
  } finally {
    savingPath.value = false
  }
}
async function onDeletePath(row: any) {
  try {
    await ElMessageBox.confirm(`确认删除 ${row.path}？`, '提示', { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.delete('/system/backup/paths', { data: { id: row.id } })
    ElMessage.success('已删除')
    loadPaths()
  } catch (e: any) {
    ElMessage.error(e?.message || '删除失败')
  }
}

// ── 归档 / 历史 ──
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
const restoreForm = ref({ kind: 'dir', path: '', engine: 'mysql', dbName: '', dbUser: '', dbPass: '', dbHost: '', dbPort: '', dbPath: '', targetDir: '', toOriginal: false })

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
    toOriginal: false,
  }
  restoreDialog.value = true
}
async function onConfirmRestore() {
  const r = restoreForm.value
  restoring.value = true
  try {
    if (r.kind === 'dir') {
      if (r.toOriginal) {
        await http.post('/system/backup/restore_dir', { path: r.path, to_original: true })
      } else {
        if (!r.targetDir.trim()) return ElMessage.warning('请填写解包目标目录')
        await http.post('/system/backup/restore_dir', { path: r.path, target_dir: r.targetDir.trim() })
      }
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

/** cron 表达式 -> 人类可读说明（命中预设则用中文，未命中回退原始表达式） */
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
  // 以下均为管理员专属接口，普通用户不调用以免 403
  if (isAdmin.value) {
    loadSetting()
    loadArchives()
    loadRecords()
    loadPolicy()
    loadPaths()
  }
})
</script>

<style scoped>
.backup-settings { padding: 12px; }
.block { margin-bottom: 16px; }
.card-head { display: flex; justify-content: space-between; align-items: center; }
.form { max-width: 560px; }
.muted { color: var(--el-text-color-secondary); font-size: 12px; }
.dp-row { display: flex; align-items: center; gap: 8px; }
</style>
