<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import type { EnvConf, EnvData, FpmSpecItem } from '@/api/serverEnv'
import {
  addFpmSpec,
  deleteFpmSpec,
  getFpmSpecs,
  getServerEnv,
  refreshServerEnv,
  saveServerEnvDefaults,
  updateFpmSpec,
} from '@/api/serverEnv'

const { t } = useI18n()

const env = ref<EnvData | null>(null)
const loading = ref(false)
const refreshing = ref(false)
const dialogVisible = ref(false)
const saving = ref(false)

const form = reactive<EnvConf>({
  webserver: '',
  php_default: '',
  database: '',
  fpm_pool_defaults: '',
  user_home_root: '/home',
  container_runtime: 'auto',
})

/** fpm pool 默认规格 —— 数值字段 */
const fpmNum = reactive({
  max_children: 10,
  start_servers: 3,
  min_spare_servers: 2,
  max_spare_servers: 5,
  max_requests: 1000,
  request_terminate_timeout: 300,
  max_execution_time: 300,
})
/** fpm pool 默认规格 —— 字符串字段 */
const fpmStr = reactive({
  pm: 'ondemand',
  memory_limit: '256M',
  post_max_size: '128M',
  upload_max_filesize: '128M',
})
const FPM_NUM_DEFAULTS: Record<string, number> = { ...fpmNum }
const FPM_STR_DEFAULTS: Record<string, string> = { ...fpmStr }

function fmtTime(ts?: number): string {
  if (!ts) return '--'
  const d = new Date(ts * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

const payload = computed(() => env.value?.payload ?? null)
const conf = computed(() => env.value?.conf ?? null)

const phpOptions = computed<string[]>(() => {
  const list = payload.value?.php?.instances ?? []
  const arr = list.map((i) => shortOf(i.version)).filter(Boolean)
  return [...new Set(arr)]
})
function shortOf(v: string): string {
  return v.split('.').slice(0, 2).join('.')
}

const dbOptions = computed<string[]>(() => {
  const list = payload.value?.databases ?? []
  const names = list.map((d) => d.name)
  const common = ['mysql', 'mariadb', 'postgresql', 'redis', 'mongodb']
  return [...new Set([...names, ...common])]
})

async function loadEnv() {
  loading.value = true
  try {
    const res = await getServerEnv()
    env.value = res.data
  } catch {
    /* 拦截器已提示 */
  } finally {
    loading.value = false
  }
}

async function refresh() {
  refreshing.value = true
  try {
    const res = await refreshServerEnv()
    env.value = res.data
    ElMessage.success(res.message || t('serverEnv.refreshed'))
  } catch {
    /* 拦截器已提示 */
  } finally {
    refreshing.value = false
  }
}

function openDefaultsDialog() {
  const c = conf.value
  form.webserver = c?.webserver ?? ''
  form.php_default = c?.php_default ?? ''
  form.database = c?.database ?? ''
  form.user_home_root = c?.user_home_root || '/home'
  form.container_runtime = c?.container_runtime || 'auto'
  // 回填 fpm 默认规格（先重置再覆盖）
  resetFpmForm()
  const raw = c?.fpm_pool_defaults
  if (raw) {
    try {
      const obj = JSON.parse(raw) as Record<string, unknown>
      Object.keys(fpmNum).forEach((k) => {
        const v = obj[k]
        const n = Number(v)
        if (v !== undefined && v !== null && Number.isFinite(n))
          fpmNum[k as keyof typeof fpmNum] = n
      })
      Object.keys(fpmStr).forEach((k) => {
        const v = obj[k]
        if (v !== undefined && v !== null) fpmStr[k as keyof typeof fpmStr] = String(v)
      })
    } catch {
      /* 非法 JSON 忽略，使用默认 */
    }
  }
  dialogVisible.value = true
}

function resetFpmForm() {
  Object.keys(fpmNum).forEach((k) => {
    fpmNum[k as keyof typeof fpmNum] = FPM_NUM_DEFAULTS[k]
  })
  Object.keys(fpmStr).forEach((k) => {
    fpmStr[k as keyof typeof fpmStr] = FPM_STR_DEFAULTS[k]
  })
}

function fpmSpecJson(): string {
  return JSON.stringify({ ...fpmStr, ...fpmNum })
}

async function saveDefaults() {
  saving.value = true
  try {
    const res = await saveServerEnvDefaults({
      webserver: form.webserver,
      php_default: form.php_default,
      database: form.database,
      fpm_pool_defaults: fpmSpecJson(),
      user_home_root: form.user_home_root.trim(),
      container_runtime: form.container_runtime,
    })
    ElMessage.success(res.message || t('serverEnv.defaultsSaved'))
    dialogVisible.value = false
    loadEnv()
  } catch {
    /* 拦截器已提示 */
  } finally {
    saving.value = false
  }
}

// ── PHP-FPM 规格模板管理 ─────────────────────────────────────

const specs = ref<FpmSpecItem[]>([])
const specsLoading = ref(false)
const specDialogVisible = ref(false)
const specSaving = ref(false)
const editingSpecId = ref<number | null>(null)
const specForm = reactive({ name: '', remark: '' })

async function loadSpecs() {
  specsLoading.value = true
  try {
    const res = await getFpmSpecs()
    specs.value = res.data ?? []
  } catch {
    /* 拦截器已提示 */
  } finally {
    specsLoading.value = false
  }
}

/** 默认新建模板的参考规格（与全局默认同字段集） */
const DEFAULT_TEMPLATE_SPEC = {
  pm: 'dynamic',
  max_children: 16,
  start_servers: 4,
  min_spare_servers: 2,
  max_spare_servers: 8,
  max_requests: 1000,
  request_terminate_timeout: 300,
  max_execution_time: 300,
  memory_limit: '512M',
  post_max_size: '128M',
  upload_max_filesize: '128M',
}

// ── 表格式规格编辑器 ───────────────────────────────────────

type FpmFieldKind = 'pm' | 'number' | 'size'
interface FpmFieldMeta {
  label: string
  kind: FpmFieldKind
  help: string
  min?: number
  max?: number
  options?: string[]
}
/** 与全局默认 pool 规格一致的字段元数据（新增字段按此渲染控件与帮助提示） */
const fpmFields = computed<Record<string, FpmFieldMeta>>(() => ({
  pm: {
    label: t('serverEnv.fpmField.pm'),
    kind: 'pm',
    options: ['dynamic', 'static', 'ondemand'],
    help: t('serverEnv.fpmHelp.pm'),
  },
  max_children: {
    label: t('serverEnv.fpmField.maxChildren'),
    kind: 'number',
    min: 1,
    max: 512,
    help: t('serverEnv.fpmHelp.maxChildren'),
  },
  start_servers: {
    label: t('serverEnv.fpmField.startServers'),
    kind: 'number',
    min: 1,
    max: 128,
    help: t('serverEnv.fpmHelp.startServers'),
  },
  min_spare_servers: {
    label: t('serverEnv.fpmField.minSpare'),
    kind: 'number',
    min: 1,
    max: 128,
    help: t('serverEnv.fpmHelp.minSpare'),
  },
  max_spare_servers: {
    label: t('serverEnv.fpmField.maxSpare'),
    kind: 'number',
    min: 1,
    max: 256,
    help: t('serverEnv.fpmHelp.maxSpare'),
  },
  max_requests: {
    label: t('serverEnv.fpmField.maxRequests'),
    kind: 'number',
    min: 0,
    max: 100000,
    help: t('serverEnv.fpmHelp.maxRequests'),
  },
  request_terminate_timeout: {
    label: t('serverEnv.fpmField.requestTimeout'),
    kind: 'number',
    min: 1,
    max: 86400,
    help: t('serverEnv.fpmHelp.requestTimeout'),
  },
  max_execution_time: {
    label: t('serverEnv.fpmField.maxExecTime'),
    kind: 'number',
    min: 1,
    max: 86400,
    help: t('serverEnv.fpmHelp.maxExecTime'),
  },
  memory_limit: {
    label: t('serverEnv.fpmField.memoryLimit'),
    kind: 'size',
    options: ['128M', '256M', '512M', '1G', '2G'],
    help: t('serverEnv.fpmHelp.memoryLimit'),
  },
  post_max_size: {
    label: t('serverEnv.fpmField.postMax'),
    kind: 'size',
    options: ['64M', '128M', '256M', '512M', '1G'],
    help: t('serverEnv.fpmHelp.postMax'),
  },
  upload_max_filesize: {
    label: t('serverEnv.fpmField.uploadMax'),
    kind: 'size',
    options: ['64M', '128M', '256M', '512M', '1G'],
    help: t('serverEnv.fpmHelp.uploadMax'),
  },
}))

interface SpecRow {
  field: string
  enabled: boolean
  value: string
}

/** 规格表格行 */
const specRows = ref<SpecRow[]>([])
/** 原始 JSON 折叠面板（默认收起，可展开预览 / 粘贴应用） */
const jsonPanel = ref<string[]>([])
const specJsonRaw = ref('')

/** 字段提示（无 meta 返回 null） */
function fpmMeta(field: string): FpmFieldMeta | null {
  return fpmFields.value[field] ?? null
}

/** 字段下拉（预设 + 自定义） */
const fieldOptions = computed(() =>
  Object.keys(fpmFields.value).map((f) => ({
    value: f,
    label: t('serverEnv.fieldOption', { field: f, label: fpmFields.value[f].label }),
  })),
)

/** JSON 文本 → 表格行（未知/旧字段也保留；解析失败则空表） */
function specToRows(jsonText: string) {
  const rows: SpecRow[] = []
  try {
    const obj = JSON.parse(jsonText) as Record<string, unknown>
    const known = Object.keys(fpmFields.value)
    const keys = Object.keys(obj).sort((a, b) => {
      const ia = known.indexOf(a)
      const ib = known.indexOf(b)
      return (ia < 0 ? 999 : ia) - (ib < 0 ? 999 : ib)
    })
    for (const k of keys) {
      const v = obj[k]
      if (v === null || v === undefined) continue
      rows.push({ field: k, enabled: true, value: String(v) })
    }
  } catch {
    /* 非法 JSON：空表由用户自行补充 */
  }
  specRows.value = rows
}

/** 表格行 → 规范化 JSON（仅收录"启用且已填"的行） */
function rowsToSpec(): string {
  const obj: Record<string, unknown> = {}
  for (const r of specRows.value) {
    const f = r.field.trim()
    if (!r.enabled || !f) continue
    const v = r.value.trim()
    if (v === '') continue
    const meta = fpmMeta(f)
    if (meta?.kind === 'number') {
      const n = Number(v)
      if (Number.isFinite(n)) obj[f] = n
    } else {
      obj[f] = v
    }
  }
  return JSON.stringify(obj, null, 2)
}

/** 原始 JSON 美化（供预览/折叠区使用） */
function prettySpec(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}

/** 规格单行摘要（用于列表展示） */
function specPreview(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw))
  } catch {
    return raw
  }
}

function openSpecDialog(row?: FpmSpecItem) {
  const seed = row ? row.spec : JSON.stringify(DEFAULT_TEMPLATE_SPEC, null, 2)
  if (row) {
    editingSpecId.value = row.id
    specForm.name = row.name
    specForm.remark = row.remark
  } else {
    editingSpecId.value = null
    specForm.name = ''
    specForm.remark = ''
  }
  specToRows(seed)
  specJsonRaw.value = prettySpec(seed)
  jsonPanel.value = [] // 原始 JSON 默认折叠
  specDialogVisible.value = true
}

function addSpecRow() {
  specRows.value.push({ field: '', enabled: true, value: '' })
}

function removeSpecRow(idx: number) {
  specRows.value.splice(idx, 1)
}

/** 选中字段后给个顺手默认值 */
function onFieldPicked(row: SpecRow) {
  const meta = fpmMeta(row.field)
  if (!meta || row.value !== '') return
  if (meta.kind === 'number') row.value = String(meta.min ?? 1)
  else if (meta.kind === 'pm') row.value = 'dynamic'
  else if (meta.options?.length) row.value = meta.options[meta.options.length - 1]
}

/** 折叠区：以当前表格重新生成 JSON */
function jsonFromTable() {
  specJsonRaw.value = prettySpec(rowsToSpec())
}

/** 折叠区：粘贴的 JSON 应用回表格 */
function applyJsonToTable() {
  try {
    const obj = JSON.parse(specJsonRaw.value) as Record<string, unknown>
    if (typeof obj !== 'object' || obj === null || Array.isArray(obj)) throw new Error()
    specToRows(specJsonRaw.value)
    ElMessage.success(t('serverEnv.jsonApplied'))
  } catch {
    ElMessage.warning(t('serverEnv.jsonInvalid'))
  }
}

async function saveSpec() {
  const name = specForm.name.trim()
  if (!name) {
    ElMessage.warning(t('serverEnv.needTemplateName'))
    return
  }
  const specRaw = rowsToSpec()
  specSaving.value = true
  try {
    const remark = specForm.remark.trim()
    if (editingSpecId.value === null) {
      const res = await addFpmSpec({ name, spec: specRaw, remark })
      ElMessage.success(res.message || t('serverEnv.specCreated'))
    } else {
      const res = await updateFpmSpec({
        id: editingSpecId.value,
        name,
        spec: specRaw,
        remark,
      })
      ElMessage.success(res.message || t('serverEnv.specUpdated'))
    }
    specDialogVisible.value = false
    loadSpecs()
  } catch {
    /* 拦截器已提示 */
  } finally {
    specSaving.value = false
  }
}

function removeSpec(row: FpmSpecItem) {
  ElMessageBox.confirm(
    t('serverEnv.deleteSpecConfirm', { name: row.name }),
    t('serverEnv.deleteSpecTitle'),
    { type: 'warning', confirmButtonText: t('common.delete') },
  )
    .then(async () => {
      const res = await deleteFpmSpec(row.id)
      ElMessage.success(res.message || t('serverEnv.specDeleted'))
      loadSpecs()
    })
    .catch(() => {
      /* 取消 */
    })
}

onMounted(() => {
  loadEnv()
  loadSpecs()
})
</script>

<template>
  <div class="env-container">
    <el-card shadow="never" v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('serverEnv.title') }}</span>
          <div class="header-actions">
            <el-tag v-if="env?.refreshed" size="small" type="success" style="margin-right: 8px">
              {{ t('serverEnv.autoRefreshed') }}
            </el-tag>
            <span class="detected-at" v-if="payload">
              {{ t('serverEnv.detectedAt', { time: fmtTime(env?.detected_at) }) }}
            </span>
            <el-button type="primary" size="small" :loading="refreshing" @click="refresh">
              {{ t('serverEnv.redetect') }}
            </el-button>
            <el-button size="small" @click="openDefaultsDialog">
              {{ t('serverEnv.defaults') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-alert
        v-if="env?.error"
        :title="t('serverEnv.probeFailed', { error: env.error })"
        type="warning"
        :closable="false"
        show-icon
        style="margin-bottom: 16px"
      />

      <template v-if="payload">
        <!-- 操作系统 -->
        <el-descriptions :title="t('serverEnv.osTitle')" :column="2" border class="env-section">
          <el-descriptions-item :label="t('serverEnv.hostname')">
            {{ payload.hostname || '--' }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('serverEnv.system')">
            {{ payload.os.name }} {{ payload.os.version }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('serverEnv.kernel')">
            {{ payload.os.kernel || '--' }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('serverEnv.arch')">
            {{ payload.os.arch || '--' }}
          </el-descriptions-item>
        </el-descriptions>

        <!-- Web 服务器 -->
        <el-descriptions
          :title="t('serverEnv.webserverTitle')"
          :column="1"
          border
          class="env-section"
        >
          <el-descriptions-item :label="t('common.type')">
            <template v-if="payload.webserver?.flavor && payload.webserver.flavor !== 'none'">
              <el-tag
                :type="payload.webserver.flavor === 'openresty' ? 'warning' : 'success'"
                size="small"
              >
                {{ payload.webserver.flavor }}
              </el-tag>
              <el-tag size="small" style="margin-left: 8px"
                >v{{ payload.webserver.version || '--' }}</el-tag
              >
              <el-tag
                size="small"
                :type="payload.webserver.running ? 'success' : 'info'"
                style="margin-left: 8px"
              >
                {{ payload.webserver.running ? t('serverEnv.running') : t('serverEnv.notRunning') }}
              </el-tag>
            </template>
            <el-tag v-else size="small" type="info">{{ t('serverEnv.noWebserver') }}</el-tag>
          </el-descriptions-item>
          <el-descriptions-item v-if="payload.webserver?.binary" :label="t('serverEnv.binary')">
            {{ payload.webserver.binary }}
          </el-descriptions-item>
          <el-descriptions-item v-if="payload.webserver?.conf" :label="t('serverEnv.mainConf')">
            {{ payload.webserver.conf }}
          </el-descriptions-item>
          <el-descriptions-item
            v-if="payload.webserver?.sites_dir"
            :label="t('serverEnv.sitesDir')"
          >
            {{ payload.webserver.sites_dir }}
          </el-descriptions-item>
        </el-descriptions>

        <!-- PHP -->
        <div class="env-section">
          <div class="section-title">
            PHP
            <el-tag
              v-if="payload.php?.default"
              size="small"
              type="primary"
              style="margin-left: 8px"
            >
              {{ t('serverEnv.defaultTag', { v: payload.php.default }) }}
            </el-tag>
          </div>
          <el-table
            :data="payload.php?.instances ?? []"
            size="small"
            border
            style="margin-top: 8px"
          >
            <el-table-column :label="t('serverEnv.phpVersion')" width="110">
              <template #default="{ row }">
                <el-tag v-if="row.default" type="primary" size="small">{{ row.version }}</el-tag>
                <span v-else>{{ row.version }}</span>
              </template>
            </el-table-column>
            <el-table-column prop="binary" :label="t('serverEnv.binary')" show-overflow-tooltip />
            <el-table-column prop="socket" label="FPM Socket" show-overflow-tooltip>
              <template #default="{ row }">{{ row.socket || '--' }}</template>
            </el-table-column>
            <el-table-column :label="t('common.status')" width="90">
              <template #default="{ row }">
                <el-tag :type="row.running ? 'success' : 'info'" size="small">
                  {{ row.running ? t('serverEnv.running') : t('serverEnv.notRunning') }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>
        </div>

        <!-- 数据库 -->
        <div class="env-section">
          <div class="section-title">{{ t('serverEnv.dbTitle') }}</div>
          <el-table :data="payload.databases ?? []" size="small" border style="margin-top: 8px">
            <el-table-column :label="t('serverEnv.instance')" width="160">
              <template #default="{ row }">
                <el-tag size="small">{{ row.name }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="version" :label="t('serverEnv.phpVersion')" />
            <el-table-column prop="binary" :label="t('serverEnv.path')" show-overflow-tooltip />
            <el-table-column :label="t('common.status')" width="110">
              <template #default="{ row }">
                <el-tag :type="row.running ? 'success' : 'info'" size="small">
                  {{ row.running ? t('serverEnv.running') : t('serverEnv.notRunning') }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>
        </div>

        <!-- 工具链 -->
        <div class="env-section">
          <div class="section-title">{{ t('serverEnv.toolsTitle') }}</div>
          <el-table :data="payload.tools ?? []" size="small" border style="margin-top: 8px">
            <el-table-column :label="t('common.name')" width="160">
              <template #default="{ row }">
                <el-tag size="small" type="info">{{ row.name }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="version" :label="t('serverEnv.phpVersion')" />
          </el-table>
        </div>
      </template>

      <el-empty v-else :description="t('serverEnv.empty')" :image-size="80" />
    </el-card>

    <!-- PHP-FPM 规格模板 -->
    <el-card shadow="never" class="spec-card" v-loading="specsLoading">
      <template #header>
        <div class="card-header">
          <div>
            <span>{{ t('serverEnv.specTitle') }}</span>
            <span class="card-sub">{{ t('serverEnv.specSub') }}</span>
          </div>
          <el-button type="primary" size="small" @click="openSpecDialog()">
            {{ t('serverEnv.addSpec') }}
          </el-button>
        </div>
      </template>
      <el-alert
        :title="t('serverEnv.specNamingAlert')"
        type="info"
        :closable="false"
        show-icon
        style="margin-bottom: 12px"
      />
      <el-table :data="specs" size="small" border style="width: 100%">
        <el-table-column :label="t('serverEnv.templateName')" width="220">
          <template #default="{ row }">
            <el-tag :type="row.owner ? 'success' : 'info'" size="small">{{ row.name }}</el-tag>
            <el-tag v-if="row.owner" type="warning" size="small" style="margin-left: 6px">
              {{ t('serverEnv.ownerUnder', { owner: row.owner }) }}
            </el-tag>
            <el-tag v-else type="info" size="small" style="margin-left: 6px" effect="plain">
              {{ t('serverEnv.globalTag') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('serverEnv.specSummary')" min-width="240">
          <template #default="{ row }">
            <el-popover placement="top-start" :width="420" trigger="click">
              <template #reference>
                <span class="spec-preview-trigger" :title="t('serverEnv.previewTip')">
                  {{ specPreview(row.spec) }}
                </span>
              </template>
              <template #default>
                <div class="spec-popover-head">
                  <el-tag size="small" :type="row.owner ? 'success' : 'info'">{{
                    row.name
                  }}</el-tag>
                  <span class="spec-popover-sub">{{ row.remark || '—' }}</span>
                  <el-button link type="primary" size="small" @click="openSpecDialog(row)">
                    {{ t('common.edit') }}
                  </el-button>
                </div>
                <pre class="spec-json-view">{{ prettySpec(row.spec) }}</pre>
              </template>
            </el-popover>
          </template>
        </el-table-column>
        <el-table-column
          prop="remark"
          :label="t('common.remark')"
          min-width="140"
          show-overflow-tooltip
        />
        <el-table-column :label="t('common.updatedAt')" width="160">
          <template #default="{ row }">{{ fmtTime(row.updated_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="130" align="center">
          <template #default="{ row }">
            <el-button link type="primary" size="small" @click="openSpecDialog(row)">
              {{ t('common.edit') }}
            </el-button>
            <el-button link type="danger" size="small" @click="removeSpec(row)">
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 规格模板编辑 -->
    <el-dialog
      v-model="specDialogVisible"
      :title="editingSpecId === null ? t('serverEnv.addSpecTitle') : t('serverEnv.editSpecTitle')"
      width="800px"
      top="6vh"
      destroy-on-close
    >
      <el-form label-width="110px" @submit.prevent>
        <el-form-item :label="t('serverEnv.templateName')" required>
          <el-input
            v-model="specForm.name"
            :placeholder="t('serverEnv.templateNamePlaceholder')"
            maxlength="64"
            show-word-limit
          />
          <div class="form-tip">{{ t('serverEnv.templateNameTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('serverEnv.specFields')">
          <el-table :data="specRows" size="small" border style="width: 100%">
            <el-table-column :label="t('common.enable')" width="56" align="center">
              <template #default="{ row }">
                <el-checkbox v-model="row.enabled" />
              </template>
            </el-table-column>
            <el-table-column :label="t('serverEnv.fieldCol')" width="250">
              <template #default="{ row }">
                <el-select
                  v-model="row.field"
                  filterable
                  allow-create
                  default-first-option
                  :placeholder="t('serverEnv.fieldPlaceholder')"
                  style="width: 100%"
                  @change="onFieldPicked(row)"
                >
                  <el-option
                    v-for="opt in fieldOptions"
                    :key="opt.value"
                    :label="opt.label"
                    :value="opt.value"
                  />
                </el-select>
              </template>
            </el-table-column>
            <el-table-column :label="t('serverEnv.valueCol')" min-width="200">
              <template #default="{ row }">
                <div class="value-cell">
                  <div class="value-control">
                    <el-select
                      v-if="fpmMeta(row.field)?.kind === 'pm'"
                      v-model="row.value"
                      :placeholder="t('serverEnv.pmPlaceholder')"
                      style="width: 100%"
                    >
                      <el-option
                        v-for="v in ['dynamic', 'static', 'ondemand']"
                        :key="v"
                        :label="v"
                        :value="v"
                      />
                    </el-select>
                    <el-input-number
                      v-else-if="fpmMeta(row.field)?.kind === 'number'"
                      style="width: 100%"
                      :min="fpmMeta(row.field)?.min ?? 0"
                      :max="fpmMeta(row.field)?.max"
                      :model-value="Number(row.value) || (fpmMeta(row.field)?.min ?? 0)"
                      @update:model-value="(v?: number) => (row.value = v == null ? '' : String(v))"
                      controls-position="right"
                    />
                    <el-select
                      v-else-if="fpmMeta(row.field)?.kind === 'size'"
                      v-model="row.value"
                      filterable
                      allow-create
                      default-first-option
                      :placeholder="t('serverEnv.sizePlaceholder')"
                      style="width: 100%"
                    >
                      <el-option
                        v-for="v in fpmMeta(row.field)?.options ?? []"
                        :key="v"
                        :label="v"
                        :value="v"
                      />
                    </el-select>
                    <el-input
                      v-else
                      v-model="row.value"
                      :placeholder="t('serverEnv.valuePlaceholder')"
                    />
                  </div>
                  <el-tooltip
                    v-if="fpmMeta(row.field)"
                    :content="fpmMeta(row.field)?.help ?? ''"
                    placement="top"
                    :show-after="150"
                    popper-class="field-help-pop"
                  >
                    <span class="field-help">?</span>
                  </el-tooltip>
                </div>
              </template>
            </el-table-column>
            <el-table-column label="" width="60" align="center">
              <template #default="{ $index }">
                <el-button link type="danger" size="small" @click="removeSpecRow($index)">
                  {{ t('serverEnv.removeRow') }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
          <div class="field-toolbar">
            <el-button size="small" type="primary" plain @click="addSpecRow">
              {{ t('serverEnv.addField') }}
            </el-button>
            <span class="form-tip">{{ t('serverEnv.specFieldsTip') }}</span>
          </div>
          <el-collapse v-model="jsonPanel" class="json-collapse">
            <el-collapse-item :title="t('serverEnv.rawJsonPanel')" name="json">
              <el-input
                v-model="specJsonRaw"
                type="textarea"
                :rows="10"
                class="spec-editor"
                spellcheck="false"
              />
              <div class="json-actions">
                <el-button size="small" @click="jsonFromTable">
                  {{ t('serverEnv.genFromTable') }}
                </el-button>
                <el-button size="small" type="primary" plain @click="applyJsonToTable">
                  {{ t('serverEnv.applyJson') }}
                </el-button>
              </div>
            </el-collapse-item>
          </el-collapse>
        </el-form-item>
        <el-form-item :label="t('common.remark')">
          <el-input
            v-model="specForm.remark"
            :placeholder="t('serverEnv.remarkPlaceholder')"
            maxlength="200"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="specDialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="specSaving" @click="saveSpec">
          {{ t('common.save') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 全局默认配置 -->
    <el-dialog v-model="dialogVisible" :title="t('serverEnv.defaultsTitle')" width="640px">
      <el-form label-width="130px" @submit.prevent>
        <el-form-item :label="t('serverEnv.defaultWebserver')">
          <el-select
            v-model="form.webserver"
            clearable
            :placeholder="t('serverEnv.followAuto')"
            style="width: 100%"
          >
            <el-option :label="t('serverEnv.followAutoOption')" value="" />
            <el-option label="nginx" value="nginx" />
            <el-option label="openresty" value="openresty" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('serverEnv.defaultPhp')">
          <el-select
            v-model="form.php_default"
            clearable
            filterable
            allow-create
            default-first-option
            :placeholder="t('serverEnv.phpUnset')"
            style="width: 100%"
          >
            <el-option v-for="v in phpOptions" :key="v" :label="v" :value="v" />
          </el-select>
          <div class="form-tip">{{ t('serverEnv.defaultPhpTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('serverEnv.defaultDb')">
          <el-select
            v-model="form.database"
            clearable
            filterable
            allow-create
            default-first-option
            :placeholder="t('serverEnv.unset')"
            style="width: 100%"
          >
            <el-option v-for="d in dbOptions" :key="d" :label="d" :value="d" />
          </el-select>
        </el-form-item>

        <el-divider content-position="left">{{ t('serverEnv.homeMountTitle') }}</el-divider>
        <el-form-item :label="t('serverEnv.mountPoint')">
          <el-input v-model="form.user_home_root" placeholder="/home" style="max-width: 360px" />
          <div class="form-tip">{{ t('serverEnv.homeRootTip') }}</div>
        </el-form-item>

        <el-divider content-position="left">
          {{ t('serverEnv.containerRuntimeTitle') }}
        </el-divider>
        <el-form-item :label="t('serverEnv.containerRuntime')">
          <el-select v-model="form.container_runtime" style="max-width: 360px">
            <el-option :label="t('serverEnv.runtimeAuto')" value="auto" />
            <el-option label="Docker" value="docker" />
            <el-option label="Podman" value="podman" />
          </el-select>
          <div class="form-tip">{{ t('serverEnv.containerRuntimeTip') }}</div>
        </el-form-item>

        <el-divider content-position="left">{{ t('serverEnv.fpmDefaultsTitle') }}</el-divider>
        <el-form-item :label="t('serverEnv.fpmField.pm')">
          <el-radio-group v-model="fpmStr.pm">
            <el-radio value="dynamic">{{ t('serverEnv.pmDynamic') }}</el-radio>
            <el-radio value="static">{{ t('serverEnv.pmStatic') }}</el-radio>
            <el-radio value="ondemand">{{ t('serverEnv.pmOndemand') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item v-if="fpmStr.pm !== 'ondemand'" :label="t('serverEnv.maxChildren')">
          <el-input-number
            v-model="fpmNum.max_children"
            :min="1"
            :max="512"
            controls-position="right"
          />
          <div class="form-tip">{{ t('serverEnv.maxChildrenTip') }}</div>
        </el-form-item>
        <el-form-item v-if="fpmStr.pm === 'dynamic'" :label="t('serverEnv.fpmField.startServers')">
          <el-input-number
            v-model="fpmNum.start_servers"
            :min="1"
            :max="128"
            controls-position="right"
          />
        </el-form-item>
        <el-form-item v-if="fpmStr.pm === 'dynamic'" :label="t('serverEnv.spareRange')">
          <el-input-number
            v-model="fpmNum.min_spare_servers"
            :min="1"
            :max="128"
            controls-position="right"
          />
          <span style="margin: 0 8px; color: var(--el-text-color-secondary)">~</span>
          <el-input-number
            v-model="fpmNum.max_spare_servers"
            :min="1"
            :max="256"
            controls-position="right"
          />
        </el-form-item>
        <el-form-item :label="t('serverEnv.maxRequests')">
          <el-input-number
            v-model="fpmNum.max_requests"
            :min="0"
            :max="100000"
            controls-position="right"
          />
          <div class="form-tip">{{ t('serverEnv.maxRequestsTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('serverEnv.fpmField.requestTimeout')">
          <el-input-number
            v-model="fpmNum.request_terminate_timeout"
            :min="1"
            :max="86400"
            controls-position="right"
          />
        </el-form-item>
        <el-form-item :label="t('serverEnv.fpmField.memoryLimit')">
          <el-select
            v-model="fpmStr.memory_limit"
            filterable
            allow-create
            default-first-option
            style="width: 180px"
          >
            <el-option
              v-for="m in ['128M', '256M', '512M', '1G', '2G']"
              :key="m"
              :label="m"
              :value="m"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('serverEnv.fpmField.uploadMax')">
          <el-select
            v-model="fpmStr.upload_max_filesize"
            filterable
            allow-create
            default-first-option
            style="width: 180px"
          >
            <el-option
              v-for="m in ['64M', '128M', '256M', '512M', '1G']"
              :key="m"
              :label="m"
              :value="m"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('serverEnv.fpmField.postMax')">
          <el-select
            v-model="fpmStr.post_max_size"
            filterable
            allow-create
            default-first-option
            style="width: 180px"
          >
            <el-option
              v-for="m in ['64M', '128M', '256M', '512M', '1G']"
              :key="m"
              :label="m"
              :value="m"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('serverEnv.fpmField.maxExecTime')">
          <el-input-number
            v-model="fpmNum.max_execution_time"
            :min="1"
            :max="86400"
            controls-position="right"
          />
        </el-form-item>
        <el-form-item label=" ">
          <el-button size="small" @click="resetFpmForm">{{ t('serverEnv.resetSpec') }}</el-button>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="saveDefaults">
          {{ t('common.save') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.env-container {
  padding: 0;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.header-actions {
  display: flex;
  align-items: center;
}
.detected-at {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  margin-right: 12px;
}
.env-section {
  margin-top: 20px;
}
.section-title {
  font-weight: 600;
  color: var(--el-text-color-primary);
  display: flex;
  align-items: center;
}
.form-tip {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.6;
}
.spec-card {
  margin-top: 20px;
}
.card-sub {
  margin-left: 8px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.spec-preview-trigger {
  display: inline-block;
  max-width: 100%;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace;
  font-size: 12px;
  color: var(--el-color-primary);
  cursor: pointer;
}
.spec-preview-trigger:hover {
  text-decoration: underline;
}
.spec-editor :deep(textarea) {
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace;
  font-size: 12px;
}
.field-toolbar {
  margin-top: 10px;
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}
.json-collapse {
  margin-top: 10px;
  border-top: 1px dashed var(--el-border-color-lighter);
}
.json-actions {
  margin-top: 8px;
  display: flex;
  gap: 8px;
}
.value-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
}
.value-control {
  flex: 1;
  min-width: 0;
}
.field-help {
  flex: none;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--el-color-primary-light-5);
  color: #fff;
  font-size: 11px;
  line-height: 16px;
  text-align: center;
  cursor: help;
  user-select: none;
}
.field-help:hover {
  background: var(--el-color-primary);
}

/* popover / tooltip 内容渲染在 body（teleport），需全局样式 */
:global(.spec-json-view) {
  margin: 0;
  max-height: 260px;
  overflow: auto;
  padding: 8px 10px;
  background: var(--el-fill-color-lighter);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace;
  font-size: 12px;
  line-height: 1.5;
  color: var(--el-text-color-regular);
  white-space: pre-wrap;
  word-break: break-all;
}
:global(.spec-popover-head) {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
:global(.spec-popover-sub) {
  flex: 1;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
:global(.field-help-pop) {
  max-width: 360px;
  line-height: 1.6;
  white-space: normal;
  word-break: break-word;
}
</style>
