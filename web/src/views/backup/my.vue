<template>
  <div class="my-backup-wrap">
    <!-- 个人保留份数 -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-head">
          <span>我的备份设置</span>
        </div>
      </template>
      <el-form label-width="110px" class="form">
        <el-form-item label="保留份数">
          <el-input-number v-model="retain" :min="1" :max="999" />
          <span class="tip">保留最近 N 份，超过自动清理（个人设置优先于全局）</span>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="savingRetain" @click="saveRetain">保存设置</el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <!-- 我的备份归档 -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-head">
          <span>我的备份</span>
          <el-button text type="primary" @click="loadList">刷新</el-button>
        </div>
      </template>
      <el-table :data="records" v-loading="loading" size="small" empty-text="暂无备份">
        <el-table-column prop="name" label="名称" min-width="180" />
        <el-table-column prop="kind" label="类型" width="80">
          <template #default="{ row }">{{ row.kind === 'db' ? '数据库' : '目录' }}</template>
        </el-table-column>
        <el-table-column label="大小" width="110">
          <template #default="{ row }">{{ formatBytes(row.size) }}</template>
        </el-table-column>
        <el-table-column label="存储位置" width="110">
          <template #default="{ row }">
            <el-tag size="small" :type="row.dest === 'home' ? 'success' : 'info'">
              {{ row.dest === 'home' ? '我的目录' : '系统目录' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="状态" width="90">
          <template #default="{ row }">
            <el-tag size="small" :type="row.status === 1 ? 'success' : (row.status === -1 ? 'danger' : 'info')">
              {{ row.status === 1 ? '成功' : (row.status === -1 ? '失败' : '进行中') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="时间" width="170">
          <template #default="{ row }">{{ formatTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column label="操作" width="160">
          <template #default="{ row }">
            <el-button text type="primary" @click="onRestore(row)">还原</el-button>
            <el-button text type="danger" @click="onDelete(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="tip" style="margin-top:8px">
        存储位置为「我的目录」= 备份在你家目录 <code>/home/你/backups</code>；
        「系统目录」= 管理员全量备份生成的副本，两者你都可以还原。
      </div>
    </el-card>

    <!-- 还原对话框 -->
    <el-dialog v-model="restoreDialog" title="还原" width="520px">
      <el-form :model="restoreForm" label-width="110px">
        <el-alert
          type="info"
          :closable="false"
          :title="restoreForm.dest === 'home'
            ? '将从你的家目录备份还原'
            : '将从服务器系统备份目录还原（管理员全量备份的副本）'"
          style="margin-bottom: 12px"
        />
        <el-form-item v-if="restoreForm.kind === 'db'" label="数据库类型">
          <el-select v-model="restoreForm.engine" style="width:160px">
            <el-option label="MySQL / MariaDB" value="mysql" />
            <el-option label="SQLite" value="sqlite" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'db'" label="目标数据库">
          <el-input v-model="restoreForm.dbName" placeholder="要还原到的数据库名" />
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir'" label="解包目标目录">
          <el-input v-model="restoreForm.targetDir" placeholder="如 /home/u/www/restore" />
        </el-form-item>
        <el-form-item>
          <span class="tip">数据库还原使用存储凭据，无需输入密码。</span>
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
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { http } from '@/utils/request'

const loading = ref(false)
const records = ref<any[]>([])
const retain = ref(7)
const savingRetain = ref(false)

const restoreDialog = ref(false)
const restoring = ref(false)
const restoreForm = ref({
  kind: 'dir',
  path: '',
  dest: 'system',
  engine: 'mysql',
  dbName: '',
  targetDir: '',
})

async function loadList() {
  loading.value = true
  try {
    const d: any = await http.get('/system/backup/my')
    records.value = d.data?.records || []
  } catch (e: any) {
    ElMessage.error(e?.message || '加载失败')
  } finally {
    loading.value = false
  }
}

async function loadRetention() {
  try {
    const d: any = await http.get('/system/backup/my-retention')
    retain.value = d.data?.retain || 7
  } catch (e: any) {
    ElMessage.error(e?.message || '加载保留设置失败')
  }
}

async function saveRetain() {
  savingRetain.value = true
  try {
    await http.post('/system/backup/my-retention', { retain: retain.value })
    ElMessage.success('已保存')
  } catch (e: any) {
    ElMessage.error(e?.message || '保存失败')
  } finally {
    savingRetain.value = false
  }
}

function onRestore(row: any) {
  restoreForm.value = {
    kind: row.kind,
    path: row.path,
    dest: row.dest,
    engine: 'mysql',
    dbName: '',
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

async function onDelete(row: any) {
  try {
    await ElMessageBox.confirm(`确认删除 ${row.name}？`, '提示', { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.post('/system/backup/delete', { path: row.path })
    ElMessage.success('已删除')
    loadList()
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

onMounted(() => {
  loadList()
  loadRetention()
})
</script>

<style scoped>
.my-backup-wrap { padding: 12px; }
.block { margin-bottom: 16px; }
.card-head { display: flex; justify-content: space-between; align-items: center; }
.form { max-width: 560px; }
.tip { color: #909399; font-size: 12px; }
</style>
