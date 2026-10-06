<template>
  <div class="plugin-page">
    <!-- 页头 -->
    <el-card shadow="never" class="head-card">
      <div class="head-row">
        <div class="head-left">
          <el-icon :size="22" color="#409eff"><Box /></el-icon>
          <div>
            <div class="head-title">{{ t('devPlugins.headTitle') }}</div>
            <div class="head-sub">{{ t('devPlugins.headSub') }}</div>
          </div>
        </div>
        <div class="head-right">
          <el-button v-if="isAdmin" :icon="Upload" @click="openUpload">
            {{ t('devPlugins.installUpload') }}
          </el-button>
          <el-button :icon="Refresh" circle :loading="loading" @click="load" />
        </div>
      </div>
    </el-card>

    <!-- 插件统一装在系统目录，所有用户共享；不再区分「我的」 -->

    <!-- 列表 -->
    <el-card shadow="never" class="table-card">
      <el-table :data="visibleRows" v-loading="loading" stripe>
        <el-table-column :label="t('devPlugins.colPlugin')" min-width="220">
          <template #default="{ row }">
            <div class="cell-name">{{ row.title || row.name }}</div>
            <div class="cell-sub">
              <code>{{ row.name }}</code>
              <span v-if="row.version" class="ver">v{{ row.version }}</span>
            </div>
            <div v-if="row.description" class="cell-desc">{{ row.description }}</div>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colScope')" width="110">
          <template #default="{ row }">
            <el-tag :type="row.scope === 'system' ? 'danger' : 'success'" effect="plain" size="small">
              {{ row.scope === 'system' ? t('devPlugins.scopeSystem') : t('devPlugins.scopeSite') }}
            </el-tag>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colPlacement')" width="130">
          <template #default="{ row }">
            <span class="mono">{{ row.placement || '-' }}</span>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colSource')" min-width="180">
          <template #default="{ row }">
            <div class="cell-sub">
              <span>{{ sourceLabel(row) }}</span>
              <span v-if="row.installed_at" class="dim">{{ fmtTime(row.installed_at) }}</span>
            </div>
            <div v-if="row.src && row.source === 'git'" class="cell-desc mono ellipsis">
              {{ row.src }}
            </div>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colActions')" width="220" align="right">
          <template #default="{ row }">
            <el-button link type="primary" size="small" @click="openSched(row)">
              {{ t('pluginSched.add') }}
            </el-button>
            <el-button link size="small" @click="onTest(row)">
              {{ t('pluginTest.run') }}
            </el-button>
            <el-button
              link
              type="danger"
              size="small"
              :disabled="!isAdmin"
              @click="onUninstall(row)"
            >
              {{ t('devPlugins.uninstall') }}
            </el-button>
          </template>
        </el-table-column>

        <template #empty>
          <el-empty :description="t('devPlugins.empty')" :image-size="60" />
        </template>
      </el-table>
    </el-card>

    <!-- 上传安装 -->
    <el-dialog v-model="uploadVisible" :title="t('devPlugins.uploadTitle')" width="520px">
      <el-form label-width="110px">
        <el-form-item :label="t('devPlugins.uploadName')">
          <el-input
            v-model="uploadForm.name"
            :placeholder="t('devPlugins.uploadNamePlaceholder')"
          />
        </el-form-item>
        <el-form-item :label="t('devPlugins.uploadFile')">
          <div class="file-row">
            <input ref="fileInputRef" type="file" class="hidden-input" @change="onFilePick" />
            <el-input :model-value="uploadForm.file?.name || ''" readonly style="flex: 1">
              <template #append>
                <el-button @click="fileInputRef?.click()">{{ t('devPlugins.uploadPick') }}</el-button>
              </template>
            </el-input>
          </div>
          <div class="form-tip">{{ t('devPlugins.uploadTip') }}</div>
        </el-form-item>
        <el-form-item>
          <el-checkbox v-model="uploadForm.force">
            {{ t('devPlugins.uploadForce') }}
          </el-checkbox>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="uploadVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="submitting" @click="submitUpload">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 定时 / Webhook 触发 -->
    <el-dialog v-model="schedVisible" width="860px" :title="schedTitle">
      <el-table :data="schedRows" v-loading="schedLoading" size="small" stripe>
        <el-table-column :label="t('pluginSched.colType')" width="100">
          <template #default="{ row }">
            <el-tag size="small" :type="row.trigger === 'webhook' ? 'warning' : ''" effect="plain">
              {{ row.trigger === 'webhook' ? t('pluginSched.webhook') : t('pluginSched.cron') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('pluginSched.colAction')" prop="action" width="140" />
        <el-table-column :label="t('pluginSched.colCron')" width="140">
          <template #default="{ row }">
            <span class="mono">{{ row.trigger === 'cron' ? row.cron : '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('pluginSched.colLastRun')" min-width="150">
          <template #default="{ row }">
            <div v-if="!row.last_run" class="dim">{{ t('pluginSched.none') }}</div>
            <div v-else class="dim">
              {{ fmtTime(row.last_run) }}
              <el-tag size="small" :type="row.last_ok === false ? 'danger' : 'success'" effect="plain">
                {{ row.last_ok === false ? t('pluginSched.lastFail') : t('pluginSched.lastOk') }}
              </el-tag>
            </div>
            <div v-if="row.trigger === 'webhook' && row.token" class="cell-desc mono ellipsis">
              {{ hookUrl(row) }}
            </div>
          </template>
        </el-table-column>
        <el-table-column :label="t('pluginSched.colEnabled')" width="90">
          <template #default="{ row }">
            <el-switch
              :model-value="row.enabled"
              size="small"
              @change="(v: boolean) => onToggle(row, v as boolean)"
            />
          </template>
        </el-table-column>
        <el-table-column width="170" align="right">
          <template #default="{ row }">
            <el-button link size="small" type="primary" @click="onEditSched(row)">
              {{ t('pluginSched.edit') }}
            </el-button>
            <el-button
              v-if="row.trigger === 'webhook'"
              link
              size="small"
              @click="onRotate(row)"
            >
              {{ t('pluginSched.rotate') }}
            </el-button>
            <el-button link size="small" type="danger" @click="onDeleteSched(row)">
              {{ t('pluginSched.del') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('pluginSched.empty')" :image-size="60" />
        </template>
      </el-table>

      <el-divider content-position="left">{{ schedFormTitle }}</el-divider>
      <el-form label-width="110px" size="small">
        <el-form-item :label="t('pluginSched.colType')">
          <el-radio-group v-model="schedForm.trigger">
            <el-radio value="cron">{{ t('pluginSched.cron') }}</el-radio>
            <el-radio value="webhook">{{ t('pluginSched.webhook') }}</el-radio>
          </el-radio-group>
          <div class="form-tip">
            {{ schedForm.trigger === 'webhook' ? t('pluginSched.hookPath') : t('pluginSched.cronTip') }}
          </div>
        </el-form-item>
        <el-form-item :label="t('pluginSched.colAction')">
          <el-select v-model="schedForm.action" filterable allow-create clearable :placeholder="t('pluginSched.actionPlaceholder')">
            <el-option v-for="a in actionNames" :key="a" :label="a" :value="a" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="schedForm.trigger === 'cron'" :label="t('pluginSched.colCron')">
          <el-input v-model="schedForm.cron" :placeholder="t('pluginSched.cronPlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('pluginSched.siteIdLabel')">
          <el-input-number v-model="schedForm.site_id" :min="0" controls-position="right" clearable />
          <div class="form-tip">{{ t('pluginSched.siteIdTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('pluginSched.optionsLabel')">
          <el-input
            v-model="schedForm.optionsText"
            type="textarea"
            :rows="2"
            :placeholder="t('pluginSched.optionsTip')"
          />
        </el-form-item>
      </el-form>

      <template #footer>
        <el-button @click="schedVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="schedSaving" @click="submitSched">
          {{ t('pluginSched.save') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 冒烟测试（tests.yaml） -->
    <el-dialog v-model="testVisible" width="900px" :title="testTitle">
      <div v-if="testSummary" class="form-tip">{{ testSummary }}</div>
      <el-table :data="testResults" v-loading="testLoading" size="small" stripe>
        <el-table-column :label="t('pluginTest.case')" prop="name" width="180" />
        <el-table-column :label="t('pluginTest.result')" width="110">
          <template #default="{ row }">
            <el-tag size="small" effect="plain" :type="resultType(row)">
              {{ resultLabel(row) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('pluginTest.detail')">
          <template #default="{ row }">
            <span>{{ row.detail || '-' }}</span>
            <pre v-if="row.log" class="cell-log">{{ row.log }}</pre>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('pluginTest.empty')" :image-size="60" />
        </template>
      </el-table>
      <template #footer>
        <el-button @click="testVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="testLoading" @click="runTest">
          {{ t('pluginTest.run') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Box, Refresh, Upload } from '@/icons'
import {
  pluginInstallUpload,
  pluginList,
  pluginScheduleCreate,
  pluginScheduleDelete,
  pluginScheduleList,
  pluginScheduleUpdate,
  pluginTest,
  pluginUninstall,
  type PluginInfo,
  type PluginSchedule,
  type ScheduleTrigger,
} from '@/api/plugin'
import { API_BASE } from '@/utils/base'
import { useUserStore } from '@/stores/user'

const { t } = useI18n()
const userStore = useUserStore()

const rows = ref<PluginInfo[]>([])
const loading = ref(false)
const submitting = ref(false)

// 插件统一装在系统目录，所有用户共享；直接展示全部
const visibleRows = computed(() => rows.value)

// 插件统一由管理员安装到系统目录，只有 admin 能上传 / 卸载；前端先拦一道，后端再兜一次。
// 运行身份由各插件 manifest 的 scope 决定（站点账号 / root），与安装级别无关。
const isAdmin = computed(() => (userStore.roles || []).includes('admin'))

async function load() {
  loading.value = true
  try {
    // 不传 slot：管理页要看全部插件，不按入口位置过滤
    const r: any = await pluginList({})
    const data = r?.data?.data ?? r?.data ?? []
    rows.value = Array.isArray(data) ? data : []
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  } finally {
    loading.value = false
  }
}

function sourceLabel(row: PluginInfo) {
  if (row.source === 'git') return t('devPlugins.sourceGit')
  if (row.source === 'appstore') return t('devPlugins.sourceAppstore')
  if (row.source === 'archive') return t('devPlugins.sourceArchive')
  return t('devPlugins.sourceManual')
}

function fmtTime(ts: number) {
  if (!ts) return ''
  const d = new Date(ts * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

// ── 上传安装 ────────────────────────────────────────────────
const uploadVisible = ref(false)
const fileInputRef = ref<HTMLInputElement | null>(null)
const uploadForm = ref<{
  force: boolean
  name: string
  file: File | null
}>({ force: false, name: '', file: null })

function openUpload() {
  uploadForm.value = { force: false, name: '', file: null }
  uploadVisible.value = true
}

function onFilePick(e: Event) {
  const input = e.target as HTMLInputElement
  uploadForm.value.file = input.files?.[0] || null
}

async function submitUpload() {
  if (!uploadForm.value.file) {
    ElMessage.warning(t('devPlugins.uploadNoFile'))
    return
  }
  if (!isAdmin.value) {
    ElMessage.warning(t('devPlugins.adminOnly'))
    return
  }
  submitting.value = true
  try {
    await pluginInstallUpload({
      file: uploadForm.value.file,
      force: uploadForm.value.force,
      name: uploadForm.value.name,
    })
    ElMessage.success(t('devPlugins.installOk'))
    uploadVisible.value = false
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  } finally {
    submitting.value = false
  }
}

// ── 卸载 ────────────────────────────────────────────────────
async function onUninstall(row: PluginInfo) {
  if (!isAdmin.value) {
    ElMessage.warning(t('devPlugins.adminOnly'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('devPlugins.uninstallConfirm', { name: row.name }),
      t('devPlugins.uninstallTitle'),
      { type: 'warning', confirmButtonText: t('devPlugins.uninstall'), cancelButtonText: t('common.cancel') },
    )
  } catch {
    return
  }
  try {
    await pluginUninstall({ name: row.name })
    ElMessage.success(t('devPlugins.uninstallOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  }
}

// ── 定时 / Webhook 触发 ────────────────────────────────────
const schedVisible = ref(false)
const schedRows = ref<PluginSchedule[]>([])
const schedLoading = ref(false)
const schedSaving = ref(false)
const schedPlugin = ref<PluginInfo | null>(null)
const editingId = ref('')

const schedForm = ref<{
  trigger: ScheduleTrigger
  action: string
  cron: string
  site_id: number | null
  optionsText: string
  rotate_token: boolean
}>({ trigger: 'cron', action: 'run', cron: '', site_id: null, optionsText: '', rotate_token: false })

const schedTitle = computed(() =>
  schedPlugin.value
    ? `${t('pluginSched.add')} · ${schedPlugin.value.title || schedPlugin.value.name}`
    : t('pluginSched.add'),
)
const schedFormTitle = computed(() =>
  editingId.value ? t('pluginSched.edit') : t('pluginSched.save'),
)
// 可选动作清单：优先用前端拿到的 i18n 文案，回落到 action 名
const actionNames = computed(() => {
  const p = schedPlugin.value
  if (!p) return ['run']
  const fromSpecs = (p.action_specs || []).map((s) => s.name)
  const fromActions = Object.keys(p.actions || {})
  const all = Array.from(new Set([...fromActions, ...fromSpecs]))
  return all.length ? all : ['run']
})

function hookUrl(row: PluginSchedule) {
  // 带上 url_prefix（utils/base 的 API_BASE），否则部署在子路径下复制出来的地址是错的
  return `${window.location.origin}${API_BASE}/plugin/hook/${row.token}`
}

async function loadSched() {
  schedLoading.value = true
  try {
    const r: any = await pluginScheduleList()
    const data = r?.data?.data ?? r?.data ?? []
    const all: PluginSchedule[] = Array.isArray(data) ? data : []
    schedRows.value = all.filter(
      (s) => !schedPlugin.value || s.plugin === schedPlugin.value.name,
    )
  } catch (e: any) {
    ElMessage.error(e?.message || t('pluginSched.loadFail'))
  } finally {
    schedLoading.value = false
  }
}

function openSched(row: PluginInfo) {
  schedPlugin.value = row
  editingId.value = ''
  schedForm.value = {
    trigger: 'cron',
    action: 'run',
    cron: '',
    site_id: null,
    optionsText: '',
    rotate_token: false,
  }
  schedVisible.value = true
  loadSched()
}

function onEditSched(row: PluginSchedule) {
  editingId.value = row.id
  schedForm.value = {
    trigger: row.trigger,
    action: row.action || 'run',
    cron: row.cron || '',
    site_id: row.site_id ?? null,
    optionsText: row.options ? JSON.stringify(row.options) : '',
    rotate_token: false,
  }
}

function parseOptions(): Record<string, string> | undefined {
  const text = schedForm.value.optionsText.trim()
  if (!text) return undefined
  try {
    const v = JSON.parse(text)
    if (!v || typeof v !== 'object' || Array.isArray(v)) {
      throw new Error('not an object')
    }
    return v
  } catch {
    ElMessage.warning(t('pluginSched.invalidJson'))
    throw new Error('invalid options')
  }
}

async function submitSched() {
  if (!schedPlugin.value) return
  if (schedForm.value.trigger === 'cron' && !schedForm.value.cron.trim()) {
    ElMessage.warning(t('pluginSched.needCron'))
    return
  }
  let options: Record<string, string> | undefined
  try {
    options = parseOptions()
  } catch {
    return
  }
  schedSaving.value = true
  try {
    const payload = {
      plugin: schedPlugin.value.name,
      action: schedForm.value.action || 'run',
      trigger: schedForm.value.trigger,
      cron: schedForm.value.cron,
      site_id: schedForm.value.site_id ?? null,
      options,
      rotate_token: schedForm.value.rotate_token,
      id: editingId.value || undefined,
    }
    if (editingId.value) {
      await pluginScheduleUpdate(payload)
    } else {
      await pluginScheduleCreate(payload)
    }
    ElMessage.success(t('pluginSched.createOk'))
    editingId.value = ''
    schedForm.value.optionsText = ''
    await loadSched()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  } finally {
    schedSaving.value = false
  }
}

async function onToggle(row: PluginSchedule, enabled: boolean) {
  try {
    await pluginScheduleUpdate({
      id: row.id,
      plugin: row.plugin,
      action: row.action,
      trigger: row.trigger,
      cron: row.cron,
      site_id: row.site_id ?? null,
      options: row.options,
      enabled,
    })
    await loadSched()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  }
}

async function onRotate(row: PluginSchedule) {
  try {
    await ElMessageBox.confirm(t('pluginSched.rotateTip'), t('pluginSched.rotate'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await pluginScheduleUpdate({
      id: row.id,
      plugin: row.plugin,
      action: row.action,
      trigger: row.trigger,
      cron: row.cron,
      site_id: row.site_id ?? null,
      options: row.options,
      rotate_token: true,
    })
    ElMessage.success(t('pluginSched.rotateOk'))
    await loadSched()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  }
}

async function onDeleteSched(row: PluginSchedule) {
  try {
    await ElMessageBox.confirm(t('pluginSched.delConfirm'), t('pluginSched.del'), {
      type: 'warning',
      confirmButtonText: t('pluginSched.del'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  try {
    await pluginScheduleDelete(row.id)
    ElMessage.success(t('pluginSched.delOk'))
    if (editingId.value === row.id) editingId.value = ''
    await loadSched()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  }
}

// ── 冒烟测试（tests.yaml）──────────────────────────────────
const testVisible = ref(false)
const testLoading = ref(false)
const testPlugin = ref<PluginInfo | null>(null)
const testResults = ref<any[]>([])
const testSummary = ref('')

const testTitle = computed(() =>
  testPlugin.value
    ? `${t('pluginTest.title')} · ${testPlugin.value.title || testPlugin.value.name}`
    : t('pluginTest.title'),
)

function resultType(row: any) {
  if (row.skipped || row.ok === null) return 'info'
  return row.ok ? 'success' : 'danger'
}

function resultLabel(row: any) {
  if (row.skipped || row.ok === null) return t('pluginTest.skipped')
  return row.ok ? t('pluginTest.passed') : t('pluginTest.failed')
}

function onTest(row: PluginInfo) {
  testPlugin.value = row
  testResults.value = []
  testSummary.value = ''
  testVisible.value = true
  runTest()
}

async function runTest() {
  if (!testPlugin.value) return
  testLoading.value = true
  try {
    const r: any = await pluginTest(testPlugin.value.name)
    const d = r?.data?.data ?? r?.data ?? {}
    testResults.value = Array.isArray(d.results) ? d.results : []
    testSummary.value = r?.data?.message || ''
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
    testResults.value = []
  } finally {
    testLoading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.plugin-page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.plugin-tabs {
  align-self: flex-start;
}
.head-card :deep(.el-card__body) {
  padding: 14px 16px;
}
.head-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
.head-left {
  display: flex;
  align-items: center;
  gap: 12px;
}
.head-title {
  font-size: 17px;
  font-weight: 600;
}
.head-sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.head-right {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cell-name {
  font-weight: 600;
}
.cell-sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  display: flex;
  gap: 8px;
  align-items: center;
}
.cell-desc {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.ver {
  color: var(--el-color-primary);
}
.dim {
  color: var(--el-text-color-placeholder);
}
.mono {
  font-family: var(--el-font-family-mono, monospace);
  font-size: 12px;
}
.ellipsis {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-row {
  display: flex;
  align-items: center;
  width: 100%;
}
.hidden-input {
  display: none;
}
.form-tip {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
}
/* 测试弹窗里的插件原始输出：保留换行，别把日志挤成一坨 */
.cell-log {
  margin: 6px 0 0;
  padding: 8px 10px;
  max-height: 180px;
  overflow: auto;
  background: var(--el-fill-color-light);
  border-radius: 4px;
  font-family: var(--el-font-family-mono, monospace);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-all;
}
</style>
