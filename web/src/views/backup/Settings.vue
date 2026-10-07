<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="backup-settings">
    <!-- 备份存储目录（可改到其他磁盘 / 挂载点，缓解系统盘空间不足） -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-head">
          <span>{{ t('backup.storageCard') }}</span>
          <el-button text type="primary" @click="loadSetting">{{ t('backup.refresh') }}</el-button>
        </div>
      </template>
      <el-form label-width="96px" class="form">
        <el-form-item :label="t('backup.storageDirLabel')">
          <div class="dp-row">
            <el-input v-model="setting.path" :placeholder="t('backup.storageDirPh')" readonly />
            <el-button @click="dirPickerStore = true">{{ t('backup.pickDir') }}</el-button>
          </div>
          <DirPicker v-model="dirPickerStore" :start-path="setting.path" :confirm-text="t('backup.pickDirConfirm')" @confirm="(p) => setting.path = p" />
        </el-form-item>
        <el-form-item :label="t('backup.diskUsage')">
          <el-progress
            :percentage="diskPercent"
            :status="diskPercent > 90 ? 'exception' : ''"
            :stroke-width="14"
            style="width: 100%"
          />
          <span class="disk-tip">
            {{ t('backup.diskTip', { free: formatBytes(setting.disk_free), total: formatBytes(setting.disk_total) }) }}
            <span v-if="diskPercent > 90" style="color:#f56c6c">{{ t('backup.diskTight') }}</span>
          </span>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="savingSetting" @click="saveSetting">{{ t('backup.saveDir') }}</el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <el-tabs v-model="tab">
      <el-tab-pane v-if="isAdmin" :label="t('backup.tabArchives')" name="archives">
        <!-- 立即备份 -->
        <el-card shadow="never" class="block">
          <template #header>
            <span>{{ t('backup.backupNow') }}</span>
          </template>
          <el-form :model="form" label-width="90px" :inline="false" class="form">
            <el-form-item :label="t('backup.kindLabel')">
              <el-radio-group v-model="form.kind">
                <el-radio value="dir">{{ t('backup.kindDirFile') }}</el-radio>
                <el-radio value="db">{{ t('backup.kindDb') }}</el-radio>
              </el-radio-group>
            </el-form-item>

            <template v-if="form.kind === 'dir'">
              <el-form-item :label="t('backup.archiveName')">
                <el-input v-model="form.name" :placeholder="t('backup.archiveNamePh')" />
              </el-form-item>
              <el-form-item :label="t('backup.pathsLabel')">
                <el-input v-model="form.pathsText" type="textarea" :rows="3"
                  :placeholder="t('backup.pathsPh')" />
              </el-form-item>
            </template>

            <template v-else>
              <el-form-item :label="t('backup.engineLabel')">
                <el-select v-model="form.engine" style="width:160px">
                  <el-option label="MySQL / MariaDB" value="mysql" />
                  <el-option label="SQLite" value="sqlite" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('backup.archiveName')">
                <el-input v-model="form.name" :placeholder="t('backup.archiveNameDbPh')" />
              </el-form-item>
              <el-form-item :label="t('backup.dbNameCol')">
                <el-input v-model="form.dbName" :placeholder="t('backup.dbNamePh')" />
              </el-form-item>
              <el-collapse class="more-collapse">
                <el-collapse-item :title="t('backup.moreConn')">
                  <el-form-item :label="t('backup.dbUser')">
                    <el-input v-model="form.dbUser" :placeholder="t('backup.dbUserPh')" />
                  </el-form-item>
                  <el-form-item :label="t('backup.dbPass')">
                    <el-input v-model="form.dbPass" type="password" show-password :placeholder="t('backup.dbPassPh')" />
                  </el-form-item>
                  <el-form-item :label="t('backup.host')">
                    <el-input v-model="form.dbHost" placeholder="127.0.0.1" />
                  </el-form-item>
                  <el-form-item :label="t('backup.port')">
                    <el-input v-model="form.dbPort" placeholder="3306" />
                  </el-form-item>
                  <el-form-item v-if="form.engine === 'sqlite'" :label="t('backup.dbFile')">
                    <el-input v-model="form.dbPath" :placeholder="t('backup.dbFilePh')" />
                  </el-form-item>
                </el-collapse-item>
              </el-collapse>
            </template>

            <el-form-item>
              <el-button type="primary" :loading="running" @click="onManualBackup">
                {{ t('backup.backupNow') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-card>

        <!-- 备份归档 -->
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>{{ t('backup.archivesCard') }}</span>
              <el-button text type="primary" @click="loadArchives">{{ t('backup.refresh') }}</el-button>
            </div>
          </template>
          <el-table :data="archives" v-loading="loadingArchives" size="small">
            <el-table-column prop="name" :label="t('backup.colFile')" min-width="200" />
            <el-table-column :label="t('backup.colSize')" width="120">
              <template #default="{ row }">{{ formatBytes(row.size) }}</template>
            </el-table-column>
            <el-table-column :label="t('backup.colMtime')" width="180">
              <template #default="{ row }">{{ formatTime(row.mtime) }}</template>
            </el-table-column>
            <el-table-column :label="t('backup.colAction')" width="180">
              <template #default="{ row }">
                <el-button text type="primary" @click="onRestore(row)">{{ t('backup.restore') }}</el-button>
                <el-button text type="danger" @click="onDeleteArchive(row)">{{ t('backup.delete') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
        </el-card>

        <!-- 备份历史 -->
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>{{ t('backup.historyCard') }}</span>
              <el-button text type="primary" @click="loadRecords">{{ t('backup.refresh') }}</el-button>
            </div>
          </template>
          <el-table :data="records" v-loading="loadingRecords" size="small">
            <el-table-column prop="name" :label="t('backup.colName')" min-width="160" />
            <el-table-column prop="kind" :label="t('backup.colKind')" width="80" />
            <el-table-column :label="t('backup.colStatus')" width="90">
              <template #default="{ row }">
                <el-tag size="small" :type="row.status === 1 ? 'success' : (row.status === -1 ? 'danger' : 'info')">
                  {{ row.status === 1 ? t('backup.statusOk') : (row.status === -1 ? t('backup.statusFail') : t('backup.statusRunning')) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('backup.colSize')" width="100">
              <template #default="{ row }">{{ formatBytes(row.size) }}</template>
            </el-table-column>
            <el-table-column :label="t('backup.colTime')" width="180">
              <template #default="{ row }">{{ formatTime(row.created_at) }}</template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-tab-pane>

      <el-tab-pane v-if="isAdmin" :label="t('backup.tabPolicy')" name="policy">
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>{{ t('backup.policyCard') }}</span>
              <el-button text type="primary" @click="loadPolicy">{{ t('backup.refresh') }}</el-button>
            </div>
          </template>
          <el-form label-width="140px" class="form">
            <el-form-item :label="t('backup.allowUserBackup')">
              <el-switch v-model="policy.allow_user_backup" />
              <span class="tip">{{ t('backup.allowUserBackupTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.allowUserJob')">
              <el-switch v-model="policy.allow_user_job" />
              <span class="tip">{{ t('backup.allowUserJobTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.fullMode')">
              <el-radio-group v-model="policy.mode">
                <el-radio value="home">{{ t('backup.modeHome') }}</el-radio>
                <el-radio value="site">{{ t('backup.modeSite') }}</el-radio>
              </el-radio-group>
              <span class="tip">{{ t('backup.modeTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.destLabel')">
              <el-radio-group v-model="policy.dest">
                <el-radio value="home">{{ t('backup.destHomeOption') }}</el-radio>
                <el-radio value="system">{{ t('backup.destSystemOption') }}</el-radio>
              </el-radio-group>
              <span class="tip">{{ t('backup.destOptionTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.excludeLabel')">
              <el-input v-model="policy.exclude_default" type="textarea" :rows="4"
                :placeholder="t('backup.excludePh')" style="max-width:480px" />
              <span class="tip">{{ t('backup.excludeTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.globalRetain')">
              <el-input-number v-model="policy.global_retain" :min="0" :max="999" />
              <span class="tip">{{ t('backup.globalRetainTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.allEnabled')">
              <el-switch v-model="policy.all_enabled" />
              <span class="tip">{{ t('backup.allEnabledTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.allSchedule')">
              <el-select
                v-model="policy.all_schedule"
                filterable
                allow-create
                default-first-option
                clearable
                :placeholder="t('backup.allSchedulePh')"
                style="max-width: 260px"
              >
                <el-option :label="t('backup.cronDaily3')" value="0 3 * * *" />
                <el-option :label="t('backup.cronDaily4')" value="0 4 * * *" />
                <el-option :label="t('backup.cronWeekly')" value="0 3 * * 1" />
                <el-option :label="t('backup.cronMonthly')" value="0 3 1 * *" />
                <el-option :label="t('backup.cronHourly')" value="0 * * * *" />
              </el-select>
              <span class="tip">{{ t('backup.allScheduleTip', { desc: describeCron(policy.all_schedule) }) }}</span>
            </el-form-item>
            <el-form-item :label="t('backup.lastReport')" v-if="policy.last_report">
              <span class="tip">
                {{ t('backup.lastReportTip', { ok: policy.last_report.ok, fail: policy.last_report.fail }) }}
                <template v-if="policy.last_report.finished_at">（{{ formatTime(policy.last_report.finished_at) }}）</template>
                <span v-if="policy.last_report.fail" style="color:#f56c6c">{{ t('backup.seeAudit') }}</span>
              </span>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingPolicy" @click="savePolicy">{{ t('backup.savePolicy') }}</el-button>
              <el-button :loading="runningFull" @click="runFull">{{ t('backup.runFull') }}</el-button>
            </el-form-item>
          </el-form>
        </el-card>

        <!-- 额外备份目录（管理员） -->
        <el-card shadow="never" class="block">
          <template #header>
            <div class="card-head">
              <span>{{ t('backup.extraDirs') }}</span>
              <el-button text type="primary" @click="loadPaths">{{ t('backup.refresh') }}</el-button>
            </div>
          </template>
          <el-table :data="paths" v-loading="loadingPaths" size="small" :empty-text="t('backup.noExtra')">
            <el-table-column prop="owner_type" :label="t('backup.colType')" width="90">
              <template #default="{ row }">{{ row.owner_type === 'site' ? t('backup.typeSite') : t('backup.typeUser') }}</template>
            </el-table-column>
            <el-table-column prop="owner_id" :label="t('backup.colOwnerId')" width="90" />
            <el-table-column prop="path" :label="t('backup.colPath')" min-width="220" />
            <el-table-column prop="note" :label="t('backup.colNote')" min-width="120" />
            <el-table-column :label="t('backup.colAction')" width="100">
              <template #default="{ row }">
                <el-button text type="danger" @click="onDeletePath(row)">{{ t('backup.delete') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-form :inline="true" class="form" style="margin-top:12px">
            <el-form-item :label="t('backup.colType')">
              <el-select v-model="pathForm.owner_type" style="width:110px">
                <el-option :label="t('backup.typeUser')" value="user" />
                <el-option :label="t('backup.typeSite')" value="site" />
              </el-select>
            </el-form-item>
            <el-form-item :label="t('backup.ownerLabel')">
              <el-input v-model="pathForm.owner_id" type="number" style="width:110px" />
            </el-form-item>
            <el-form-item :label="t('backup.colPath')">
              <div class="dp-row">
                <el-input v-model="pathForm.path" placeholder="/abs/path" readonly style="width:260px" />
                <el-button @click="dirPickerExtra = true">{{ t('backup.pickDir') }}</el-button>
              </div>
              <DirPicker v-model="dirPickerExtra" :start-path="pathForm.path" :confirm-text="t('backup.pickDirConfirm')" @confirm="(p) => pathForm.path = p" />
            </el-form-item>
            <el-form-item :label="t('backup.noteLabel')">
              <el-input v-model="pathForm.note" style="width:160px" />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingPath" @click="onAddPath">{{ t('backup.add') }}</el-button>
            </el-form-item>
          </el-form>
          <div class="tip" style="margin-top:8px">
            {{ t('backup.extraTip') }}
          </div>
        </el-card>
      </el-tab-pane>
    </el-tabs>

    <!-- 还原对话框 -->
    <el-dialog v-model="restoreDialog" :title="t('backup.restoreTitle')" width="520px">
      <el-form :model="restoreForm" label-width="96px">
        <el-form-item :label="t('backup.restoreDbEngine')">
          <el-select v-model="restoreForm.engine" style="width:160px" :disabled="restoreForm.kind === 'dir'">
            <el-option label="MySQL / MariaDB" value="mysql" />
            <el-option label="SQLite" value="sqlite" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('backup.restoreTargetDb')">
          <el-input v-model="restoreForm.dbName" :disabled="restoreForm.kind === 'dir'" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" :label="t('backup.restoreUser')">
          <el-input v-model="restoreForm.dbUser" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" :label="t('backup.restorePass')">
          <el-input v-model="restoreForm.dbPass" type="password" show-password />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" :label="t('backup.restoreHost')">
          <el-input v-model="restoreForm.dbHost" placeholder="127.0.0.1" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'mysql'" :label="t('backup.restorePort')">
          <el-input v-model="restoreForm.dbPort" placeholder="3306" />
        </el-form-item>
        <el-form-item v-if="restoreForm.engine === 'sqlite'" :label="t('backup.restoreDbFile')">
          <el-input v-model="restoreForm.dbPath" :placeholder="t('backup.dbFilePh')" />
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
      </el-form>
      <template #footer>
        <el-button @click="restoreDialog = false">{{ t('backup.cancel') }}</el-button>
        <el-button type="primary" :loading="restoring" @click="onConfirmRestore">{{ t('backup.restore') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { http } from '@/utils/request'
import DirPicker from '@/components/DirPicker.vue'
import { useUserStore } from '@/stores/user'

const { t } = useI18n()
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
  const t2 = setting.value.disk_total
  if (!t2) return 0
  const used = t2 - setting.value.disk_free
  return Math.max(0, Math.min(100, Math.round((used / t2) * 100)))
})

async function loadSetting() {
  try {
    const d: any = await http.get('/system/backup/setting')
    setting.value = d.data || { path: '', disk_free: 0, disk_total: 0 }
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.loadFailed'))
  }
}
async function saveSetting() {
  savingSetting.value = true
  try {
    await http.post('/system/backup/setting', { path: setting.value.path.trim() })
    ElMessage.success(t('backup.dirSaved'))
    loadSetting()
    loadArchives()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.saveFailed'))
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
    ElMessage.error(e?.message || t('backup.policyLoadFailed'))
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
    ElMessage.success(t('backup.policySaved'))
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.saveFailed'))
  } finally {
    savingPolicy.value = false
  }
}
async function runFull() {
  runningFull.value = true
  try {
    await http.post('/system/backup/all')
    ElMessage.success(t('backup.fullStarted'))
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.startFailed'))
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
    ElMessage.error(e?.message || t('backup.loadFailed'))
  } finally {
    loadingPaths.value = false
  }
}
async function onAddPath() {
  const f = pathForm.value
  if (!f.path.trim()) return ElMessage.warning(t('backup.dirRequired'))
  if (!['site', 'user'].includes(f.owner_type)) return ElMessage.warning(t('backup.typeInvalid'))
  savingPath.value = true
  try {
    await http.post('/system/backup/paths', {
      owner_type: f.owner_type,
      owner_id: Number(f.owner_id) || 0,
      path: f.path.trim(),
      note: f.note.trim(),
    })
    ElMessage.success(t('backup.added'))
    pathForm.value = { owner_type: 'user', owner_id: 0, path: '', note: '' }
    loadPaths()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.addFailed'))
  } finally {
    savingPath.value = false
  }
}
async function onDeletePath(row: any) {
  try {
    await ElMessageBox.confirm(t('backup.deleteConfirm', { name: row.path }), t('backup.notice'), { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.delete('/system/backup/paths', { data: { id: row.id } })
    ElMessage.success(t('backup.deleted'))
    loadPaths()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.deleteFailed'))
  }
}

// ── 归档 / 历史 ──
async function loadArchives() {
  loadingArchives.value = true
  try {
    const d: any = await http.get('/system/backup/list')
    archives.value = (d.data?.items || []).filter((i: any) => /\.(tar\.gz|sql\.gz)$/.test(i.name))
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.loadFailed'))
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
    ElMessage.error(e?.message || t('backup.loadFailed'))
  } finally {
    loadingRecords.value = false
  }
}

// ── 立即备份 ──
async function onManualBackup() {
  const f = form.value
  if (!f.name.trim()) return ElMessage.warning(t('backup.archiveNameRequired'))
  running.value = true
  try {
    if (f.kind === 'dir') {
      const paths = f.pathsText.split('\n').map((s) => s.trim()).filter(Boolean)
      if (!paths.length) return ElMessage.warning(t('backup.pathsRequired'))
      await http.post('/system/backup/create_dir', { name: f.name.trim(), paths })
      ElMessage.success(t('backup.dirBackupDone'))
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
      ElMessage.success(t('backup.dbBackupDone'))
    }
    loadArchives()
    loadRecords()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.backupFailed'))
  } finally {
    running.value = false
  }
}

// ── 删除归档 ──
async function onDeleteArchive(row: any) {
  try {
    await ElMessageBox.confirm(t('backup.deleteConfirm', { name: row.name }), t('backup.notice'), { type: 'warning' })
  } catch {
    return
  }
  try {
    await http.post('/system/backup/delete', { path: row.path })
    ElMessage.success(t('backup.deleted'))
    loadArchives()
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.deleteFailed'))
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
        if (!r.targetDir.trim()) return ElMessage.warning(t('backup.targetDirRequired'))
        await http.post('/system/backup/restore_dir', { path: r.path, target_dir: r.targetDir.trim() })
      }
    } else {
      if (!r.dbName.trim()) return ElMessage.warning(t('backup.dbNameRequired'))
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
    ElMessage.success(t('backup.restoreDone'))
    restoreDialog.value = false
  } catch (e: any) {
    ElMessage.error(e?.message || t('backup.restoreFailed'))
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
