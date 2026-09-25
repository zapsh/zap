<template>
  <div v-loading="busy" class="mysql-conf">
    <el-card shadow="never" class="top-card">
      <div class="top-row">
        <div class="title">
          <span class="t-name">MySQL / MariaDB</span>
          <el-tag v-if="engineName" size="small" type="warning">{{ engineName }}</el-tag>
          <el-tag v-if="status.installed" size="small" :type="running ? 'success' : 'danger'">
            {{ running ? t('servicesCommon.running') : t('servicesCommon.notRunning') }}
          </el-tag>
          <el-tag v-if="version" size="small" type="info">{{ version }}</el-tag>
          <el-tag v-if="status.unit" size="small" type="warning">systemd</el-tag>
        </div>
        <div v-if="status.installed" class="actions">
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
      <div v-if="status.installed" class="desc">{{ t('servicesMysql.desc') }}</div>
      <div v-if="details" class="desc details">{{ details }}</div>
    </el-card>

    <!-- 未安装引导 -->
    <el-card v-if="status.installed === false && !busy" shadow="never" class="mt-3">
      <el-result
        icon="warning"
        :title="t('servicesMysql.notInstalledTitle')"
        :sub-title="t('servicesMysql.notInstalledSub')"
      >
        <template #extra>
          <el-button type="primary" @click="router.push('/appstore')">
            {{ t('servicesCommon.goAppstore') }}
          </el-button>
        </template>
      </el-result>
    </el-card>

    <template v-if="status.installed">
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
                <el-input
                  v-if="f.kind === 'list'"
                  v-model="visual[f.key]"
                  type="textarea"
                  :rows="3"
                  :placeholder="t('servicesCommon.emptyValue')"
                  class="field-ctrl"
                />
                <el-select
                  v-else-if="f.kind === 'select' || f.kind === 'bool'"
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
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { Document } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import { useServiceConf } from '@/composables/useServiceConf.ts'

const { t } = useI18n()
const router = useRouter()
const mode = ref('keys')

const {
  status,
  listData,
  keysData,
  confFiles,
  busy,
  acting,
  running,
  version,
  engineName,
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
} = useServiceConf('mysql', 'MySQL / MariaDB')

/** 安装信息明细：安装目录 / 主配置 / systemd 单元 + 保存生效提示 */
const details = computed(() => {
  const dir = status.value.dir
  if (!dir || !status.value.installed) return ''
  const parts = [t('servicesCommon.installDir', { path: dir })]
  if (status.value.conf_file) {
    parts.push(t('servicesCommon.confPath', { path: status.value.conf_file }))
  }
  if (status.value.unit) parts.push(t('servicesCommon.unitPath', { path: status.value.unit }))
  return parts.join(t('servicesCommon.separator')) + t('servicesCommon.detailsSuffix')
})

/** 未检测到主配置时的排查提示：把后端尝试过的候选路径展示出来 */
const detectHint = computed(() => {
  const tried = status.value.conf_candidates?.length
    ? status.value.conf_candidates.join(t('servicesCommon.nameSeparator'))
    : ''
  const base = tried ? t('servicesCommon.detectTried', { tried }) : ''
  return base + t('servicesCommon.detectReason')
})

onMounted(loadAll)
</script>

<style scoped>
.mysql-conf {
  min-height: 300px;
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
.desc {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}
.details {
  color: var(--el-text-color-regular);
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
.detect-warn {
  margin-bottom: 12px;
}
.editor-layout {
  display: flex;
  gap: 12px;
  min-height: 460px;
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
