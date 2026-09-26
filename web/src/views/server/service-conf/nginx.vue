<template>
  <div class="nginx-page">
    <!-- 四层转发（stream{}）是 nginx 的能力，入口收在这里，不再单独占一个菜单 -->
    <nav class="nginx-pills">
      <button
        type="button"
        class="nginx-pill"
        :class="{ 'is-active': page === 'conf' }"
        @click="page = 'conf'"
      >
        {{ t('servicesNginx.pageConf') }}
      </button>
      <button
        type="button"
        class="nginx-pill"
        :class="{ 'is-active': page === 'stream' }"
        @click="page = 'stream'"
      >
        {{ t('servicesNginx.pageStream') }}
      </button>
    </nav>

    <!-- v-show：切回来时编辑器里没保存的内容还在 -->
    <div v-show="page === 'conf'" v-loading="bootLoading" class="nginx-config">
    <!-- 顶部信息与操作 -->
    <el-card shadow="never" class="top-card">
      <div class="top-row">
        <div class="title">
          <el-icon :size="18"><Document /></el-icon>
          <span class="t-name">Nginx</span>
          <el-tag v-if="installed" size="small" :type="running ? 'success' : 'danger'">
            {{ running ? t('servicesNginx.running') : t('servicesNginx.notRunning') }}
          </el-tag>
          <el-tag v-if="installed" size="small" type="info">{{ versionText }}</el-tag>
          <el-tag v-if="installed && status.systemd" size="small" type="warning">systemd</el-tag>
        </div>
        <div class="actions" v-if="installed">
          <span class="path mono">{{ status.conf_file }}</span>
          <el-button size="small" :loading="busy" @click="refreshAll">{{
            t('servicesNginx.refreshStatus')
          }}</el-button>
          <el-button
            size="small"
            type="primary"
            plain
            :disabled="!running || busy"
            :loading="acting === 'reload'"
            @click="doControl('reload')"
            >{{ t('servicesNginx.reload') }}</el-button
          >
          <el-button
            size="small"
            type="warning"
            plain
            :disabled="!running || busy"
            :loading="acting === 'restart'"
            @click="doControl('restart')"
            >{{ t('servicesNginx.restart') }}</el-button
          >
          <el-button
            size="small"
            type="success"
            plain
            :disabled="running || busy"
            :loading="acting === 'start'"
            @click="doControl('start')"
            >{{ t('servicesNginx.start') }}</el-button
          >
        </div>
      </div>
    </el-card>

    <!-- 未安装引导 -->
    <el-card v-if="installed === false && !bootLoading" shadow="never" class="mt-3">
      <el-result
        icon="warning"
        :title="t('servicesNginx.notInstalledTitle')"
        :sub-title="t('servicesNginx.notInstalledSub')"
      >
        <template #extra>
          <el-button type="primary" @click="router.push('/appstore')">{{
            t('servicesNginx.goAppstore')
          }}</el-button>
        </template>
      </el-result>
    </el-card>

    <template v-if="installed">
      <!-- 默认站点（IP / 未匹配域名兜底） -->
      <el-card shadow="never" class="mt-3 default-vhost">
        <div class="dvh-row">
          <div class="dvh-left">
            <div class="dvh-title">
              {{ t('servicesNginx.defaultVhostTitle') }}
              <el-tag size="small" :type="ipAccess ? 'success' : 'info'">
                {{ ipAccess ? t('servicesNginx.ipOn') : t('servicesNginx.ipOff') }}
              </el-tag>
            </div>
            <div class="dvh-desc">
              {{ t('servicesNginx.defaultVhostDesc') }}
            </div>
            <div v-if="ipAccess && status.default_page" class="dvh-path mono">
              {{ t('servicesNginx.welcomePage', { path: status.default_page }) }}
            </div>
          </div>
          <div class="dvh-right">
            <el-switch
              v-model="ipAccess"
              :loading="savingDefaultVhost"
              inline-prompt
              :active-text="t('servicesNginx.on')"
              :inactive-text="t('servicesNginx.off')"
              @change="saveDefaultVhost"
            />
          </div>
        </div>
      </el-card>

      <el-tabs v-model="mode" type="border-card" class="mt-3">
        <!-- 可视化配置 -->
        <el-tab-pane :label="t('servicesNginx.tabVisual')" name="visual">
          <div class="visual-body">
            <el-alert
              type="info"
              :closable="false"
              show-icon
              class="visual-tip"
              :title="t('servicesNginx.visualTip')"
            />
            <div class="field-grid">
              <div v-for="f in allFields" :key="f.key" class="field">
                <div class="field-label">
                  <span class="mono">{{ f.key }}</span>
                  <el-tag v-if="f.key === TOP_LEVEL_KEY" size="small" type="warning">{{
                    t('servicesNginx.topLevel')
                  }}</el-tag>
                  <el-tag v-else size="small" type="info">http</el-tag>
                </div>
                <el-input
                  v-if="f.kind === 'input'"
                  v-model="visual[f.key]"
                  :placeholder="f.placeholder || t('servicesNginx.followDefaultPlaceholder')"
                  clearable
                  class="field-ctrl"
                />
                <el-select
                  v-else-if="f.kind === 'switch'"
                  v-model="visual[f.key]"
                  :placeholder="t('servicesNginx.followDefault')"
                  clearable
                  class="field-ctrl"
                >
                  <el-option :label="t('servicesNginx.optionOn')" value="on" />
                  <el-option :label="t('servicesNginx.optionOff')" value="off" />
                </el-select>
                <el-select
                  v-else
                  v-model="visual[f.key]"
                  :placeholder="t('servicesNginx.followDefault')"
                  clearable
                  class="field-ctrl"
                >
                  <el-option v-for="n in 9" :key="n" :label="`${n}`" :value="`${n}`" />
                </el-select>
                <div v-if="f.tip" class="field-tip">{{ f.tip }}</div>
              </div>
            </div>
            <div class="save-row">
              <el-button :loading="savingVisual" type="primary" @click="saveVisual">{{
                t('servicesNginx.saveVisual')
              }}</el-button>
              <el-button :disabled="savingVisual" @click="loadVisualFromFile">{{
                t('servicesNginx.reloadFromFile')
              }}</el-button>
            </div>
          </div>
        </el-tab-pane>

        <!-- 文件编辑 -->
        <el-tab-pane :label="t('servicesNginx.tabFiles')" name="files">
          <div class="editor-layout">
            <div class="file-list">
              <div class="list-head">
                <span>{{ t('servicesNginx.editableConfs') }}</span>
                <el-tag size="small" type="info">{{ confFiles.length }}</el-tag>
              </div>
              <el-scrollbar class="list-scroll">
                <div
                  v-for="f in confFiles"
                  :key="f.path"
                  class="file-item"
                  :class="{ active: activeFile?.path === f.path }"
                  @click="openFile(f)"
                >
                  <el-icon><Document /></el-icon>
                  <span class="file-name mono">{{
                    f.is_main ? t('servicesNginx.mainConfName') : f.rel
                  }}</span>
                </div>
              </el-scrollbar>
            </div>
            <div class="editor-main">
              <div class="editor-bar">
                <div class="bar-left">
                  <span class="mono path">{{
                    activeFile?.path || t('servicesNginx.selectFile')
                  }}</span>
                  <span v-if="activeFile" class="meta">{{ formatBytes(activeFile.size) }}</span>
                  <el-tag v-if="activeFile?.is_main" size="small" type="warning">{{
                    t('servicesNginx.mainConfTag')
                  }}</el-tag>
                </div>
                <div class="bar-right">
                  <el-button
                    size="small"
                    :disabled="!dirty || !activeFile"
                    :loading="savingFile"
                    type="primary"
                    @click="saveFile"
                    >{{ t('servicesNginx.save') }}</el-button
                  >
                  <el-button
                    size="small"
                    :disabled="!dirty || !activeFile"
                    @click="reloadActiveFile"
                    >{{ t('servicesNginx.discardChanges') }}</el-button
                  >
                </div>
              </div>
              <div class="editor-host">
                <CodeEditor v-model="fileContent" :path="activeFile?.path || ''" />
              </div>
              <div class="editor-tip">
                {{ t('servicesNginx.editorTip1') }}<code>nginx -t</code
                >{{ t('servicesNginx.editorTip2') }} <code>include …/sites-enabled/*.conf</code
                >{{ t('servicesNginx.editorTip3') }}
              </div>
            </div>
          </div>
        </el-tab-pane>

        <!-- WAF（ModSecurity）：可选能力，未安装时只给安装引导 -->
        <el-tab-pane :label="t('waf.tab')" name="waf">
          <div v-loading="wafLoading">
            <!-- 未安装：说明缺什么，能装才给按钮 -->
            <el-result
              v-if="waf && !waf.installed"
              icon="warning"
              :title="t('waf.notInstalledTitle')"
              :sub-title="waf.hint"
            >
              <template #extra>
                <div class="waf-blockers">
                  <div v-for="b in waf.blockers" :key="b" class="waf-blocker">· {{ b }}</div>
                </div>
                <el-button
                  type="primary"
                  :disabled="!waf.installable"
                  :loading="wafInstalling"
                  @click="doInstallWaf"
                >
                  {{ waf.installable ? t('waf.install') : t('waf.cannotInstall') }}
                </el-button>
              </template>
            </el-result>

            <template v-if="waf?.installed">
              <el-descriptions :column="2" border size="small" class="waf-desc">
                <el-descriptions-item :label="t('waf.engine')">
                  <el-tag
                    size="small"
                    :type="waf.engine === 'On' ? 'success' : 'warning'"
                  >
                    {{ waf.engine || '-' }}
                  </el-tag>
                  <span v-if="waf.engine !== 'On'" class="waf-inline-tip">
                    {{ t('waf.detectionOnlyTip') }}
                  </span>
                </el-descriptions-item>
                <el-descriptions-item :label="t('waf.crs')">
                  <el-tag size="small" :type="waf.crs ? 'success' : 'info'">
                    {{ waf.crs ? t('waf.deployed') : t('waf.missing') }}
                  </el-tag>
                </el-descriptions-item>
                <el-descriptions-item :label="t('waf.module')">
                  <span class="mono">{{ waf.module || '-' }}</span>
                </el-descriptions-item>
                <el-descriptions-item :label="t('waf.rulesDir')">
                  <span class="mono">{{ waf.rules_dir }}</span>
                </el-descriptions-item>
                <el-descriptions-item :label="t('waf.auditLog')">
                  <span class="mono">{{ waf.audit_log || '-' }}</span>
                  <el-button
                    size="small"
                    class="waf-inline-btn"
                    :disabled="!waf.audit_log"
                    @click="openAudit"
                  >
                    {{ t('waf.viewAudit') }}
                  </el-button>
                </el-descriptions-item>
              </el-descriptions>

              <el-alert
                type="warning"
                :closable="false"
                show-icon
                class="mt-3"
                :title="t('waf.saveTip')"
              />

              <el-table :data="waf.files" size="small" stripe class="mt-3">
                <el-table-column prop="rel" :label="t('waf.ruleFile')" min-width="240">
                  <template #default="{ row }">
                    <span class="mono">{{ row.rel }}</span>
                  </template>
                </el-table-column>
                <el-table-column :label="t('waf.colOp')" width="120" align="right">
                  <template #default="{ row }">
                    <el-button size="small" @click="openRule(row.path)">
                      {{ t('waf.edit') }}
                    </el-button>
                  </template>
                </el-table-column>
              </el-table>
            </template>
          </div>
        </el-tab-pane>
      </el-tabs>

    </template>

    <!-- WAF：规则编辑 / 审计日志 / 安装日志（未安装时不会走到这里） -->
    <el-dialog v-model="ruleVisible" :title="rulePath" width="70%" top="6vh" append-to-body>
      <div v-loading="ruleLoading" class="waf-rule-editor">
        <CodeEditor v-model="ruleContent" :path="rulePath" />
      </div>
      <div class="waf-rule-tip">{{ t('waf.ruleTip') }}</div>
      <template #footer>
        <el-button @click="ruleVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="ruleSaving" @click="saveRule">
          {{ t('waf.save') }}
        </el-button>
      </template>
    </el-dialog>

    <el-dialog
      v-model="auditVisible"
      :title="t('waf.auditTitle')"
      width="70%"
      top="6vh"
      append-to-body
    >
      <pre class="waf-audit mono">{{ auditText || t('waf.auditEmpty') }}</pre>
    </el-dialog>

    <AppStoreLogDrawer ref="logDrawer" />
    </div>

    <StreamConf v-if="page === 'stream'" />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Document } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import {
  controlNginx,
  getNginxStatus,
  listNginxConfs,
  readNginxConf,
  saveNginxConf,
  setNginxDefaultVhost,
  type NginxConfFile,
  type NginxStatus,
} from '@/api/serverNginx.ts'
import {
  applyVisualValues,
  parseVisualValues,
  TOP_LEVEL_KEY,
  type VisualValues,
} from '@/utils/nginxConf.ts'
import StreamConf from './stream.vue'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import { getTask } from '@/api/task.ts'
import {
  getWafAudit,
  getWafConfRead,
  getWafStatus,
  installWaf,
  saveWafConf,
  type WafStatus,
} from '@/api/waf.ts'

const { t } = useI18n()
const router = useRouter()
const bootLoading = ref(true)
const busy = ref(false)
const acting = ref('')
const installed = ref<boolean | null>(null)
const status = ref<NginxStatus>({})
/** conf = nginx 自身配置；stream = 四层转发规则（原独立菜单已收进来） */
const page = ref<'conf' | 'stream'>('conf')
const mode = ref('visual')

const running = computed(() => !!status.value.running)
const versionText = computed(() => {
  const m = (status.value.version || '').match(/nginx\/([\d.]+)/)
  return m ? m[1] : status.value.version || '-'
})

/* ---------- 默认站点（IP 访问） ---------- */
const savingDefaultVhost = ref(false)
const ipAccess = computed<boolean>({
  get: () => status.value.default_ip_access ?? false,
  set: (v: boolean) => {
    status.value.default_ip_access = v
  },
})

async function saveDefaultVhost(v: boolean | string | number) {
  const enable = !!v
  savingDefaultVhost.value = true
  try {
    const res = await setNginxDefaultVhost(enable)
    const stateText = t(enable ? 'servicesNginx.defaultEnabled' : 'servicesNginx.defaultDisabled')
    const tail = res.data?.reason
      ? t('servicesNginx.defaultReason', { reason: res.data.reason })
      : t('servicesNginx.defaultReloaded')
    ElMessage.success(stateText + tail)
    await refreshStatus()
  } catch {
    /* interceptor：设置失败时刷新状态回退开关 */
    await refreshStatus()
  } finally {
    savingDefaultVhost.value = false
  }
}

/* ---------- 可视化 ---------- */
interface FieldDef {
  key: string
  kind: 'input' | 'switch' | 'level'
  label: string
  placeholder?: string
  tip?: string
}

const allFields = computed<FieldDef[]>(() => [
  {
    key: TOP_LEVEL_KEY,
    kind: 'input',
    label: t('servicesNginx.fieldWorkers'),
    placeholder: t('servicesNginx.fieldWorkersPlaceholder'),
    tip: t('servicesNginx.fieldWorkersTip'),
  },
  { key: 'sendfile', kind: 'switch', label: 'sendfile', tip: t('servicesNginx.fieldSendfileTip') },
  {
    key: 'tcp_nopush',
    kind: 'switch',
    label: 'tcp_nopush',
    tip: t('servicesNginx.fieldTcpNopushTip'),
  },
  {
    key: 'keepalive_timeout',
    kind: 'input',
    label: 'keepalive_timeout',
    placeholder: t('servicesNginx.fieldKeepalivePlaceholder'),
  },
  {
    key: 'server_tokens',
    kind: 'switch',
    label: 'server_tokens',
    tip: t('servicesNginx.fieldServerTokensTip'),
  },
  {
    key: 'client_max_body_size',
    kind: 'input',
    label: 'client_max_body_size',
    placeholder: t('servicesNginx.fieldClientMaxBodyPlaceholder'),
    tip: t('servicesNginx.fieldClientMaxBodyTip'),
  },
  { key: 'gzip', kind: 'switch', label: 'gzip', tip: t('servicesNginx.fieldGzipTip') },
  {
    key: 'gzip_min_length',
    kind: 'input',
    label: 'gzip_min_length',
    placeholder: t('servicesNginx.fieldGzipMinLengthPlaceholder'),
  },
  {
    key: 'gzip_comp_level',
    kind: 'level',
    label: 'gzip_comp_level',
    tip: t('servicesNginx.fieldGzipCompLevelTip'),
  },
  {
    key: 'gzip_types',
    kind: 'input',
    label: 'gzip_types',
    placeholder: t('servicesNginx.fieldGzipTypesPlaceholder'),
    tip: t('servicesNginx.fieldGzipTypesTip'),
  },
])

const visual = reactive<VisualValues>({})
const mainPath = ref('')
const savingVisual = ref(false)

function initVisual() {
  for (const f of allFields.value) {
    visual[f.key] = null
  }
}

async function loadVisualFromFile() {
  if (!mainPath.value) return
  try {
    const res = await readNginxConf(mainPath.value)
    if (res.code !== 0) return
    const parsed = parseVisualValues(res.data.content)
    initVisual()
    for (const k of Object.keys(visual)) {
      if (parsed[k] !== undefined) visual[k] = parsed[k]
    }
  } catch {
    /* interceptor */
  }
}

async function saveVisual() {
  if (!mainPath.value) return
  savingVisual.value = true
  try {
    // 始终基于磁盘最新文本做最小行级补丁，避免覆盖面板 include 等托管行
    const latest = await readNginxConf(mainPath.value)
    if (latest.code !== 0) return
    const patched = applyVisualValues(latest.data.content, { ...visual })
    if (patched === null) {
      ElMessage.error(t('servicesNginx.noHttpBlock'))
      return
    }
    const res = await saveNginxConf(mainPath.value, patched)
    ElMessage.success(
      res.data?.reloaded
        ? t('servicesNginx.visualSavedReloaded')
        : t('servicesNginx.visualSavedReason', {
            reason: res.data?.reason || t('servicesNginx.passed'),
          }),
    )
  } catch {
    /* interceptor */
  } finally {
    savingVisual.value = false
  }
}

/* ---------- 文件编辑 ---------- */
const confFiles = ref<NginxConfFile[]>([])
const activeFile = ref<NginxConfFile | null>(null)
const fileContent = ref('')
let fileOriginal = ''
const savingFile = ref(false)

const dirty = computed(() => fileContent.value !== fileOriginal)

async function refreshConfList() {
  try {
    const res = await listNginxConfs()
    if (res.code !== 0) return
    const data = res.data
    confFiles.value = data.files
    if (!mainPath.value && data.conf_file) mainPath.value = data.conf_file
    if (!activeFile.value && confFiles.value.length) {
      await openFile(confFiles.value.find((f) => f.is_main) || confFiles.value[0])
    } else if (activeFile.value) {
      const cur = confFiles.value.find((f) => f.path === activeFile.value?.path)
      if (!cur) activeFile.value = null
    }
  } catch {
    /* interceptor */
  }
}

async function openFile(f: NginxConfFile) {
  if (activeFile.value && dirty.value) {
    try {
      await ElMessageBox.confirm(t('servicesNginx.unsavedSwitch'), t('common.tip'), {
        type: 'warning',
      })
    } catch {
      return
    }
  }
  activeFile.value = f
  try {
    const res = await readNginxConf(f.path)
    if (res.code !== 0) return
    fileContent.value = res.data.content
    fileOriginal = res.data.content
  } catch {
    /* interceptor */
  }
}

function reloadActiveFile() {
  if (activeFile.value) {
    fileContent.value = fileOriginal
    ElMessage.info(t('servicesNginx.discarded'))
  }
}

async function saveFile() {
  if (!activeFile.value || !dirty.value) return
  savingFile.value = true
  try {
    const res = await saveNginxConf(activeFile.value.path, fileContent.value)
    fileOriginal = fileContent.value
    ElMessage.success(
      res.data?.reloaded
        ? t('servicesNginx.fileSavedReloaded')
        : t('servicesNginx.fileSavedReason', {
            reason: res.data?.reason || t('servicesNginx.fileSavedPending'),
          }),
    )
  } catch {
    /* interceptor */
  } finally {
    savingFile.value = false
  }
}

/* ---------- 状态与控制 ---------- */
async function refreshStatus() {
  busy.value = true
  try {
    const res = await getNginxStatus()
    if (res.code === 0) {
      status.value = res.data
      installed.value = res.data.installed ?? false
    }
  } catch {
    /* interceptor */
  } finally {
    busy.value = false
  }
}

async function refreshAll() {
  bootLoading.value = true
  await refreshStatus()
  if (installed.value) {
    await refreshConfList()
    if (mainPath.value) await loadVisualFromFile()
  }
  bootLoading.value = false
}

const ctrlLabels = computed<Record<string, string>>(() => ({
  reload: t('servicesNginx.reload'),
  restart: t('servicesNginx.restart'),
  start: t('servicesNginx.start'),
  stop: t('servicesNginx.stop'),
}))

async function doControl(action: 'reload' | 'restart' | 'start' | 'stop') {
  const warn = action === 'stop' || action === 'restart'
  try {
    await ElMessageBox.confirm(
      t('servicesNginx.controlConfirm', { action: ctrlLabels.value[action] }),
      t('common.tip'),
      {
        type: warn ? 'warning' : 'info',
      },
    )
  } catch {
    return
  }
  acting.value = action
  try {
    const res = await controlNginx(action)
    ElMessage.success(
      res.message ?? t('servicesNginx.actionOk', { action: ctrlLabels.value[action] }),
    )
    await refreshStatus()
  } catch {
    /* interceptor */
  } finally {
    acting.value = ''
  }
}

/* ---------- WAF（ModSecurity）：可选能力，未安装即只显示安装引导 ---------- */
const waf = ref<WafStatus | null>(null)
const wafLoading = ref(false)
const wafInstalling = ref(false)
const logDrawer = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
const ruleVisible = ref(false)
const rulePath = ref('')
const ruleContent = ref('')
const ruleLoading = ref(false)
const ruleSaving = ref(false)
const auditVisible = ref(false)
const auditText = ref('')
/** 安装任务轮询（编译分钟级），完成后刷新状态 */
let wafTimer: number | undefined

async function loadWaf() {
  wafLoading.value = true
  try {
    const res = await getWafStatus()
    waf.value = res.data
  } catch {
    /* interceptor 已提示 */
  } finally {
    wafLoading.value = false
  }
}

/** 盯安装任务到终态：编译期间日志在抽屉里流式输出 */
function watchWafTask(runId: string) {
  logDrawer.value?.openDrawer(runId, t('waf.installTitle'))
  if (wafTimer) window.clearInterval(wafTimer)
  wafTimer = window.setInterval(async () => {
    try {
      const res = await getTask(runId)
      const st = res.data?.status || res.data?.task?.status
      if (st === 'success' || st === 'failed' || st === 'canceled') {
        window.clearInterval(wafTimer)
        wafTimer = undefined
        await loadWaf()
      }
    } catch {
      if (wafTimer) window.clearInterval(wafTimer)
      wafTimer = undefined
    }
  }, 3000)
}

async function doInstallWaf() {
  wafInstalling.value = true
  try {
    const res = await installWaf()
    if (res.data?.run_id) {
      watchWafTask(res.data.run_id)
    } else {
      await loadWaf()
    }
  } catch {
    /* interceptor 已提示 */
  } finally {
    wafInstalling.value = false
  }
}

async function openRule(path: string) {
  rulePath.value = path
  ruleVisible.value = true
  ruleLoading.value = true
  try {
    const res = await getWafConfRead(path)
    ruleContent.value = res.data.content
  } catch {
    ruleVisible.value = false
  } finally {
    ruleLoading.value = false
  }
}

async function saveRule() {
  ruleSaving.value = true
  try {
    const res = await saveWafConf(rulePath.value, ruleContent.value)
    ElMessage.success(res.data?.reload || res.message || t('servicesCommon.opSuccess'))
    ruleVisible.value = false
  } catch {
    /* 规则不合法时后端已回滚并给出 nginx -t 的报错 */
  } finally {
    ruleSaving.value = false
  }
}

async function openAudit() {
  auditVisible.value = true
  auditText.value = ''
  try {
    const res = await getWafAudit(200)
    auditText.value = res.data.content
  } catch {
    /* interceptor 已提示 */
  }
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${n} B`
}

initVisual()
onMounted(async () => {
  await refreshAll()
  // WAF 与 nginx 之间是"可选"关系：nginx 装了也未必有 WAF，状态各自独立
  loadWaf()
})
onUnmounted(() => {
  if (wafTimer) window.clearInterval(wafTimer)
})
</script>

<style scoped>
.nginx-pills {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px;
  margin-bottom: 12px;
  background: var(--el-fill-color-light);
  border-radius: 10px;
}
.nginx-pill {
  padding: 6px 14px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--el-text-color-regular);
  font-size: 13px;
  cursor: pointer;
  transition: background 0.2s, color 0.2s;
}
.nginx-pill:hover {
  color: var(--el-color-primary);
}
.nginx-pill.is-active {
  background: var(--el-color-primary);
  color: #fff;
  font-weight: 500;
}
.nginx-config {
  min-height: 60vh;
}
.mt-3 {
  margin-top: 12px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.tip {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
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
}
.t-name {
  font-weight: 600;
  font-size: 15px;
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

/* 可视化 */
.visual-body {
  padding: 4px 2px;
}
.visual-tip {
  margin-bottom: 12px;
}
.field-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
  gap: 14px 28px;
}
.field-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--el-text-color-primary);
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

/* 文件编辑 */
.editor-layout {
  display: flex;
  gap: 12px;
  min-height: 560px;
}
.file-list {
  width: 280px;
  flex-shrink: 0;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  display: flex;
  flex-direction: column;
}
.list-head {
  padding: 8px 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 13px;
  font-weight: 600;
}
.list-scroll {
  flex: 1;
}
.file-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 10px;
  cursor: pointer;
  font-size: 13px;
  color: var(--el-text-color-regular);
  transition: background 0.15s;
}
.file-item:hover {
  background: var(--el-fill-color-light);
}
.file-item.active {
  background: var(--el-color-primary-light-9);
  color: var(--el-color-primary);
}
.file-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
}
.editor-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  min-width: 0;
}
.editor-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  flex-wrap: wrap;
}
.bar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.bar-left .path {
  max-width: 560px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--el-text-color-primary);
}
.meta {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.editor-host {
  flex: 1;
  min-height: 460px;
}
.waf-blockers {
  max-width: 620px;
  margin: 0 auto 12px;
  text-align: left;
  font-size: 12px;
  color: var(--el-text-color-regular);
  line-height: 1.9;
}
.waf-blocker {
  padding-left: 4px;
}
.waf-desc {
  margin-bottom: 4px;
}
.waf-inline-tip {
  margin-left: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.waf-inline-btn {
  margin-left: 8px;
}
.waf-rule-editor {
  height: 52vh;
}
.waf-rule-tip {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
}
.waf-audit {
  max-height: 60vh;
  overflow: auto;
  margin: 0;
  padding: 10px;
  font-size: 12px;
  line-height: 1.6;
  background: var(--el-fill-color-light);
  border-radius: 4px;
  white-space: pre-wrap;
  word-break: break-all;
}
.editor-tip {
  padding: 6px 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  border-top: 1px solid var(--el-border-color-lighter);
  line-height: 1.6;
}

/* 默认站点（IP 访问） */
.dvh-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}
.dvh-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 6px;
}
.dvh-desc {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.7;
  max-width: 720px;
}
.dvh-path {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-color-primary);
  word-break: break-all;
}
</style>
