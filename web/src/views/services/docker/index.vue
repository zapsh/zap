<template>
  <div v-loading="bootLoading" class="docker-conf">
    <!-- 顶部状态与操作 -->
    <el-card shadow="never" class="top-card">
      <div class="top-row">
        <div class="title">
          <span class="t-name">Docker</span>
          <el-tag v-if="installed" size="small" :type="running ? 'success' : 'danger'">
            {{ running ? t('servicesCommon.running') : t('servicesCommon.notRunning') }}
          </el-tag>
          <el-tag v-if="version" size="small" type="info">{{ version }}</el-tag>
          <el-tag v-if="status.unit" size="small" type="warning">systemd</el-tag>
        </div>
        <div v-if="installed" class="actions">
          <span v-if="daemonPath" class="path mono">{{ daemonPath }}</span>
          <el-button size="small" :loading="busy" @click="refreshAll">{{
            t('servicesCommon.refresh')
          }}</el-button>
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
      <div v-if="installed" class="desc">
        {{ t('servicesDocker.daemonHint', { path: daemonPath || '/etc/docker/daemon.json' }) }}
      </div>
    </el-card>

    <!-- 未安装引导 -->
    <el-card v-if="installed === false && !bootLoading" shadow="never" class="mt-3">
      <el-result
        icon="warning"
        :title="t('servicesCommon.notInstalledTitle', { label: 'Docker' })"
        :sub-title="t('servicesCommon.notInstalledSub', { label: 'Docker', hint: installHint })"
      >
        <template #extra>
          <el-button type="primary" @click="router.push('/appstore')">{{
            t('servicesCommon.goAppstore')
          }}</el-button>
        </template>
      </el-result>
    </el-card>

    <template v-if="installed">
      <el-tabs v-model="mode" type="border-card" class="mt-3">
        <!-- 关键配置 -->
        <el-tab-pane :label="t('servicesDocker.tabKeys')" name="keys">
          <div class="keys-body">
            <el-alert
              type="info"
              :closable="false"
              show-icon
              class="keys-tip"
              :title="t('servicesDocker.keysTip')"
            />
            <div v-for="g in groups" :key="g.title" class="group">
              <div class="group-head">
                <span class="group-title">{{ g.title }}</span>
                <span class="group-desc">{{ g.desc }}</span>
              </div>
              <div class="field-grid">
                <div v-for="f in g.items" :key="f.key" class="field">
                  <div class="field-label">
                    <span>{{ f.label }}</span>
                    <span class="mono key">{{ f.key }}</span>
                    <el-tag v-if="f.kind === 'list'" size="small" type="info">{{
                      t('servicesDocker.kindList')
                    }}</el-tag>
                  </div>

                  <!-- 列表：一行一项，可增删（清空保存即从 daemon.json 删除该键） -->
                  <div v-if="f.kind === 'list'" class="list-field">
                    <div v-for="(_item, i) in lists[f.key]" :key="i" class="list-row">
                      <el-input
                        v-model="lists[f.key][i]"
                        :placeholder="itemPlaceholder(f.key)"
                        clearable
                        class="list-input"
                      />
                      <el-button text type="danger" :icon="Delete" @click="removeItem(f.key, i)" />
                    </div>
                    <div class="list-actions">
                      <el-button size="small" :icon="Plus" @click="addItem(f.key)">{{
                        t('servicesDocker.listAdd')
                      }}</el-button>
                      <el-select
                        v-if="f.key === 'registry_mirrors'"
                        v-model="quickMirror"
                        size="small"
                        class="quick-select"
                        :placeholder="t('servicesDocker.quickAdd')"
                        @change="addPresetMirror"
                      >
                        <el-option
                          v-for="m in PRESET_MIRRORS"
                          :key="m.url"
                          :label="m.name"
                          :value="m.url"
                        />
                      </el-select>
                    </div>
                    <div v-if="!lists[f.key]?.length" class="list-empty">
                      {{ t('servicesDocker.listEmpty') }}
                    </div>
                  </div>

                  <!-- 开关 / 枚举：清空即"未设置"，沿用 Docker 默认 -->
                  <el-select
                    v-else-if="f.kind === 'select' || f.kind === 'bool'"
                    v-model="scalar[f.key]"
                    :placeholder="t('servicesDocker.unset')"
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

                  <el-input
                    v-else
                    v-model="scalar[f.key]"
                    :placeholder="t('servicesDocker.unset')"
                    clearable
                    class="field-ctrl"
                  />
                  <div v-if="f.help" class="field-tip">{{ f.help }}</div>
                </div>
              </div>
            </div>
            <div class="save-row">
              <el-button :loading="saving" type="primary" @click="saveKeys">{{
                t('servicesDocker.saveKeys')
              }}</el-button>
              <el-button :disabled="saving" @click="loadKeys(true)">{{
                t('servicesCommon.reloadFromFile')
              }}</el-button>
            </div>
          </div>
        </el-tab-pane>

        <!-- daemon.json 文件编辑 -->
        <el-tab-pane :label="t('servicesDocker.tabFile')" name="file">
          <div v-if="confFiles.length" class="editor-layout">
            <div class="file-list">
              <div class="list-head">
                <span>{{ t('servicesDocker.editableConfs') }}</span>
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
                  </div>
                </div>
              </el-scrollbar>
            </div>
            <div class="editor-main">
              <div v-loading="fileLoading" class="editor-wrap">
                <div class="editor-bar">
                  <div class="bar-left">
                    <span class="mono path">{{ activeFile }}</span>
                    <el-tag v-if="fileMissing" size="small" type="danger">{{
                      t('servicesCommon.fileWillCreate')
                    }}</el-tag>
                  </div>
                  <div class="bar-right">
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
                  :path="activeFile"
                  lang="json"
                />
              </div>
              <div class="editor-tip">{{ t('servicesDocker.fileTip') }}</div>
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
import { Delete, Document, Plus } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import {
  controlDocker,
  getDockerKeys,
  getDockerStatus,
  listDockerConfs,
  readDockerConf,
  saveDockerConf,
  saveDockerKeys,
  type DockerConfFile,
  type DockerField,
  type DockerStatus,
} from '@/api/serverDocker.ts'

const { t } = useI18n()
const router = useRouter()

const installHint = computed(() => t('servicesDocker.installHint'))

/** 常用镜像加速器：点一下填进列表，剩下的（如阿里云 ID）用户自己替换 */
const PRESET_MIRRORS = [
  { name: '腾讯云(内网)', url: 'https://mirror.ccs.tencentyun.com' },
  { name: '阿里云（替换 <ID>）', url: 'https://<ID>.mirror.aliyuncs.com' },
  { name: 'DaoCloud', url: 'https://docker.m.daocloud.io' },
  { name: '轩辕镜像（免费版）', url: 'https://docker.xuanyuan.me' },
  { name: '毫秒镜像', url: 'https://docker.1ms.run' },
]

/**
 * 分组顺序 = 页面顺序；key 与后端 DOCKER_FIELDS 对齐，
 * 后端没给的 key（如旧版 zapexec）会被自动跳过，不会渲染出空控件。
 */
const GROUP_DEFS: { titleKey: string; descKey: string; keys: string[] }[] = [
  {
    titleKey: 'servicesDocker.groupMirrors',
    descKey: 'servicesDocker.groupMirrorsDesc',
    keys: ['registry_mirrors', 'insecure_registries'],
  },
  {
    titleKey: 'servicesDocker.groupLogging',
    descKey: 'servicesDocker.groupLoggingDesc',
    keys: ['log_driver', 'log_max_size', 'log_max_file'],
  },
  {
    titleKey: 'servicesDocker.groupStorage',
    descKey: 'servicesDocker.groupStorageDesc',
    keys: ['storage_driver', 'data_root'],
  },
  {
    titleKey: 'servicesDocker.groupRuntime',
    descKey: 'servicesDocker.groupRuntimeDesc',
    keys: ['dns', 'exec_opts', 'live_restore', 'userland_proxy', 'icc', 'debug'],
  },
]

const bootLoading = ref(false)
const busy = ref(false)
const acting = ref('')
const mode = ref('keys')
const status = ref<DockerStatus>({})
const fields = ref<DockerField[]>([])

const installed = computed(() => status.value.installed)
const running = computed(() => !!status.value.running)
const daemonPath = computed(() => status.value.conf_file || '')
/** `Docker version 27.3.1, build xxx` → `27.3.1` */
const version = computed(() => status.value.version?.match(/(\d+\.\d+(?:\.\d+)?)/)?.[1] || '')

const fieldMap = computed(() => new Map(fields.value.map((f) => [f.key, f])))
const groups = computed(() =>
  GROUP_DEFS.map((g) => ({
    title: t(g.titleKey),
    desc: t(g.descKey),
    items: g.keys.map((k) => fieldMap.value.get(k)).filter((f): f is DockerField => !!f),
  })).filter((g) => g.items.length),
)

// ── 关键配置表单 ──────────────────────────────
/** 标量字段（text / number / select / bool）：空串 = 未设置 */
const scalar = reactive<Record<string, string>>({})
/** 列表字段（registry-mirrors / insecure-registries / dns / exec-opts） */
const lists = reactive<Record<string, string[]>>({})
const quickMirror = ref('')
const saving = ref(false)

function itemPlaceholder(key: string) {
  return key === 'registry_mirrors'
    ? 'https://<ID>.mirror.aliyuncs.com'
    : t('servicesDocker.itemPlaceholder')
}

function addItem(key: string) {
  if (!lists[key]) lists[key] = []
  lists[key].push('')
}

function removeItem(key: string, i: number) {
  lists[key]?.splice(i, 1)
}

function addPresetMirror(url: string) {
  if (!url) return
  const list = (lists['registry_mirrors'] ||= [])
  if (!list.includes(url)) list.push(url)
  quickMirror.value = ''
}

function applyValues(values: Record<string, string | number | boolean | null>) {
  for (const k of Object.keys(scalar)) delete scalar[k]
  for (const k of Object.keys(lists)) delete lists[k]
  for (const f of fields.value) {
    const v = values[f.key]
    const text =
      v === null || v === undefined
        ? ''
        : typeof v === 'boolean'
          ? v
            ? 'true'
            : 'false'
          : String(v)
    if (f.kind === 'list') {
      lists[f.key] = text
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean)
    } else {
      scalar[f.key] = text
    }
  }
}

function collectPayload() {
  const payload: Record<string, string> = {}
  for (const f of fields.value) {
    payload[f.key] =
      f.kind === 'list'
        ? (lists[f.key] || [])
            .map((s) => s.trim())
            .filter(Boolean)
            .join('\n')
        : (scalar[f.key] || '').trim()
  }
  return payload
}

async function saveKeys() {
  // 后端也会校验，这里先拦一道是为了把错误定位到具体那一行
  const bad = (lists['registry_mirrors'] || []).find(
    (m) => m.trim() && !/^https?:\/\//.test(m.trim()),
  )
  if (bad) {
    ElMessage.error(t('servicesDocker.mirrorInvalid', { url: bad.trim() }))
    return
  }
  saving.value = true
  try {
    const res = await saveDockerKeys(collectPayload())
    ElMessage.success(res.data?.reason || t('servicesDocker.keysSaved'))
    await loadKeys(true)
    await loadStatus()
    await askRestart()
  } catch {
    /* interceptor 已提示 */
  } finally {
    saving.value = false
  }
}

/** daemon.json 只有重启 dockerd 才会重新加载，保存完顺手问一句 */
async function askRestart() {
  if (!running.value) return
  try {
    await ElMessageBox.confirm(t('servicesDocker.restartAsk'), t('common.tip'), {
      type: 'warning',
      confirmButtonText: t('servicesCommon.restart'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  await doControl('restart', true)
}

// ── 文件编辑 ────────────────────────────────
const confFiles = ref<DockerConfFile[]>([])
const activeFile = ref('')
const editorContent = ref<string | null>(null)
let originalContent = ''
const fileLoading = ref(false)
const savingFile = ref(false)
const fileMissing = ref(false)

const dirty = computed(
  () => editorContent.value !== null && editorContent.value !== originalContent,
)

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
    const res = await readDockerConf(path)
    editorContent.value = res.data.content
    originalContent = res.data.content
    fileMissing.value = !!res.data.missing
  } catch {
    editorContent.value = null
    originalContent = ''
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
    const res = await saveDockerConf(activeFile.value, editorContent.value)
    ElMessage.success(res.data?.reason || t('servicesCommon.confSaved'))
    originalContent = editorContent.value
    await loadList()
    await askRestart()
  } catch {
    /* interceptor 已提示 */
  } finally {
    savingFile.value = false
  }
}

// ── 状态与控制 ──────────────────────────────
async function loadStatus() {
  busy.value = true
  try {
    const res = await getDockerStatus()
    status.value = res.data
  } catch {
    /* interceptor 已提示 */
  } finally {
    busy.value = false
  }
}

async function loadList() {
  try {
    const res = await listDockerConfs()
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
    const res = await getDockerKeys()
    fields.value = res.data.fields
    applyValues(res.data.values)
  } catch {
    if (!silent) ElMessage.error(t('servicesCommon.readKeysFailed'))
  }
}

async function doControl(action: 'start' | 'restart', skipConfirm = false) {
  if (!skipConfirm) {
    try {
      await ElMessageBox.confirm(
        t('servicesCommon.controlConfirm', {
          label: 'Docker',
          action: t(`servicesCommon.${action === 'start' ? 'start' : 'restart'}`),
          warn: action === 'restart' ? t('servicesCommon.restartWarn') : '',
        }),
        t('common.tip'),
        { type: action === 'restart' ? 'warning' : 'info' },
      )
    } catch {
      return
    }
  }
  acting.value = action
  try {
    const res = await controlDocker(action)
    ElMessage.success(res.message || t('servicesCommon.opSuccess'))
    await new Promise((r) => setTimeout(r, 500))
    await loadStatus()
  } catch {
    /* interceptor 已提示 */
  } finally {
    acting.value = ''
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

// 未安装 → 安装（如用户在应用商店安装后返回）
watch(
  () => status.value.installed,
  (v) => {
    if (v) void refreshAll()
  },
)

onMounted(refreshAll)
</script>

<style scoped>
.docker-conf {
  min-height: 300px;
}
.mt-3 {
  margin-top: 12px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
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
.desc {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

/* 关键配置 */
.keys-body {
  padding: 8px 4px;
}
.keys-tip {
  margin-bottom: 16px;
}
.group + .group {
  margin-top: 18px;
  padding-top: 14px;
  border-top: 1px solid var(--el-border-color-lighter);
}
.group-head {
  margin-bottom: 12px;
}
.group-title {
  font-size: 14px;
  font-weight: 600;
}
.group-desc {
  margin-left: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.field-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
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
  font-size: 13px;
}
.field-label .key {
  font-size: 12px;
  color: var(--el-text-color-secondary);
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
/* 列表字段 */
.list-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
}
.list-input {
  flex: 1;
  min-width: 0;
}
.list-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
.quick-select {
  width: 190px;
}
.list-empty {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-placeholder);
}
.save-row {
  margin-top: 18px;
  display: flex;
  gap: 10px;
}

/* 文件编辑 */
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
.file-meta {
  margin-top: 4px;
  display: flex;
  gap: 6px;
}
.editor-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.editor-wrap {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.editor-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.bar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.bar-right {
  display: flex;
  gap: 8px;
}
.editor-tip {
  padding: 8px 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  border-top: 1px solid var(--el-border-color-lighter);
  line-height: 1.6;
}
</style>
