<template>
  <div>
    <!-- 全局默认访问开关 -->
    <el-card shadow="never" class="default-card">
      <div class="default-row">
        <div class="default-main">
          <div class="default-title">
            <el-switch
              :model-value="inst.is_default"
              :loading="toggling"
              :disabled="toggling"
              @change="toggleDefault"
            />
            <span class="t-name">{{ t('servicesPhpPanel.defaultAccess') }}</span>
            <el-tag v-if="inst.is_default" type="success" size="small">
              {{ t('servicesPhpPanel.systemDefault') }}
            </el-tag>
            <el-tag v-else size="small" type="info">
              {{ t('servicesPhpPanel.notRegistered') }}
            </el-tag>
          </div>
          <div class="default-desc">
            {{ t('servicesPhpPanel.defaultDesc1') }}<code>php / php-cgi / pear / pecl</code>
            >{{ t('servicesPhpPanel.defaultDesc2') }} <code>/usr/local/bin</code>
            >{{ t('servicesPhpPanel.defaultDesc3') }}<code>php</code>
            >{{ t('servicesPhpPanel.defaultDesc4', { version: inst.version || inst.svc }) }}
          </div>
        </div>
      </div>
    </el-card>

    <!-- 实例配置：状态 / 关键项 / 配置文件 / 启停（svc = 实例名，如 php74） -->
    <el-card shadow="never" class="top-card">
      <div class="top-row">
        <div class="title">
          <span class="t-name">PHP {{ inst.version || inst.svc }}</span>
          <el-tag v-if="status.installed" size="small" :type="running ? 'success' : 'danger'">
            {{ running ? t('servicesCommon.running') : t('servicesCommon.notRunning') }}
          </el-tag>
          <el-tag v-if="inst.unit" size="small" type="warning">systemd</el-tag>
        </div>
        <div v-if="status.installed" class="actions">
          <span v-if="status.conf_file" class="path mono">{{ status.conf_file }}</span>
          <el-button size="small" :loading="busy" @click="loadStatus">
            {{ t('servicesCommon.refresh') }}
          </el-button>
          <el-button
            size="small"
            type="primary"
            plain
            :disabled="!running || !!acting"
            :loading="acting === 'reload'"
            @click="doControl('reload')"
          >
            {{ t('servicesCommon.reload') }}
          </el-button>
          <el-button
            size="small"
            type="warning"
            plain
            :disabled="!running || !!acting"
            :loading="acting === 'restart'"
            @click="doControl('restart')"
          >
            {{ t('servicesCommon.restart') }}
          </el-button>
          <el-button
            v-if="running"
            size="small"
            type="danger"
            plain
            :disabled="!!acting"
            :loading="acting === 'stop'"
            @click="doControl('stop')"
          >
            {{ t('servicesCommon.stop') }}
          </el-button>
          <el-button
            v-else
            size="small"
            type="success"
            plain
            :disabled="!!acting"
            :loading="acting === 'start'"
            @click="doControl('start')"
          >
            {{ t('servicesCommon.start') }}
          </el-button>
        </div>
      </div>
      <div class="desc">{{ descText }}</div>
    </el-card>

    <el-tabs v-model="mode" type="border-card" class="mt-3">
      <!-- 关键配置 -->
      <el-tab-pane :label="t('servicesCommon.tabKeys')" name="keys">
        <div v-if="keysData.fields.length" class="keys-body">
          <el-alert
            type="info"
            :closable="false"
            show-icon
            class="keys-tip"
            :title="t('servicesCommon.keysTip')"
          />
          <div class="field-grid">
            <div v-for="f in keysData.fields" :key="f.key" class="field">
              <div class="field-label">
                <span class="mono">{{ f.key }}</span>
                <el-tag v-if="f.section" size="small" type="info">[{{ f.section }}]</el-tag>
              </div>
              <el-select
                v-if="f.kind === 'select' || f.kind === 'bool'"
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
              <el-input
                v-else
                v-model="visual[f.key]"
                :placeholder="t('servicesCommon.emptyValue')"
                clearable
                class="field-ctrl"
              />
              <div v-if="f.help" class="field-tip">{{ f.help }}</div>
            </div>
          </div>
          <div class="save-row">
            <el-button :loading="savingKeys" type="primary" @click="saveKeys">
              {{ t('servicesCommon.saveKeys') }}
            </el-button>
            <el-button :disabled="savingKeys" @click="loadKeys">
              {{ t('servicesCommon.reloadFromFile') }}
            </el-button>
          </div>
        </div>
        <el-empty v-else :description="t('servicesCommon.noKeys')" />
      </el-tab-pane>

      <!-- 配置文件 -->
      <el-tab-pane :label="t('servicesCommon.tabFiles')" name="files">
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
                :class="{ active: activeFile === f.path }"
                @click="selectFile(f.path)"
              >
                <div class="file-name">
                  <el-icon :size="14"><Document /></el-icon>
                  <span class="mono">{{ f.rel }}</span>
                </div>
                <div class="file-meta">
                  <el-tag v-if="f.is_main" size="small" type="warning">
                    {{ t('servicesCommon.mainConfTag') }}
                  </el-tag>
                  <el-tag v-if="!f.exists" size="small" type="danger">
                    {{ t('servicesCommon.notExist') }}
                  </el-tag>
                </div>
              </div>
            </el-scrollbar>
          </div>
          <div class="editor-panel">
            <div v-loading="fileLoading" class="editor-wrap">
              <div v-if="editorContent !== null" class="editor-head">
                <span class="mono path">{{ activeFile }}</span>
                <el-tag v-if="fileMissing" size="small" type="danger">
                  {{ t('servicesCommon.fileWillCreate') }}
                </el-tag>
                <div class="editor-actions">
                  <el-button size="small" :disabled="!dirty" @click="reloadFile">
                    {{ t('servicesCommon.discardChanges') }}
                  </el-button>
                  <el-button
                    size="small"
                    type="primary"
                    :disabled="!dirty || savingFile"
                    :loading="savingFile"
                    @click="saveFile"
                  >
                    {{ t('servicesCommon.saveFile') }}
                  </el-button>
                </div>
              </div>
              <CodeEditor v-if="editorContent !== null" v-model="editorContent" :lang="editorLang" />
            </div>
          </div>
        </div>
        <el-empty v-else :description="t('servicesCommon.noConfFiles')" />
      </el-tab-pane>

      <!-- 扩展 -->
      <el-tab-pane :label="t('servicesPhpExt.tab')" name="ext">
        <el-alert
          v-if="extData.installed"
          type="info"
          :closable="false"
          show-icon
          class="ext-tip"
          :title="extSummary"
        />
        <el-alert
          v-if="extData.installed && extData.installer === 'none'"
          type="warning"
          :closable="false"
          show-icon
          class="ext-tip"
          :title="extData.installer_hint || ''"
        />
        <el-empty v-if="!extData.installed" :description="extData.reason || ''" />

        <template v-if="extData.installed">
          <div class="ext-toolbar">
            <el-input
              v-model="extKeyword"
              :placeholder="t('servicesPhpExt.searchPh')"
              clearable
              class="ext-search"
            />
            <el-button :loading="extLoading" @click="loadExts">
              {{ t('servicesCommon.refresh') }}
            </el-button>
            <el-button type="primary" @click="openInstall">
              {{ t('servicesPhpExt.install') }}
            </el-button>
          </div>

          <el-table v-loading="extLoading" :data="filteredExts" size="small" stripe>
            <el-table-column prop="name" :label="t('servicesPhpExt.colName')" min-width="140">
              <template #default="{ row }">
                <span class="mono">{{ row.name }}</span>
              </template>
            </el-table-column>
            <el-table-column
              prop="version"
              :label="t('servicesPhpExt.colVersion')"
              min-width="110"
            >
              <template #default="{ row }">
                <span>{{ row.version || '-' }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('servicesPhpExt.colState')" width="110">
              <template #default="{ row }">
                <el-tag v-if="row.builtin" size="small" type="info">
                  {{ t('servicesPhpExt.builtin') }}
                </el-tag>
                <el-tag
                  v-else
                  size="small"
                  :type="row.enabled ? 'success' : 'info'"
                >
                  {{ row.enabled ? t('servicesPhpExt.enabled') : t('servicesPhpExt.disabled') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('servicesPhpExt.colOp')" width="200" align="right">
              <template #default="{ row }">
                <el-button
                  v-if="!row.builtin"
                  size="small"
                  :type="row.enabled ? 'warning' : 'success'"
                  plain
                  :loading="extActing === row.name"
                  @click="toggleExt(row)"
                >
                  {{
                    row.enabled ? t('servicesPhpExt.disable') : t('servicesPhpExt.enable')
                  }}
                </el-button>
                <el-button
                  v-if="row.removable"
                  size="small"
                  type="danger"
                  plain
                  :loading="extRemoving === row.name"
                  @click="removeExt(row)"
                >
                  {{ t('servicesPhpExt.remove') }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </template>
      </el-tab-pane>
    </el-tabs>

    <!-- 安装扩展 -->
    <el-dialog
      v-model="installVisible"
      :title="t('servicesPhpExt.installTitle')"
      width="520px"
      append-to-body
    >
      <el-form label-width="90px">
        <el-form-item :label="t('servicesPhpExt.installPkg')">
          <el-input
            v-model="installForm.pkg"
            :placeholder="t('servicesPhpExt.installPkgPh')"
            class="mono"
          />
          <div class="ext-common">
            <el-tag
              v-for="p in COMMON_EXTS"
              :key="p"
              size="small"
              class="ext-common-tag"
              @click="installForm.pkg = p"
            >
              {{ p }}
            </el-tag>
          </div>
        </el-form-item>
        <el-form-item :label="t('servicesPhpExt.installVersion')">
          <el-input
            v-model="installForm.version"
            :placeholder="t('servicesPhpExt.installVersionPh')"
            class="mono"
          />
        </el-form-item>
      </el-form>
      <div class="ext-hint">
        {{ t('servicesPhpExt.installHint', { way: installerText }) }}
      </div>
      <template #footer>
        <el-button @click="installVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="installing" @click="doInstall">
          {{ t('servicesPhpExt.installSubmit') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 安装 / 卸载的编译日志（与应用商店同一套任务日志组件） -->
    <AppStoreLogDrawer ref="logDrawer" />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Document } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import { useServiceConf } from '@/composables/useServiceConf.ts'
import { getTask } from '@/api/task.ts'
import {
  getPhpExtList,
  installPhpExt,
  removePhpExt,
  setServiceConfDefault,
  togglePhpExt,
  type PhpExtItem,
  type PhpExtListData,
  type ServiceConfInstance,
} from '@/api/servicesConf.ts'

/** 常用扩展：PECL 上的裸名，点一下填进输入框（版本留空取最新稳定版） */
const COMMON_EXTS = [
  'redis',
  'imagick',
  'swoole',
  'zip',
  'bcmath',
  'gd',
  'intl',
  'memcached',
  'mongodb',
  'xdebug',
  'igbinary',
  'apcu',
]

const props = defineProps<{
  inst: ServiceConfInstance
}>()

const emit = defineEmits<{
  (e: 'changed'): void
}>()

const { t } = useI18n()
const mode = ref('keys')
const toggling = ref(false)

const instanceLabel = computed(() => `PHP ${props.inst.version || props.inst.svc}`)

const {
  status,
  keysData,
  confFiles,
  busy,
  acting,
  running,
  visual,
  savingKeys,
  loadKeys,
  saveKeys,
  activeFile,
  editorContent,
  fileLoading,
  savingFile,
  fileMissing,
  dirty,
  editorLang,
  selectFile,
  reloadFile,
  saveFile,
  loadStatus,
  loadAll,
  doControl,
} = useServiceConf(() => props.inst.svc, instanceLabel)

const descText = computed(() => {
  const parts: string[] = []
  if (props.inst.dir) parts.push(t('servicesCommon.installDir', { path: props.inst.dir }))
  if (props.inst.conf_file) {
    parts.push(t('servicesCommon.confPath', { path: props.inst.conf_file }))
  }
  if (props.inst.unit) parts.push(t('servicesCommon.unitPath', { path: props.inst.unit }))
  const prefix = parts.length
    ? parts.join(t('servicesPhpPanel.separator')) + t('servicesPhpPanel.partsSuffix')
    : ''
  return prefix + t('servicesPhpPanel.descSuffix')
})

// ── 扩展 ─────────────────────────────────────────────────────
const extData = ref<PhpExtListData>({ installed: false, extensions: [] })
const extLoading = ref(false)
const extKeyword = ref('')
const extActing = ref('')
const extRemoving = ref('')
const installVisible = ref(false)
const installing = ref(false)
const logDrawer = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
const installForm = ref({ pkg: '', version: '' })
/** 长任务轮询定时器（编译分钟级，完成后自动刷新清单；组件卸载时清掉） */
let taskTimer: number | undefined

const filteredExts = computed(() => {
  const k = extKeyword.value.trim().toLowerCase()
  const list = extData.value.extensions || []
  return k ? list.filter((e: PhpExtItem) => e.name.toLowerCase().includes(k)) : list
})

const installerText = computed(() => {
  const key = extData.value.installer || 'none'
  return t(`servicesPhpExt.way${key.charAt(0).toUpperCase()}${key.slice(1)}`)
})

const extSummary = computed(() => {
  const d = extData.value
  const parts: string[] = []
  if (d.version) parts.push(`PHP ${d.version}`)
  if (d.extension_dir) parts.push(t('servicesPhpExt.extDir', { path: d.extension_dir }))
  return parts.join(t('servicesCommon.separator')) + t('servicesPhpExt.summarySuffix', {
    way: installerText.value,
  })
})

async function loadExts() {
  extLoading.value = true
  try {
    const res = await getPhpExtList(props.inst.svc)
    extData.value = res.data
  } catch {
    /* interceptor 已提示 */
  } finally {
    extLoading.value = false
  }
}

/** 安装 / 卸载都是后台编译任务：打开日志抽屉，轮询到终态再刷新清单 */
function watchTask(runId: string, title: string) {
  logDrawer.value?.openDrawer(runId, title)
  if (taskTimer) window.clearInterval(taskTimer)
  taskTimer = window.setInterval(async () => {
    try {
      const res = await getTask(runId)
      const st = res.data?.status || res.data?.task?.status
      if (st === 'success' || st === 'failed' || st === 'canceled') {
        window.clearInterval(taskTimer)
        taskTimer = undefined
        await loadExts()
      }
    } catch {
      if (taskTimer) window.clearInterval(taskTimer)
      taskTimer = undefined
    }
  }, 3000)
}

async function toggleExt(row: PhpExtItem) {
  const tip = row.enabled
    ? t('servicesPhpExt.confirmDisable', { name: row.name })
    : t('servicesPhpExt.confirmEnable', { name: row.name })
  try {
    await ElMessageBox.confirm(tip, t('common.tip'), {
      type: row.enabled ? 'warning' : 'info',
    })
  } catch {
    return
  }
  extActing.value = row.name
  try {
    const res = await togglePhpExt(props.inst.svc, row.name, !row.enabled)
    ElMessage.success(res.data?.reload || res.message || t('servicesCommon.opSuccess'))
    await loadExts()
  } catch {
    /* interceptor 已提示 */
  } finally {
    extActing.value = ''
  }
}

function openInstall() {
  installForm.value = { pkg: '', version: '' }
  installVisible.value = true
}

async function doInstall() {
  const pkg = installForm.value.pkg.trim()
  if (!pkg) {
    ElMessage.warning(t('servicesPhpExt.needPkg'))
    return
  }
  installing.value = true
  try {
    const res = await installPhpExt(props.inst.svc, pkg, installForm.value.version.trim())
    installVisible.value = false
    const data = res.data
    if (!data?.run_id) {
      ElMessage.success(res.message || t('servicesCommon.opSuccess'))
      await loadExts()
      return
    }
    if (data.queued) {
      ElMessage.success(t('servicesPhpExt.queued', { n: data.position ?? 1 }))
    }
    watchTask(data.run_id, `${t('servicesPhpExt.install')} ${pkg}`)
  } catch {
    /* interceptor 已提示 */
  } finally {
    installing.value = false
  }
}

async function removeExt(row: PhpExtItem) {
  try {
    await ElMessageBox.confirm(
      t('servicesPhpExt.confirmRemove', { name: row.name }),
      t('common.tip'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  extRemoving.value = row.name
  try {
    const res = await removePhpExt(props.inst.svc, row.name)
    if (res.data?.run_id) {
      watchTask(res.data.run_id, `${t('servicesPhpExt.remove')} ${row.name}`)
    } else {
      await loadExts()
    }
  } catch {
    /* interceptor 已提示 */
  } finally {
    extRemoving.value = ''
  }
}

async function toggleDefault(next: boolean) {
  if (toggling.value) return
  if (next === props.inst.is_default) return
  const version = props.inst.version || props.inst.svc
  const tip = next
    ? t('servicesPhpPanel.confirmOn', { version })
    : t('servicesPhpPanel.confirmOff', { version })
  try {
    await ElMessageBox.confirm(tip, t('common.tip'), { type: next ? 'warning' : 'info' })
  } catch {
    return
  }
  toggling.value = true
  try {
    const res = await setServiceConfDefault(props.inst.svc, next)
    ElMessage.success(
      res.data?.registered
        ? t('servicesPhpPanel.registered', { names: res.data.registered.join(' / ') })
        : res.data?.removed
          ? t('servicesPhpPanel.unregistered', { names: res.data.removed.join(' / ') })
          : res.message || t('servicesCommon.opSuccess'),
    )
    emit('changed')
  } catch {
    /* interceptor 已提示 */
  } finally {
    toggling.value = false
  }
}

onMounted(async () => {
  await loadAll()
  loadExts()
})
onUnmounted(() => {
  if (taskTimer) window.clearInterval(taskTimer)
})
</script>

<style scoped>
.default-card {
  border-radius: 8px;
  margin-bottom: 12px;
}
.default-row {
  display: flex;
  align-items: flex-start;
}
.default-main {
  min-width: 0;
  flex: 1;
}
.default-title {
  display: flex;
  align-items: center;
  gap: 10px;
}
.t-name {
  font-weight: 600;
}
.default-desc {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}
.top-card {
  border-radius: 8px;
}
.top-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
}
.title {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.desc {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}
.mt-3 {
  margin-top: 12px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
}
.keys-body {
  padding: 4px 0;
}
.keys-tip {
  margin-bottom: 12px;
}
.field-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 14px;
}
.field-label {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
  font-size: 13px;
  font-weight: 600;
}
.field-ctrl {
  width: 100%;
}
.field-tip {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.save-row {
  display: flex;
  gap: 10px;
  margin-top: 16px;
}
.editor-layout {
  display: flex;
  gap: 12px;
  min-height: 420px;
}
.file-list {
  display: flex;
  flex-direction: column;
  width: 260px;
  flex: 0 0 260px;
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
}
.list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  border-bottom: 1px solid var(--el-border-color-light);
  font-size: 13px;
  font-weight: 600;
}
.list-scroll {
  flex: 1;
}
.file-item {
  padding: 8px 12px;
  cursor: pointer;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.file-item:hover {
  background: var(--el-fill-color-light);
}
.file-item.active {
  background: var(--el-color-primary-light-9);
}
.file-name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  word-break: break-all;
}
.file-meta {
  display: flex;
  gap: 6px;
  margin-top: 4px;
}
.editor-panel {
  flex: 1;
  min-width: 0;
}
.editor-wrap {
  display: flex;
  flex-direction: column;
  height: 100%;
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
}
.editor-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--el-border-color-light);
  font-size: 12px;
}
.editor-head .path {
  flex: 1;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
.editor-actions {
  display: flex;
  gap: 6px;
}
.ext-tip {
  margin-bottom: 12px;
}
.ext-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}
.ext-search {
  max-width: 260px;
}
.ext-common {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}
.ext-common-tag {
  cursor: pointer;
}
.ext-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}
</style>
