<template>
  <div v-loading="bootLoading" class="service-conf">
    <!-- 顶部状态与操作 -->
    <el-card shadow="never" class="top-card">
      <div class="top-row">
        <div class="title">
          <span class="t-name">{{ label }}</span>
          <el-tag v-if="engineName" size="small" type="warning">{{ engineName }}</el-tag>
          <el-tag v-if="status.installed" size="small" :type="running ? 'success' : 'danger'">
            {{ running ? t('servicesCommon.running') : t('servicesCommon.notRunning') }}
          </el-tag>
          <el-tag v-if="version" size="small" type="info">{{ version }}</el-tag>
          <el-tag v-if="status.unit" size="small" type="warning">systemd</el-tag>
        </div>
        <div v-if="status.installed" class="actions">
          <span v-if="status.conf_file" class="path mono">{{ status.conf_file }}</span>
          <el-button size="small" :loading="busy" @click="refreshAll">{{
            t('servicesCommon.refresh')
          }}</el-button>
          <el-button
            size="small"
            type="primary"
            plain
            :disabled="!running || busy"
            :loading="acting === 'reload'"
            @click="doControl('reload')"
            >{{ t('servicesCommon.reload') }}</el-button
          >
          <el-button
            size="small"
            type="warning"
            plain
            :disabled="!running || busy"
            :loading="acting === 'restart'"
            @click="doControl('restart')"
            >{{ t('servicesCommon.restart') }}</el-button
          >
          <el-button
            size="small"
            type="success"
            plain
            :disabled="running || busy"
            :loading="acting === 'start'"
            @click="doControl('start')"
            >{{ t('servicesCommon.start') }}</el-button
          >
        </div>
      </div>
      <div v-if="desc && status.installed" class="desc">{{ desc }}</div>
      <div v-if="details" class="desc details">{{ details }}</div>
    </el-card>

    <!-- 未安装引导 -->
    <el-card v-if="status.installed === false && !bootLoading" shadow="never" class="mt-3">
      <el-result
        icon="warning"
        :title="t('servicesCommon.notInstalledTitle', { label })"
        :sub-title="t('servicesCommon.notInstalledSub', { label, hint: installHint })"
      >
        <template #extra>
          <el-button type="primary" @click="router.push('/appstore')">{{
            t('servicesCommon.goAppstore')
          }}</el-button>
        </template>
      </el-result>
    </el-card>

    <template v-if="status.installed">
      <el-tabs v-model="mode" type="border-card" class="mt-3">
        <!-- 关键配置 -->
        <el-tab-pane :label="t('servicesCommon.tabKeys')" name="keys">
          <div v-if="keysData.fields.length" class="visual-body">
            <el-alert
              type="info"
              :closable="false"
              show-icon
              class="visual-tip"
              :title="t('servicesCommon.keysTip')"
            />
            <div class="field-grid">
              <div v-for="f in keysData.fields" :key="f.key" class="field">
                <div class="field-label">
                  <span class="mono">{{ f.key }}</span>
                  <el-tag v-if="f.section" size="small" type="info">[{{ f.section }}]</el-tag>
                </div>
                <el-input
                  v-if="f.kind === 'text'"
                  v-model="visual[f.key]"
                  :placeholder="t('servicesCommon.emptyValue')"
                  clearable
                  class="field-ctrl"
                />
                <el-input
                  v-else-if="f.kind === 'number'"
                  v-model="visual[f.key]"
                  :placeholder="t('servicesCommon.emptyValue')"
                  clearable
                  class="field-ctrl"
                />
                <!-- list：配置里是数组，表单里一行一项 -->
                <el-input
                  v-else-if="f.kind === 'list'"
                  v-model="visual[f.key]"
                  type="textarea"
                  :rows="3"
                  :placeholder="t('servicesCommon.emptyValue')"
                  class="field-ctrl"
                />
                <el-select
                  v-else
                  v-model="visual[f.key]"
                  :placeholder="t('servicesCommon.emptyValue')"
                  clearable
                  class="field-ctrl"
                >
                  <el-option
                    v-for="opt in f.options || []"
                    :key="opt"
                    :label="
                      opt === 'true'
                        ? t('servicesCommon.optionOn')
                        : opt === 'false'
                          ? t('servicesCommon.optionOff')
                          : opt
                    "
                    :value="opt"
                  />
                </el-select>
                <div v-if="f.help" class="field-tip">{{ f.help }}</div>
              </div>
            </div>
            <div class="save-row">
              <el-button :loading="savingKeys" type="primary" @click="saveKeys">{{
                t('servicesCommon.saveKeys')
              }}</el-button>
              <el-button :disabled="savingKeys" @click="loadKeys(true)">{{
                t('servicesCommon.reloadFromFile')
              }}</el-button>
            </div>
          </div>
          <el-empty v-else :description="t('servicesCommon.noKeys')" />
        </el-tab-pane>

        <!-- 配置文件编辑 -->
        <el-tab-pane :label="t('servicesCommon.tabFiles')" name="files">
          <el-alert
            v-if="listData.installed && listData.main_exists === false"
            type="warning"
            show-icon
            :closable="false"
            class="detect-warn"
            :title="
              t('servicesCommon.mainConfMissing', {
                path: listData.conf_file || t('servicesCommon.noPath'),
              })
            "
            :description="detectHint"
          />
          <div v-if="confFiles.length" class="editor-layout">
            <div class="file-list">
              <div class="list-head">
                <span>{{ t('servicesCommon.editableConfs') }}</span>
                <el-tag size="small" type="info">{{ confFiles.length }}</el-tag>
              </div>
              <el-scrollbar class="list-scroll">
                <div
                  v-for="f in confFiles"
                  :key="f.path"
                  class="file-item"
                  :class="{ active: activeFile === f.path, 'no-exist': !f.exists }"
                  @click="selectFile(f.path)"
                >
                  <div class="file-name">
                    <el-icon :size="14"><Document /></el-icon>
                    <span>{{ f.rel }}</span>
                  </div>
                  <div class="file-meta">
                    <el-tag v-if="f.is_main" size="small" type="warning">{{
                      t('servicesCommon.mainConfTag')
                    }}</el-tag>
                    <el-tag v-if="!f.exists" size="small" type="danger">{{
                      t('servicesCommon.notExist')
                    }}</el-tag>
                    <span class="mono">{{ f.size > 0 ? formatBytes(f.size) : '-' }}</span>
                  </div>
                </div>
              </el-scrollbar>
            </div>
            <div class="editor-panel">
              <div v-loading="fileLoading" class="editor-wrap">
                <div v-if="editorContent !== null" class="editor-head">
                  <span class="mono path">{{ activeFile }}</span>
                  <el-tag v-if="fileMissing" size="small" type="danger">{{
                    t('servicesCommon.fileWillCreate')
                  }}</el-tag>
                  <div class="editor-actions">
                    <el-button size="small" :disabled="!dirty" @click="reloadFile">{{
                      t('servicesCommon.discardChanges')
                    }}</el-button>
                    <el-button
                      size="small"
                      type="primary"
                      :disabled="!dirty || savingFile"
                      :loading="savingFile"
                      @click="saveFile"
                      >{{ t('servicesCommon.saveFile') }}</el-button
                    >
                  </div>
                </div>
                <CodeEditor
                  v-if="editorContent !== null"
                  v-model="editorContent"
                  :lang="editorLang"
                />
              </div>
            </div>
          </div>
          <el-empty v-else :description="t('servicesCommon.noConfFiles')" />
        </el-tab-pane>
      </el-tabs>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Document } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import { formatBytes } from '@/utils/fmt'
import {
  controlServiceConf,
  getServiceConfKeys,
  getServiceConfList,
  getServiceConfRead,
  getServiceConfStatus,
  saveServiceConf,
  saveServiceConfKeys,
  type ServiceConfField,
  type ServiceConfFile,
  type ServiceConfKeysData,
  type ServiceConfListData,
  type ServiceConfStatus,
} from '@/api/servicesConf.ts'

const props = defineProps<{
  service: string
  label: string
  desc?: string
  installHint?: string
}>()

const { t } = useI18n()
const router = useRouter()
const label = computed(() => props.label)
const desc = computed(() => props.desc || '')

const bootLoading = ref(false)
const busy = ref(false)
const acting = ref('')
const status = ref<ServiceConfStatus>({})
const listData = ref<ServiceConfListData>({ installed: false, files: [] })
const keysData = ref<ServiceConfKeysData>({ installed: false, fields: [], values: {} })

const mode = ref('keys')

const running = computed(() => !!status.value.running)
const version = computed(() => {
  const v = status.value.version || ''
  if (!v) return ''
  // docker/php/mysql 输出首行含版本号，压缩显示
  const m = v.match(/(?:PHP\s+)?([\d]+\.[\d]+(?:\.[\d]+)?|Docker\s+version\s+[\d.]+)/i)
  return m ? m[1] : v
})
/** 自动识别的数据库引擎显示名（MySQL / MariaDB 服务：mysql | mariadb） */
const engineName = computed(() =>
  status.value.engine === 'mariadb' ? 'MariaDB' : status.value.engine === 'mysql' ? 'MySQL' : '',
)
/**
 * 安装信息明细（仅后端返回 dir 的服务展示，目前即 MySQL / MariaDB）：
 * 安装目录 / 主配置 / systemd 单元 + 保存生效提示，与 PHP 实例页 desc 风格一致。
 */
const details = computed(() => {
  const dir = status.value.dir
  if (!dir || !status.value.installed) return ''
  const parts = [t('servicesCommon.installDir', { path: dir })]
  if (status.value.conf_file)
    parts.push(t('servicesCommon.confPath', { path: status.value.conf_file }))
  if (status.value.unit) parts.push(t('servicesCommon.unitPath', { path: status.value.unit }))
  return parts.join(t('servicesCommon.separator')) + t('servicesCommon.detailsSuffix')
})

// ── 关键配置表单 ──────────────────────────────
const visual = reactive<Record<string, string>>({})

function resetVisualFromValues(values: Record<string, string | number | boolean | null>) {
  for (const k of Object.keys(visual)) delete visual[k]
  for (const [k, v] of Object.entries(values)) {
    if (v === null || v === undefined) {
      visual[k] = ''
    } else if (typeof v === 'boolean') {
      visual[k] = v ? 'true' : 'false'
    } else {
      visual[k] = String(v)
    }
  }
}

const savingKeys = ref(false)

async function saveKeys() {
  const payload: Record<string, string> = {}
  for (const f of keysData.value.fields) {
    payload[f.key] = (visual[f.key] || '').trim()
  }
  savingKeys.value = true
  try {
    const res = await saveServiceConfKeys(props.service, payload)
    ElMessage.success(res.data?.reason || t('servicesCommon.keysSaved'))
    await loadKeys(true)
    await loadStatus()
  } catch {
    /* interceptor 已提示 */
  } finally {
    savingKeys.value = false
  }
}

// ── 文件编辑 ────────────────────────────────
const confFiles = ref<ServiceConfFile[]>([])
const activeFile = ref('')
const editorContent = ref<string | null>(null)
const originalContent = ref('')
const fileLoading = ref(false)
const savingFile = ref(false)
/** 当前选中的配置文件在磁盘上不存在（探测失败或尚未创建） */
const fileMissing = ref(false)

/** 未检测到主配置时的排查提示：把后端尝试过的候选路径展示出来 */
const detectHint = computed(() => {
  const tried = status.value.conf_candidates?.length
    ? status.value.conf_candidates.join(t('servicesCommon.nameSeparator'))
    : ''
  const base = tried ? t('servicesCommon.detectTried', { tried }) : ''
  return base + t('servicesCommon.detectReason')
})

const dirty = computed(
  () => editorContent.value !== null && editorContent.value !== originalContent.value,
)
const editorLang = computed(() => {
  const name = activeFile.value.toLowerCase()
  if (name.endsWith('.json')) return 'json'
  if (name.endsWith('.ini') || name.endsWith('.cnf') || name.endsWith('.conf')) return 'ini'
  return 'text'
})

async function selectFile(path: string, force = false) {
  if (dirty.value && !force) {
    try {
      await ElMessageBox.confirm(t('servicesCommon.unsavedSwitch'), t('common.tip'), {
        type: 'warning',
      })
    } catch {
      return
    }
  }
  if (!path) return
  activeFile.value = path
  fileLoading.value = true
  try {
    const res = await getServiceConfRead(props.service, path)
    editorContent.value = res.data.content
    originalContent.value = res.data.content
    fileMissing.value = !!res.data.missing
  } catch {
    editorContent.value = null
    originalContent.value = ''
  } finally {
    fileLoading.value = false
  }
}

async function reloadFile() {
  if (dirty.value) {
    try {
      await ElMessageBox.confirm(t('servicesCommon.reloadConfirm'), t('common.tip'), {
        type: 'warning',
      })
    } catch {
      return
    }
  }
  await selectFile(activeFile.value, true)
}

async function saveFile() {
  if (editorContent.value === null || !activeFile.value) return
  savingFile.value = true
  try {
    const res = await saveServiceConf(props.service, activeFile.value, editorContent.value)
    ElMessage.success(res.data?.reason || t('servicesCommon.confSaved'))
    originalContent.value = editorContent.value
    await loadList()
  } catch {
    /* interceptor 已提示 */
  } finally {
    savingFile.value = false
  }
}

// ── 控制 ────────────────────────────────────
const controlLabels = computed<Record<string, string>>(() => ({
  reload: t('servicesCommon.reload'),
  restart: t('servicesCommon.restart'),
  start: t('servicesCommon.start'),
}))

async function doControl(action: 'reload' | 'restart' | 'start') {
  const warn = action === 'restart'
  try {
    await ElMessageBox.confirm(
      t('servicesCommon.controlConfirm', {
        label: label.value,
        action: controlLabels.value[action],
        warn: warn ? t('servicesCommon.restartWarn') : '',
      }),
      t('common.tip'),
      { type: warn ? 'warning' : 'info' },
    )
  } catch {
    return
  }
  acting.value = action
  try {
    const res = await controlServiceConf(props.service, action)
    ElMessage.success(res.message || t('servicesCommon.opSuccess'))
    await new Promise((r) => setTimeout(r, 500))
    await loadStatus()
  } catch {
    /* interceptor 已提示 */
  } finally {
    acting.value = ''
  }
}

// ── 加载 ────────────────────────────────────
async function loadStatus() {
  busy.value = true
  try {
    const res = await getServiceConfStatus(props.service)
    status.value = res.data
  } catch {
    /* interceptor 已提示 */
  } finally {
    busy.value = false
  }
}

async function loadList() {
  try {
    const res = await getServiceConfList(props.service)
    listData.value = res.data
    confFiles.value = res.data.files
    const current = activeFile.value
    if (!current || !res.data.files.some((f) => f.path === current)) {
      const main = res.data.files.find((f) => f.is_main) || res.data.files[0]
      if (main) await selectFile(main.path, true)
    }
  } catch {
    /* interceptor 已提示 */
  }
}

async function loadKeys(silent = false) {
  try {
    const res = await getServiceConfKeys(props.service)
    keysData.value = res.data
    resetVisualFromValues(res.data.values)
  } catch {
    if (!silent) ElMessage.error(t('servicesCommon.readKeysFailed'))
  }
}

async function refreshAll() {
  bootLoading.value = true
  try {
    await loadStatus()
    if (status.value.installed) {
      await Promise.all([loadKeys(true), loadList()])
    }
  } finally {
    bootLoading.value = false
  }
}

// 监听未安装 → 安装（如用户在应用商店安装后返回）
watch(
  () => status.value.installed,
  (v) => {
    if (v) void refreshAll()
  },
)

onMounted(refreshAll)
</script>

<style scoped>
.service-conf {
  min-height: 300px;
}
.top-card {
  border-radius: 8px;
}
.top-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.title {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.t-name {
  font-size: 16px;
  font-weight: 600;
}
.actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.path {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
}
.desc {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.mt-3 {
  margin-top: 12px;
}
.detect-warn {
  margin-top: 12px;
}
/* 关键配置表单 */
.visual-body {
  padding: 8px 4px;
}
.visual-tip {
  margin-bottom: 16px;
}
.field-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 16px 20px;
}
.field {
  min-width: 0;
}
.field-label {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
}
.field-ctrl {
  width: 100%;
}
.field-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-top: 4px;
  line-height: 1.5;
}
.save-row {
  margin-top: 18px;
  display: flex;
  gap: 10px;
}
/* 文件编辑器 */
.editor-layout {
  display: flex;
  height: 560px;
  overflow: hidden;
}
.file-list {
  width: 260px;
  border-right: 1px solid var(--el-border-color-lighter);
  display: flex;
  flex-direction: column;
}
.list-head {
  padding: 10px 12px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  border-bottom: 1px solid var(--el-border-color-lighter);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.list-scroll {
  flex: 1;
}
.file-item {
  padding: 8px 12px;
  cursor: pointer;
  border-bottom: 1px solid var(--el-border-color-lighter);
  transition: background 0.15s;
}
.file-item:hover {
  background: var(--el-fill-color-light);
}
.file-item.active {
  background: var(--el-color-primary-light-9);
}
.file-item.no-exist {
  opacity: 0.7;
}
.file-name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  word-break: break-all;
}
.file-name .el-icon {
  color: var(--el-text-color-secondary);
}
.file-meta {
  margin-top: 4px;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.editor-panel {
  flex: 1;
  display: flex;
  min-width: 0;
}
.editor-wrap {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.editor-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.editor-head .path {
  max-width: 70%;
}
.editor-actions {
  display: flex;
  gap: 8px;
}
</style>
