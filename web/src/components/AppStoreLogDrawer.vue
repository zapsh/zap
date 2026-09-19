<template>
  <el-drawer v-model="visible" :title="drawerTitle" size="62%" :destroy-on-close="false">
    <div class="log-wrap">
      <div class="log-toolbar">
        <el-tag :type="statusTagType" size="small" effect="dark">
          {{ statusText }}
        </el-tag>
        <span v-if="exitCode !== null" class="exit-code">{{
          t('runLogDrawer.exitCode', { code: exitCode })
        }}</span>
        <el-button
          v-if="!simple"
          size="small"
          type="danger"
          plain
          :disabled="done || !runId"
          :loading="stopping"
          @click="handleStop"
        >
          {{ t('runLogDrawer.stop') }}
        </el-button>
        <div class="toolbar-spacer" />
        <template v-if="isAdmin && !simple && failedState && probed && snapshotReady">
          <el-button size="small" type="warning" plain :loading="probeLoading" @click="openEditor">
            {{ t('runLogDrawer.editScript') }}
          </el-button>
          <el-button size="small" type="danger" plain :loading="retrying" @click="handleRetry">
            {{ t('runLogDrawer.retry') }}
          </el-button>
        </template>
        <el-button size="small" plain :disabled="!done" @click="handleScrollBottom">
          {{ t('runLogDrawer.scrollBottom') }}
        </el-button>
      </div>
      <div v-if="failedState && probed && !snapshotReady && !simple" class="snap-hint">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ t('runLogDrawer.noSnapshotHint') }}</span>
      </div>
      <div ref="termRef" class="term-box"></div>
    </div>

    <!-- 编辑运行快照脚本 -->
    <el-dialog
      v-model="editorVisible"
      :title="t('runLogDrawer.editDialogTitle')"
      width="780px"
      append-to-body
      :close-on-click-modal="false"
    >
      <div class="snap-editor">
        <div class="snap-files">
          <div class="snap-files-head">
            <span class="snap-files-title">{{ t('runLogDrawer.snapshotFiles') }}</span>
            <el-button text size="small" :loading="fileLoading" @click="refreshFiles">{{
              t('runLogDrawer.refresh')
            }}</el-button>
          </div>
          <el-scrollbar class="snap-files-list">
            <div
              v-for="f in files"
              :key="f.path"
              class="snap-file"
              :class="{ active: f.path === currentPath }"
              :title="t('runLogDrawer.fileTitle', { path: f.path, size: f.size })"
              @click="openFile(f.path)"
            >
              {{ f.path }}
            </div>
            <el-empty
              v-if="!files.length"
              :description="t('runLogDrawer.noFiles')"
              :image-size="44"
            />
          </el-scrollbar>
        </div>
        <div class="snap-main">
          <div class="snap-main-head">
            <span class="snap-path">{{ currentPath || t('runLogDrawer.selectFile') }}</span>
            <el-button
              type="primary"
              size="small"
              :disabled="!dirty || !currentPath"
              :loading="saving"
              @click="handleSave"
            >
              {{ t('runLogDrawer.saveChanges') }}
            </el-button>
          </div>
          <div class="snap-code-wrap">
            <CodeEditor
              v-model="fileContent"
              :path="currentPath"
              :readonly="!currentPath || fileLoading"
              :placeholder="
                currentPath
                  ? t('runLogDrawer.editorPlaceholderEdit')
                  : t('runLogDrawer.editorPlaceholderSelect')
              "
            />
          </div>
        </div>
      </div>
      <div class="snap-editor-tip">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ t('runLogDrawer.editorTip') }}</span>
      </div>
    </el-dialog>
  </el-drawer>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick, onBeforeUnmount } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { InfoFilled } from '@/icons'
import {
  stopScript,
  getRunFiles,
  readRunFile,
  writeRunFile,
  retryRun,
  type RunFileItem,
} from '@/api/appstore'
import { useUserStore } from '@/stores/user'
import { taskWsUrl } from '@/api/task'
import CodeEditor from '@/components/CodeEditor.vue'

/**
 * `simple`：只做日志展示，隐藏「停止 / 编辑脚本 / 重跑」。
 *
 * 非脚本类运行（如镜像构建）复用本抽屉时没有可终止的任务句柄，
 * 也拿不到 appstore 的运行快照，留着这些按钮只会报错。
 */
const props = withDefaults(defineProps<{ simple?: boolean }>(), { simple: false })

/** `retried`：抽屉内重跑成功后带出新任务号，好让列表刷新 */
const emit = defineEmits<{ (e: 'retried', runId: string): void }>()

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

const visible = ref(false)
const runId = ref('')
const drawerTitle = ref(t('runLogDrawer.title'))

const statusText = ref(t('runLogDrawer.statusWaiting'))
const statusTagType = ref<'info' | 'success' | 'danger' | 'warning'>('info')
const exitCode = ref<number | null>(null)
const done = ref(false)
const stopping = ref(false)

/** 运行失败（done 且非 success）：用于展示“编辑脚本/重跑”入口 */
const failedState = ref(false)
/** 是否已探测过快照（避免失败后重复请求） */
const probed = ref(false)
/** 该 run 存在可编辑快照（runs/<run_id>/pkg） */
const snapshotReady = ref(false)
const probeLoading = ref(false)
const retrying = ref(false)

// 快照脚本编辑对话框
const editorVisible = ref(false)
const files = ref<RunFileItem[]>([])
const currentPath = ref('')
const fileContent = ref('')
const originalContent = ref('')
const saving = ref(false)
const fileLoading = ref(false)
const dirty = computed(() => fileContent.value !== originalContent.value)

const termRef = ref<HTMLElement | null>(null)
let term: Terminal | null = null
let fitAddon: FitAddon | null = null
let ws: WebSocket | null = null

function openDrawer(id: string, title?: string) {
  if (!id) {
    ElMessage.warning(t('runLogDrawer.noRunId'))
    return
  }
  const reopen = visible.value
  runId.value = id
  if (title) drawerTitle.value = title
  visible.value = true
  // 抽屉已处于打开状态时 watch(visible) 不会再触发，需手动重连到新的 run
  if (reopen) connect()
}

/**
 * 供任务列表行直接打开「编辑脚本」：先探测该运行有没有快照，有就打开编辑对话框。
 *
 * 抽屉本身也会顺带连上日志，便于对照报错改脚本；从列表走这条路，
 * 用户不必先打开日志、等它跑完才知道能不能编辑。
 */
async function openEditorFor(id: string, title?: string) {
  if (!id) {
    ElMessage.warning(t('runLogDrawer.noRunId'))
    return
  }
  runId.value = id
  if (title) drawerTitle.value = title
  visible.value = true
  probeLoading.value = true
  try {
    const resp = await getRunFiles(id)
    files.value = resp.data?.files || []
    probed.value = true
    snapshotReady.value = true
    if (!files.value.length) {
      ElMessage.warning(t('runLogDrawer.noSnapshotHint'))
      return
    }
    editorVisible.value = true
    await openFile(files.value[0].path)
  } catch (e: any) {
    probed.value = true
    snapshotReady.value = false
    ElMessage.error(e?.message || t('runLogDrawer.loadFilesFailed'))
  } finally {
    probeLoading.value = false
  }
}

defineExpose({ openDrawer, openEditorFor })

function initTerminal() {
  if (!termRef.value) return
  if (!term) {
    term = new Terminal({
      cursorBlink: false,
      fontSize: 13,
      fontFamily: 'Menlo, Monaco, "Courier New", monospace',
      theme: {
        background: '#1e1e1e',
        foreground: '#d4d4d4',
      },
      convertEol: true,
      disableStdin: true,
    })
    fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(termRef.value)
    // 纯日志终端没有输入通道，xterm 也不内置复制快捷键；
    // 在捕获阶段接管 keydown，让 Ctrl/Cmd+C 与 Ctrl+Shift+C 都能复制选区
    term.element?.addEventListener('keydown', handleTermCopy, true)
  }
  term.clear()
  fitAddon?.fit()
}

function resetState() {
  done.value = false
  exitCode.value = null
  stopping.value = false
  failedState.value = false
  probed.value = false
  snapshotReady.value = false
  statusText.value = t('runLogDrawer.statusConnecting')
  statusTagType.value = 'info'
}

function connect() {
  resetState()
  if (!runId.value) return
  nextTick(() => initTerminal())
  if (!term) return

  // 日志流走通用任务队列端点（/appstore/ws 与它同一实现，仅作兼容路径保留）
  ws = new WebSocket(taskWsUrl(runId.value))
  ws.onopen = () => {
    statusText.value = t('runLogDrawer.statusRunning')
  }
  ws.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data)
      if (msg.type === 'log') {
        term?.write(msg.data)
      } else if (msg.type === 'done') {
        done.value = true
        exitCode.value = msg.exit_code
        if (msg.status === 'success') {
          statusText.value = t('runLogDrawer.statusSuccess')
          statusTagType.value = 'success'
        } else {
          statusText.value = t('runLogDrawer.statusFailed')
          statusTagType.value = 'danger'
          failedState.value = true
          // 仅管理员需要编辑/重跑入口，且只探测一次
          if (isAdmin.value && !probed.value) {
            probeSnapshot()
          } else if (!isAdmin.value) {
            probed.value = true
            snapshotReady.value = false
          }
        }
      } else if (msg.type === 'error') {
        done.value = true
        statusText.value = msg.message || t('runLogDrawer.statusError')
        statusTagType.value = 'danger'
      }
    } catch {
      // 非 JSON 帧直接输出
      term?.write(event.data)
    }
  }
  ws.onerror = () => {
    done.value = true
    statusText.value = t('runLogDrawer.statusConnError')
    statusTagType.value = 'danger'
  }
  ws.onclose = () => {
    if (!done.value) {
      statusText.value = t('runLogDrawer.statusDisconnected')
      statusTagType.value = 'warning'
    }
  }
}

function closeWs() {
  if (ws) {
    ws.close()
    ws = null
  }
}

async function handleStop() {
  if (!runId.value) return
  stopping.value = true
  try {
    await stopScript({ run_id: runId.value })
    ElMessage.success(t('runLogDrawer.stopSent'))
    statusText.value = t('runLogDrawer.statusStopped')
    statusTagType.value = 'warning'
  } catch (e: any) {
    ElMessage.error(e.message || t('runLogDrawer.stopFailed'))
  } finally {
    stopping.value = false
  }
}

function handleScrollBottom() {
  term?.scrollToBottom()
}

// ── 复制选中：xterm 日志终端的 Ctrl/Cmd+C 与 Ctrl+Shift+C ─────────
// disableStdin 的日志视图里 Ctrl+C 不会被消费成中断；
// xterm 画布渲染的选区不在 DOM 中，浏览器默认复制拿不到内容，
// 因此这里把选中文本显式写入剪贴板（无选区时放行，不拦截按键）。
function handleTermCopy(e: KeyboardEvent) {
  if (!term) return
  if (!(e.ctrlKey || e.metaKey)) return
  if (e.key.toLowerCase() !== 'c') return
  if (!term.hasSelection()) return
  e.preventDefault()
  e.stopPropagation()
  const selected = term.getSelection()
  if (typeof navigator.clipboard?.writeText === 'function') {
    navigator.clipboard.writeText(selected).then(
      () => ElMessage.success(t('runLogDrawer.copied')),
      () => ElMessage.warning(t('runLogDrawer.copyFailed')),
    )
    return
  }
  // 非安全上下文（纯 http）兜底：同一用户手势内 execCommand 同步复制
  const ta = document.createElement('textarea')
  ta.value = selected
  ta.setAttribute('readonly', '')
  ta.style.position = 'fixed'
  ta.style.top = '-9999px'
  document.body.appendChild(ta)
  ta.select()
  let ok = false
  try {
    ok = document.execCommand('copy')
  } catch {
    ok = false
  }
  document.body.removeChild(ta)
  if (ok) ElMessage.success(t('runLogDrawer.copied'))
  else ElMessage.warning(t('runLogDrawer.copyFailed'))
}

// ── 运行快照（失败后编辑/重跑）────────────────────────────────

async function probeSnapshot() {
  if (probed.value) return
  probed.value = true
  probeLoading.value = true
  try {
    const resp = await getRunFiles(runId.value)
    files.value = resp.data?.files || []
    snapshotReady.value = true
  } catch {
    snapshotReady.value = false
  } finally {
    probeLoading.value = false
  }
}

async function refreshFiles() {
  fileLoading.value = true
  try {
    const resp = await getRunFiles(runId.value)
    files.value = resp.data?.files || []
    snapshotReady.value = files.value.length > 0
  } catch (e: any) {
    ElMessage.error(e.message || t('runLogDrawer.loadFilesFailed'))
  } finally {
    fileLoading.value = false
  }
}

function openEditor() {
  editorVisible.value = true
  if (!files.value.length) refreshFiles()
}

async function openFile(path: string) {
  if (dirty.value && path !== currentPath.value) {
    try {
      await ElMessageBox.confirm(t('runLogDrawer.unsavedDiscard'), t('runLogDrawer.notice'), {
        type: 'warning',
      })
    } catch {
      return
    }
  }
  fileLoading.value = true
  try {
    const resp = await readRunFile(runId.value, path)
    currentPath.value = path
    fileContent.value = resp.data?.content ?? ''
    originalContent.value = fileContent.value
  } catch (e: any) {
    ElMessage.error(e.message || t('runLogDrawer.readFileFailed'))
  } finally {
    fileLoading.value = false
  }
}

async function handleSave() {
  if (!currentPath.value) return
  saving.value = true
  try {
    await writeRunFile({
      run_id: runId.value,
      path: currentPath.value,
      content: fileContent.value,
    })
    originalContent.value = fileContent.value
    ElMessage.success(t('runLogDrawer.savedRetry'))
  } catch (e: any) {
    ElMessage.error(e.message || t('runLogDrawer.saveFailed'))
  } finally {
    saving.value = false
  }
}

async function handleRetry() {
  try {
    await ElMessageBox.confirm(t('runLogDrawer.retryConfirm'), t('runLogDrawer.retryTitle'), {
      type: 'warning',
    })
  } catch {
    return
  }
  retrying.value = true
  try {
    const resp = await retryRun(runId.value)
    // 重跑安装/升级要等编译槽位：此时还没有新日志，留在原日志上提示即可
    if (resp.data?.queued) {
      ElMessage.success(t('task.retryQueued', { n: resp.data.position ?? 1 }))
      emit('retried', resp.data?.run_id || '')
      return
    }
    ElMessage.success(t('runLogDrawer.retryStarted'))
    // 切换到新运行日志
    closeWs()
    runId.value = resp.data?.run_id
    editorVisible.value = false
    currentPath.value = ''
    fileContent.value = ''
    originalContent.value = ''
    connect()
    emit('retried', runId.value)
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('runLogDrawer.retryFailed'))
  } finally {
    retrying.value = false
  }
}

watch(visible, (v) => {
  if (v) {
    nextTick(() => {
      initTerminal()
      connect()
    })
  } else {
    closeWs()
    editorVisible.value = false
  }
})

onBeforeUnmount(() => {
  closeWs()
  term?.element?.removeEventListener('keydown', handleTermCopy, true)
  term?.dispose()
  term = null
})
</script>

<style scoped>
.log-wrap {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.log-toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-bottom: 10px;
  margin-bottom: 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.toolbar-spacer {
  flex: 1;
}

.exit-code {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.snap-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  padding-bottom: 8px;
}

.term-box {
  flex: 1;
  background: #1e1e1e;
  border-radius: 4px;
  overflow: hidden;
  min-height: 300px;
}

/* 快照脚本编辑器 */
.snap-editor {
  display: flex;
  gap: 12px;
  height: 56vh;
}

.snap-files {
  width: 220px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  overflow: hidden;
}

.snap-files-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  background: var(--el-bg-color-page);
}

.snap-files-title {
  font-size: 13px;
  font-weight: 600;
}

.snap-files-list {
  flex: 1;
}

.snap-file {
  padding: 6px 12px;
  font-size: 12px;
  font-family: Menlo, Monaco, 'Courier New', monospace;
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.snap-file:hover {
  background: var(--el-color-primary-light-9);
}

.snap-file.active {
  background: var(--el-color-primary-light-8);
  color: var(--el-color-primary);
}

.snap-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.snap-main-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 0 8px;
}

.snap-path {
  font-size: 13px;
  font-family: Menlo, Monaco, 'Courier New', monospace;
  color: var(--el-text-color-regular);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.snap-code-wrap {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  overflow: hidden;
}

.snap-editor-tip {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
