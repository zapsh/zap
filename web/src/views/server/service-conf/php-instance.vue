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
    </el-tabs>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Document } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import { useServiceConf } from '@/composables/useServiceConf.ts'
import { setServiceConfDefault, type ServiceConfInstance } from '@/api/servicesConf.ts'

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

onMounted(loadAll)
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
</style>
