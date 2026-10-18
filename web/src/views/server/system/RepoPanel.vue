<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useUserStore } from '@/stores/user'
import {
  Check,
  CloudDownload,
  Delete,
  Document,
  Edit,
  History,
  Plus,
  Refresh,
} from '@/icons'
import {
  applySwitch,
  deleteRepoFile,
  getRepoBackup,
  getRepoBackups,
  getRepoCatalog,
  getRepoFile,
  getRepoStatus,
  previewSwitch,
  restoreRepoBackup,
  saveRepoFile,
  updateRepoCache,
  type RepoBackup,
  type RepoCatalog,
  type RepoFile,
  type RepoPreset,
  type RepoStatus,
  type SwitchChange,
} from '@/api/softwareRepo'

const { t, te } = useI18n()
const userStore = useUserStore()

/** 预设的名称 / 说明按当前语言展示，后端字段仅作兜底。 */
function presetLabel(row: RepoPreset) {
  const key = `softwareRepo.preset.${row.id}`
  return te(key) ? t(key) : row.label
}
function presetDesc(row: RepoPreset) {
  const key = `softwareRepo.presetDesc.${row.id}`
  return te(key) ? t(key) : row.desc
}

/** 整个「服务器配置」组本就仅管理员可见，这里再兜一层（接口同样是 admin only）。 */
const isAdmin = computed(() => userStore.roles.includes('admin'))

const loading = ref(false)
const status = ref<RepoStatus | null>(null)
const catalog = ref<RepoCatalog | null>(null)
const activeTab = ref('switch')

// ── 换源 ──
const mirrorId = ref('')
const includeEpel = ref(false)
const updateCache = ref(true)
const previewVisible = ref(false)
const changes = ref<SwitchChange[]>([])
const applying = ref(false)
const cacheUpdating = ref(false)

const cnMirrors = computed(() => catalog.value?.mirrors.filter((m) => m.region === 'cn') ?? [])
const intlMirrors = computed(() => catalog.value?.mirrors.filter((m) => m.region === 'intl') ?? [])
// family / suite 只有 catalog（zapd 侧识别）才带；status 里的 distro 仅用于展示
const isRpm = computed(() => catalog.value?.distro.family === 'rhel')
const supported = computed(() => catalog.value?.switchable ?? false)

// ── 第三方源 ──
const presets = computed(() => catalog.value?.presets ?? [])
const thirdPartyFiles = computed(
  () => (status.value?.files ?? []).filter((f) => f.kind !== 'main'),
)

// ── 系统已存在的源（条目级，apt 系）──
const existingEntries = computed(() => status.value?.entries ?? [])

/** 条目来源文件名 → status.files 里的文件对象（主 sources.list 也在 files 里） */
function entryFile(name: string) {
  return status.value?.files.find(
    (f) => f.name === name || f.path.endsWith(`/${name}`),
  )
}

// ── 备份 ──
const backups = ref<RepoBackup[]>([])
const backupDir = ref('')
const restoreVisible = ref(false)
const restoreBackupName = ref('')
const restoreTarget = ref('')

// ── 备份预览 ──
const backupPreviewVisible = ref(false)
const backupPreviewName = ref('')
const backupPreviewContent = ref('')

// ── 文件编辑 / 新建（预设添加、手工新建、编辑已有文件共用） ──
const editVisible = ref(false)
const editMode = ref<'create' | 'edit'>('edit')
const editSaving = ref(false)
const editForm = reactive({ path: '', content: '' })

const editDialogTitle = computed(() =>
  editMode.value === 'create'
    ? t('softwareRepo.createTitle')
    : t('softwareRepo.editTitle'),
)

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

function formatTime(ts: number): string {
  if (!ts) return '--'
  return new Date(ts * 1000).toLocaleString()
}

async function load() {
  loading.value = true
  try {
    const [s, c] = await Promise.all([getRepoStatus(), getRepoCatalog()])
    status.value = s.data
    catalog.value = c.data
    if (!mirrorId.value && c.data.mirrors.length) {
      mirrorId.value = c.data.mirrors[0].id
    }
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

async function loadBackups() {
  try {
    const res = await getRepoBackups()
    backups.value = res.data.items ?? []
    backupDir.value = res.data.dir ?? ''
  } catch {
    /* handled */
  }
}

async function doPreview() {
  if (!mirrorId.value) return
  try {
    const res = await previewSwitch(mirrorId.value, includeEpel.value && isRpm.value)
    changes.value = res.data.changes ?? []
    if (!changes.value.length) {
      ElMessage.warning(t('softwareRepo.noChanges'))
      return
    }
    previewVisible.value = true
  } catch {
    /* handled */
  }
}

async function doApply() {
  if (!mirrorId.value) return
  try {
    await ElMessageBox.confirm(
      t('softwareRepo.applyConfirm', { n: changes.value.length || '' }),
      t('common.tip'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  applying.value = true
  try {
    await applySwitch(mirrorId.value, includeEpel.value && isRpm.value, updateCache.value)
    ElMessage.success(t('softwareRepo.applyDone'))
    previewVisible.value = false
    await Promise.all([load(), loadBackups()])
  } catch {
    /* handled */
  } finally {
    applying.value = false
  }
}

async function doUpdateCache() {
  try {
    await ElMessageBox.confirm(t('softwareRepo.cacheConfirm'), t('common.tip'), {
      type: 'info',
    })
  } catch {
    return
  }
  cacheUpdating.value = true
  try {
    const res = await updateRepoCache()
    ElMessage.success(t('softwareRepo.cacheDone', { manager: res.data?.manager ?? '' }))
  } catch {
    /* handled */
  } finally {
    cacheUpdating.value = false
  }
}

// ── 文件编辑 / 新建（预设添加、手工新建、编辑已有文件共用） ──
function openEdit(file: RepoFile) {
  editMode.value = 'edit'
  editForm.path = file.path
  editForm.content = ''
  editVisible.value = true
  getRepoFile(file.path)
    .then((res) => {
      editForm.content = res.data.content ?? ''
    })
    .catch(() => {
      /* handled */
    })
}

/// 新建源文件：传预设则带入其路径与内容；不传就是空表单（手工创建）
function openCreate(preset?: RepoPreset) {
  editMode.value = 'create'
  const dir = isRpm.value ? '/etc/yum.repos.d/' : '/etc/apt/sources.list.d/'
  editForm.path = preset ? `${dir}${preset.file}` : dir
  editForm.content = preset?.content ?? ''
  editVisible.value = true
}

// ── 主配置文件（在线换源页快捷编辑入口）──
const mainFiles = computed(() =>
  isRpm.value
    ? [
        { path: '/etc/dnf/dnf.conf', label: 'dnf.conf' },
        { path: '/etc/yum.conf', label: 'yum.conf' },
      ]
    : [{ path: '/etc/apt/sources.list', label: 'sources.list' }],
)

/** 打开主配置文件编辑；文件不存在（部分系统没有 sources.list）则按新建处理。 */
async function openMainFile(path: string) {
  editMode.value = 'edit'
  editForm.path = path
  editForm.content = ''
  editVisible.value = true
  try {
    const res = await getRepoFile(path)
    editForm.content = res.data.content ?? ''
  } catch {
    editMode.value = 'create'
  }
}

async function submitEdit() {
  const path = editForm.path.trim()
  if (!path.startsWith('/') || !editForm.content.trim()) {
    ElMessage.warning(t('softwareRepo.editRequired'))
    return
  }
  editSaving.value = true
  try {
    await saveRepoFile(path, editForm.content)
    ElMessage.success(t('softwareRepo.saved'))
    editVisible.value = false
    await Promise.all([load(), loadBackups()])
  } catch {
    /* handled */
  } finally {
    editSaving.value = false
  }
}

async function removeFile(file: RepoFile) {
  try {
    await ElMessageBox.confirm(t('softwareRepo.deleteConfirm', { path: file.path }), t('common.tip'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await deleteRepoFile(file.path)
    ElMessage.success(t('softwareRepo.deleted'))
    await Promise.all([load(), loadBackups()])
  } catch {
    /* handled */
  }
}

// ── 备份还原 ──
/**
 * 备份名 `{ts}-{原文件名}.bak` → 还原目标路径。
 *
 * 备份文件名里带着原文件名（`1791957111-epel.repo.bak` → `epel.repo`），
 * 优先匹配当前系统里同名文件拿到真实路径；匹配不到就按发行版目录推导，
 * 免得默认落到列表第一项、把 A 的备份写进 B 的文件。
 */
function targetFromBackup(backupName: string): string {
  const name = backupName.replace(/^\d+-/, '').replace(/\.bak$/, '')
  if (!name) return status.value?.files[0]?.path ?? ''
  const hit = status.value?.files.find((f) => f.name === name)
  if (hit) return hit.path
  const dir = name.endsWith('.repo')
    ? '/etc/yum.repos.d/'
    : name === 'sources.list'
      ? '/etc/apt/'
      : '/etc/apt/sources.list.d/'
  return `${dir}${name}`
}

function openRestore(backup: RepoBackup) {
  restoreBackupName.value = backup.name
  restoreTarget.value = targetFromBackup(backup.name)
  restoreVisible.value = true
}

async function openBackupPreview(backup: RepoBackup) {
  backupPreviewName.value = backup.name
  backupPreviewContent.value = ''
  backupPreviewVisible.value = true
  try {
    const res = await getRepoBackup(backup.name)
    backupPreviewContent.value = res.data.content ?? ''
  } catch {
    backupPreviewVisible.value = false
  }
}

async function doRestore() {
  if (!restoreBackupName.value || !restoreTarget.value) return
  try {
    await ElMessageBox.confirm(
      t('softwareRepo.restoreConfirm', { path: restoreTarget.value }),
      t('common.tip'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await restoreRepoBackup(restoreBackupName.value, restoreTarget.value)
    ElMessage.success(t('softwareRepo.restored'))
    restoreVisible.value = false
    await Promise.all([load(), loadBackups()])
  } catch {
    /* handled */
  }
}

onMounted(() => {
  load()
  loadBackups()
})
</script>

<template>
  <div class="repo-panel" v-loading="loading">
    <!-- 系统识别 + 全局操作 -->
    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <div class="head-left">
            <span class="page-title">{{ t('softwareRepo.title') }}</span>
            <el-tag v-if="status" size="small" type="info" style="margin-left: 8px">
              {{ status.distro.prettyName || status.distro.id || '--' }}
            </el-tag>
            <el-tag v-if="status" size="small" type="warning" style="margin-left: 6px">
              {{ status.manager }}
            </el-tag>
          </div>
          <div class="head-right">
            <el-button :icon="Refresh" circle :loading="loading" @click="load" />
            <el-button
              type="primary"
              :icon="CloudDownload"
              :loading="cacheUpdating"
              :disabled="!isAdmin"
              @click="doUpdateCache"
            >
              {{ t('softwareRepo.updateCache') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-alert
        v-if="status && !supported"
        :title="
          isRpm
            ? t('softwareRepo.unsupportedLayout', {
                distro: status.distro.prettyName || status.distro.id,
              })
            : t('softwareRepo.unsupported')
        "
        type="warning"
        :closable="false"
        show-icon
      />
      <el-descriptions v-else-if="status" :column="3" border size="small">
        <el-descriptions-item :label="t('softwareRepo.distro')">
          {{ status.distro.prettyName || '--' }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('softwareRepo.family')">
          {{ isRpm ? 'RPM (dnf/yum)' : 'APT (deb)' }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('softwareRepo.fileCount')">
          {{ status.files.length }}
        </el-descriptions-item>
      </el-descriptions>
    </el-card>

    <el-tabs v-model:active-tab="activeTab" style="margin-top: 16px">
      <!-- ── 换源 ── -->
      <el-tab-pane :label="t('softwareRepo.tabSwitch')" name="switch">
        <el-card shadow="never">
          <div class="switch-row">
            <el-select v-model="mirrorId" :disabled="!supported" style="width: 260px">
              <el-option-group v-for="g in [cnMirrors, intlMirrors]" :key="g[0]?.region" :label="g[0]?.region === 'cn' ? t('softwareRepo.mirrorCn') : t('softwareRepo.mirrorIntl')">
                <el-option
                  v-for="m in g"
                  :key="m.id"
                  :label="m.label"
                  :value="m.id"
                />
              </el-option-group>
            </el-select>
            <el-checkbox v-if="isRpm" v-model="includeEpel">
              {{ t('softwareRepo.includeEpel') }}
            </el-checkbox>
            <el-checkbox v-model="updateCache">
              {{ t('softwareRepo.updateCacheAfter') }}
            </el-checkbox>
            <el-button type="primary" :disabled="!supported || !isAdmin" @click="doPreview">
              {{ t('softwareRepo.preview') }}
            </el-button>
          </div>
          <el-alert
            style="margin-top: 12px"
            :title="t('softwareRepo.switchHint')"
            type="info"
            :closable="false"
            show-icon
          />
        </el-card>

        <!-- 主配置文件快捷编辑：主源不在 sources.list.d 目录里，这里给直达入口 -->
        <el-card shadow="never" style="margin-top: 16px">
          <template #header>
            <div class="card-header">
              <span class="page-title">{{ t('softwareRepo.mainFilesTitle') }}</span>
            </div>
          </template>
          <el-alert
            style="margin-bottom: 12px"
            :title="t('softwareRepo.mainFilesHint')"
            type="info"
            :closable="false"
            show-icon
          />
          <el-button
            v-for="f in mainFiles"
            :key="f.path"
            type="primary"
            plain
            :icon="Edit"
            :disabled="!isAdmin"
            @click="openMainFile(f.path)"
          >
            {{ t('softwareRepo.editMainFile', { name: f.label }) }}
          </el-button>
        </el-card>
      </el-tab-pane>

      <!-- ── 第三方源 ── -->
      <el-tab-pane :label="t('softwareRepo.tabThirdParty')" name="third">
        <el-card shadow="never">
          <template #header>
            <div class="card-header">
              <span class="page-title">{{ t('softwareRepo.presetTitle') }}</span>
            </div>
          </template>
          <el-table :data="presets" stripe size="small">
            <el-table-column :label="t('softwareRepo.colName')" min-width="160">
              <template #default="{ row }">
                <el-tag size="small" type="primary">{{ presetLabel(row) }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('softwareRepo.colDesc')" min-width="300">
              <template #default="{ row }">{{ presetDesc(row) }}</template>
            </el-table-column>
            <el-table-column prop="file" :label="t('softwareRepo.colFile')" min-width="160" />
            <el-table-column :label="t('common.operation')" width="110" fixed="right">
              <template #default="{ row }">
                <el-button link type="primary" :icon="Plus" :disabled="!isAdmin" @click="openCreate(row)">
                  {{ t('softwareRepo.add') }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </el-card>

        <el-card shadow="never" style="margin-top: 16px">
          <template #header>
            <div class="card-header">
              <span class="page-title">{{ t('softwareRepo.installedTitle') }}</span>
              <el-button type="primary" :icon="Plus" :disabled="!isAdmin" @click="openCreate()">
                {{ t('softwareRepo.newFile') }}
              </el-button>
            </div>
          </template>
          <el-table :data="thirdPartyFiles" stripe size="small">
            <el-table-column prop="name" :label="t('softwareRepo.colName')" min-width="180" />
            <el-table-column prop="path" :label="t('softwareRepo.colPath')" min-width="280" show-overflow-tooltip />
            <el-table-column :label="t('softwareRepo.colSize')" width="100">
              <template #default="{ row }">{{ formatSize(row.size) }}</template>
            </el-table-column>
            <el-table-column :label="t('softwareRepo.colMtime')" width="180">
              <template #default="{ row }">{{ formatTime(row.mtime) }}</template>
            </el-table-column>
            <el-table-column :label="t('softwareRepo.colEnabled')" width="90">
              <template #default="{ row }">
                <el-tag size="small" :type="row.enabled ? 'success' : 'info'">
                  {{ row.enabled ? t('softwareRepo.enabled') : t('softwareRepo.disabled') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="170" fixed="right">
              <template #default="{ row }">
                <el-button link type="primary" :icon="Edit" @click="openEdit(row)">
                  {{ t('softwareRepo.edit') }}
                </el-button>
                <el-button link type="danger" :icon="Delete" :disabled="!isAdmin" @click="removeFile(row)">
                  {{ t('softwareRepo.delete') }}
                </el-button>
              </template>
            </el-table-column>
            <template #empty>
              <el-empty :description="t('softwareRepo.noThirdParty')" :image-size="60" />
            </template>
          </el-table>
        </el-card>

        <!-- 系统已存在的源：条目级清单（apt 系才有；主 sources.list 里的行也在这里可见） -->
        <el-card v-if="!isRpm && existingEntries.length" shadow="never" style="margin-top: 16px">
          <template #header>
            <div class="card-header">
              <span class="page-title">{{ t('softwareRepo.existingTitle') }}</span>
            </div>
          </template>
          <el-alert
            style="margin-bottom: 12px"
            :title="t('softwareRepo.existingHint')"
            type="info"
            :closable="false"
            show-icon
          />
          <el-table :data="existingEntries" stripe size="small">
            <el-table-column prop="file" :label="t('softwareRepo.colFile')" min-width="150" />
            <el-table-column prop="type" :label="t('softwareRepo.colType')" width="90">
              <template #default="{ row }">
                <el-tag size="small" :type="row.type === 'deb' ? 'primary' : 'info'">{{ row.type }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="uri" :label="t('softwareRepo.colUri')" min-width="260" show-overflow-tooltip />
            <el-table-column prop="suite" :label="t('softwareRepo.colSuite')" min-width="130" />
            <el-table-column prop="components" :label="t('softwareRepo.colComponents')" min-width="200" show-overflow-tooltip />
            <el-table-column :label="t('softwareRepo.colEnabled')" width="90">
              <template #default="{ row }">
                <el-tag size="small" :type="row.enabled ? 'success' : 'info'">
                  {{ row.enabled ? t('softwareRepo.enabled') : t('softwareRepo.disabled') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="110" fixed="right">
              <template #default="{ row }">
                <el-button
                  v-if="entryFile(row.file)"
                  link
                  type="primary"
                  :icon="Edit"
                  :disabled="!isAdmin"
                  @click="openEdit(entryFile(row.file)!)"
                >
                  {{ t('softwareRepo.edit') }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-tab-pane>

      <!-- ── 备份还原 ── -->
      <el-tab-pane :label="t('softwareRepo.tabBackup')" name="backup">
        <el-card shadow="never">
          <template #header>
            <div class="card-header">
              <span class="page-title">{{ t('softwareRepo.backupTitle') }}</span>
              <el-button :icon="Refresh" circle size="small" @click="loadBackups" />
            </div>
          </template>
          <el-alert
            style="margin-bottom: 12px"
            :title="t('softwareRepo.backupHint')"
            type="info"
            :closable="false"
            show-icon
          />
          <el-table :data="backups" stripe size="small">
            <el-table-column prop="name" :label="t('softwareRepo.colBackup')" min-width="260" />
            <el-table-column :label="t('softwareRepo.colSize')" width="100">
              <template #default="{ row }">{{ formatSize(row.size) }}</template>
            </el-table-column>
            <el-table-column :label="t('softwareRepo.colMtime')" width="180">
              <template #default="{ row }">{{ formatTime(row.mtime) }}</template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="170" fixed="right">
              <template #default="{ row }">
                <el-button link type="primary" :icon="Document" @click="openBackupPreview(row)">
                  {{ t('softwareRepo.backupPreview') }}
                </el-button>
                <el-button link type="primary" :icon="History" :disabled="!isAdmin" @click="openRestore(row)">
                  {{ t('softwareRepo.restore') }}
                </el-button>
              </template>
            </el-table-column>
            <template #empty>
              <el-empty :description="t('softwareRepo.backupEmpty')" :image-size="60" />
            </template>
          </el-table>
        </el-card>
      </el-tab-pane>
    </el-tabs>

    <!-- 换源预览 -->
    <el-dialog v-model="previewVisible" :title="t('softwareRepo.previewTitle')" width="720px">
      <el-alert
        style="margin-bottom: 12px"
        :title="t('softwareRepo.previewHint')"
        type="warning"
        :closable="false"
        show-icon
      />
      <div class="preview-body">
        <div v-for="c in changes" :key="c.path" class="change-block">
          <div class="change-path">
            <el-icon><Check /></el-icon>
            <span>{{ c.path }}</span>
          </div>
          <pre class="code-preview">{{ c.content }}</pre>
        </div>
      </div>
      <template #footer>
        <el-button @click="previewVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="applying" @click="doApply">
          {{ t('softwareRepo.apply') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 文件编辑 / 新建 -->
    <el-dialog v-model="editVisible" :title="editDialogTitle" width="720px">
      <el-alert
        style="margin-bottom: 12px"
        :title="t('softwareRepo.editHint')"
        type="info"
        :closable="false"
        show-icon
      />
      <el-form label-width="90px">
        <el-form-item :label="t('softwareRepo.pathLabel')">
          <el-input v-model="editForm.path" placeholder="/etc/yum.repos.d/xxx.repo" />
        </el-form-item>
        <el-form-item :label="t('softwareRepo.contentLabel')">
          <el-input
            v-model="editForm.content"
            type="textarea"
            :rows="14"
            class="code-input"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="editSaving" @click="submitEdit">
          {{ t('common.save') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 备份还原 -->
    <el-dialog v-model="restoreVisible" :title="t('softwareRepo.restoreTitle')" width="560px">      <el-form label-width="110px">
        <el-form-item :label="t('softwareRepo.colBackup')">
          <el-input :model-value="restoreBackupName" readonly />
        </el-form-item>
        <el-form-item :label="t('softwareRepo.restoreTarget')">
          <el-select
            v-model="restoreTarget"
            filterable
            allow-create
            default-first-option
            style="width: 100%"
          >
            <el-option
              v-for="f in status?.files ?? []"
              :key="f.path"
              :label="f.path"
              :value="f.path"
            />
          </el-select>
          <div class="form-tip">{{ t('softwareRepo.restoreAutoHint') }}</div>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="restoreVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doRestore">{{ t('softwareRepo.restore') }}</el-button>
      </template>
    </el-dialog>

    <!-- 备份内容预览 -->
    <el-dialog v-model="backupPreviewVisible" :title="t('softwareRepo.backupPreviewTitle')" width="720px">
      <div class="change-path">
        <el-icon><Document /></el-icon>
        <span>{{ backupPreviewName }}</span>
      </div>
      <pre class="code-preview backup-preview">{{ backupPreviewContent }}</pre>
      <template #footer>
        <el-button @click="backupPreviewVisible = false">{{ t('common.close') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style lang="scss" scoped>
.repo-panel {
  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;

    .head-left {
      display: flex;
      align-items: center;
    }
  }

  .page-title {
    font-weight: 600;
  }

  .switch-row {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }

  .change-block {
    margin-bottom: 14px;

    .change-path {
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 13px;
      font-weight: 600;
      margin-bottom: 6px;

      .el-icon {
        color: var(--el-color-success);
      }
    }
  }

  .preview-body {
    max-height: 54vh;
    overflow-y: auto;
    padding-right: 4px;
  }

  .form-tip {
    margin-top: 4px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--el-text-color-secondary);
  }

  .backup-preview {
    max-height: 60vh;
    margin-top: 10px;
  }

  .code-preview {
    margin: 0;
    padding: 12px;
    max-height: 200px;
    overflow: auto;
    font-size: 12px;
    line-height: 1.6;
    font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
    background: var(--el-fill-color-light);
    border: 1px solid var(--el-border-color-lighter);
    border-radius: 6px;
    white-space: pre-wrap;
    word-break: break-all;
  }
}
</style>
