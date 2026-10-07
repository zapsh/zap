<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="my-backup-wrap">
    <!-- 个人保留份数 -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-head">
          <span>{{ t('backup.mySettings') }}</span>
        </div>
      </template>
      <el-form label-width="110px" class="form">
        <el-form-item :label="t('backup.retain')">
          <el-input-number v-model="retain" :min="1" :max="999" />
          <span class="tip">{{ t('backup.retainTip') }}</span>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="savingRetain" @click="saveRetain">{{ t('backup.save') }}</el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <!-- 我的备份归档 -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-head">
          <span>{{ t('backup.myBackups') }}</span>
          <el-button text type="primary" @click="loadList">{{ t('backup.refresh') }}</el-button>
        </div>
      </template>
      <el-table :data="records" v-loading="loading" size="small" :empty-text="t('backup.myEmpty')">
        <el-table-column prop="name" :label="t('backup.colName')" min-width="180" />
        <el-table-column prop="kind" :label="t('backup.colKind')" width="80">
          <template #default="{ row }">{{ row.kind === 'db' ? t('backup.kindDb') : t('backup.kindDir') }}</template>
        </el-table-column>
        <el-table-column :label="t('backup.colSize')" width="110">
          <template #default="{ row }">{{ formatBytes(row.size) }}</template>
        </el-table-column>
        <el-table-column :label="t('backup.colDest')" width="110">
          <template #default="{ row }">
            <el-tag size="small" :type="row.dest === 'home' ? 'success' : 'info'">
              {{ row.dest === 'home' ? t('backup.destHome') : t('backup.destSystem') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('backup.colStatus')" width="90">
          <template #default="{ row }">
            <el-tag size="small" :type="row.status === 1 ? 'success' : (row.status === -1 ? 'danger' : 'info')">
              {{ row.status === 1 ? t('backup.statusOk') : (row.status === -1 ? t('backup.statusFail') : t('backup.statusRunning')) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('backup.colTime')" width="170">
          <template #default="{ row }">{{ formatTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('backup.colAction')" width="160">
          <template #default="{ row }">
            <el-button text type="primary" @click="onRestore(row)">{{ t('backup.restore') }}</el-button>
            <el-button text type="danger" @click="onDelete(row)">{{ t('backup.delete') }}</el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="tip" style="margin-top:8px">
        {{ t('backup.destTip') }}
      </div>
    </el-card>

    <!-- 还原对话框 -->
    <el-dialog v-model="restoreDialog" :title="t('backup.restoreTitle')" width="520px">
      <el-form :model="restoreForm" label-width="110px">
        <el-alert
          type="info"
          :closable="false"
          :title="restoreForm.dest === 'home' ? t('backup.restoreFromHome') : t('backup.restoreFromSystem')"
          style="margin-bottom: 12px"
        />
        <el-form-item v-if="restoreForm.kind === 'db'" :label="t('backup.engineLabel')">
          <el-select v-model="restoreForm.engine" style="width:160px">
            <el-option label="MySQL / MariaDB" value="mysql" />
            <el-option label="SQLite" value="sqlite" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'db'" :label="t('backup.targetDb')">
          <el-input v-model="restoreForm.dbName" :placeholder="t('backup.targetDbPh')" />
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir'" :label="t('backup.restoreMode')">
          <el-radio-group v-model="restoreForm.toOriginal">
            <el-radio :value="false">{{ t('backup.unpackToDir') }}</el-radio>
            <el-radio :value="true">{{ t('backup.restoreToOriginal') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir' && !restoreForm.toOriginal" :label="t('backup.unpackTargetDir')">
          <el-input v-model="restoreForm.targetDir" :placeholder="t('backup.unpackTargetDirPh')" />
        </el-form-item>
        <el-form-item v-if="restoreForm.kind === 'dir' && restoreForm.toOriginal">
          <span class="tip">{{ t('backup.toOriginalTip') }}</span>
        </el-form-item>
        <el-form-item>
          <span class="tip">{{ t('backup.dbNoPasswordTip') }}</span>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="restoreDialog = false">{{ t('backup.cancel') }}</el-button>
        <el-button type="primary" :loading="restoring" @click="onConfirmRestore">{{ t('backup.restore') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { http } from '@/utils/request'

const { t } = useI18n()

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
  toOriginal: false,
})

async function loadList() {
  loading.value = true
  try {
    const d: any = await http.get('/system/backup/my')
    records.value = d.data?.records || []
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function loadRetention() {
  try {
    const d: any = await http.get('/system/backup/my-retention')
    retain.value = d.data?.retain || 7
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.retentionLoadFailed'))
  }
}

async function saveRetain() {
  savingRetain.value = true
  try {
    await http.post('/system/backup/my-retention', { retain: retain.value })
    ElMessage.success(t('backup.saved'))
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.saveFailed'))
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
        if (!r.targetDir.trim()) return ElMessage.warning(t('backup.targetDirRequired'))
        await http.post('/system/backup/restore_dir', { path: r.path, target_dir: r.targetDir.trim() })
      }
    } else {
      if (!r.dbName.trim()) return ElMessage.warning(t('backup.dbNameRequired'))
      await http.post('/system/backup/restore_db', {
        path: r.path,
        engine: r.engine,
        db_name: r.dbName,
      })
    }
    ElMessage.success(t('backup.restoreDone'))
    restoreDialog.value = false
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.restoreFailed'))
  } finally {
    restoring.value = false
  }
}

async function onDelete(row: any) {
  try {
    await ElMessageBox.confirm(t('backup.deleteConfirm', { name: row.name }), t('backup.notice'), { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.post('/system/backup/delete', { path: row.path })
    ElMessage.success(t('backup.deleted'))
    loadList()
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
