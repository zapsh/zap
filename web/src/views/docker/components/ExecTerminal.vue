<template>
  <el-dialog
    v-model="visible"
    :title="`${t('docker.exec.title')} · ${containerName}`"
    width="80%"
    top="6vh"
    :close-on-click-modal="false"
    @opened="onOpened"
    @closed="onClosed"
  >
    <div class="exec-wrap">
      <div class="exec-toolbar">
        <span class="exec-toolbar__label">{{ t('docker.exec.shell') }}</span>
        <el-select v-model="shell" size="small" :style="{ width: '120px' }" @change="reconnect">
          <el-option v-for="s in DOCKER_SHELLS" :key="s" :label="s" :value="s" />
        </el-select>
        <el-tag size="small" effect="dark" :type="statusType">{{ statusText }}</el-tag>
        <div class="exec-toolbar__spacer" />
        <el-button size="small" :icon="Refresh" :disabled="status === 'connecting'" @click="reconnect">
          {{ t('docker.exec.reconnect') }}
        </el-button>
      </div>

      <!-- xterm 挂载点：需要真实尺寸，因此等 dialog opened 之后再初始化 -->
      <div ref="termRef" class="exec-term" />
    </div>
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import '@xterm/xterm/css/xterm.css'
import { Refresh } from '@/icons'
import { DOCKER_SHELLS, type DockerShell } from '@/api/docker'
import { getToken } from '@/utils/auth'
import { wsUrl } from '@/utils/base'

const props = defineProps<{ modelValue: boolean; containerId: string; containerName: string }>()
const emit = defineEmits<{ 'update:modelValue': [boolean] }>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

type Status = 'idle' | 'connecting' | 'connected' | 'closed' | 'error'

const shell = ref<DockerShell>('sh')
const status = ref<Status>('idle')
const termRef = ref<HTMLDivElement>()

let term: Terminal | null = null
let fitAddon: FitAddon | null = null
let ws: WebSocket | null = null
let observer: ResizeObserver | null = null

const statusType = computed(() => {
  switch (status.value) {
    case 'connected':
      return 'success'
    case 'connecting':
      return 'warning'
    case 'error':
      return 'danger'
    default:
      return 'info'
  }
})

const statusText = computed(() => {
  switch (status.value) {
    case 'connecting':
      return t('docker.exec.connecting')
    case 'connected':
      return t('docker.exec.connected')
    case 'error':
      return t('docker.exec.openFailed')
    case 'closed':
      return t('docker.exec.closed')
    default:
      return t('docker.exec.closed')
  }
})

/** 带颜色的提示行（终端里没有 Element 的消息条，只能自己上色） */
function hint(text: string, color = 36): string {
  return `\x1b[${color}m${text}\x1b[0m`
}

function initTerminal() {
  if (term || !termRef.value) return

  term = new Terminal({
    cursorBlink: true,
    cursorStyle: 'bar',
    fontSize: 13,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    // 与 SSH 终端同一套深色主题，视觉上保持一致
    theme: {
      background: '#1e1e1e',
      foreground: '#d4d4d4',
      cursor: '#ffffff',
      selectionBackground: '#264f78',
    },
    allowProposedApi: true,
  })
  fitAddon = new FitAddon()
  term.loadAddon(fitAddon)
  term.loadAddon(new WebLinksAddon())
  term.open(termRef.value)

  // 输入 → WS（二进制帧：UTF-8 字节，保证中文/控制字符原样进入容器）
  term.onData((data) => send(new TextEncoder().encode(data)))
  // 尺寸变化 → 通知后端 resize_exec，vim / top 这类程序才能正常重绘
  term.onResize(({ cols, rows }) => sendControl({ type: 'resize', cols, rows }))

  // 容器尺寸变化（窗口缩放）时重新自适应列宽
  observer = new ResizeObserver(() => fit())
  observer.observe(termRef.value)

  nextTick(fit)
  term.focus()
}

function fit() {
  try {
    fitAddon?.fit()
  } catch {
    // 容器尚未布局完成时 fit 会抛错，忽略即可（尺寸稳定后会再触发）
  }
}

function send(data: Uint8Array) {
  if (ws?.readyState !== WebSocket.OPEN) return
  // 复制一份：`TextEncoder` 产出的是 `Uint8Array<ArrayBufferLike>`，
  // 而 `WebSocket.send` 只接受 backing buffer 为 `ArrayBuffer` 的视图
  ws.send(new Uint8Array(data))
}

function sendControl(payload: Record<string, unknown>) {
  if (ws?.readyState !== WebSocket.OPEN) return
  ws.send(JSON.stringify(payload))
}

function connect() {
  disconnect()
  if (!props.containerId) return

  initTerminal()
  status.value = 'connecting'
  term?.writeln(hint(t('docker.exec.connecting')))

  const cols = term?.cols ?? 120
  const rows = term?.rows ?? 30
  const url = wsUrl(
    `/api/docker/exec/ws?token=${getToken()}` +
      `&id=${encodeURIComponent(props.containerId)}` +
      `&shell=${shell.value}&cols=${cols}&rows=${rows}`,
  )

  const sock = new WebSocket(url)
  sock.binaryType = 'arraybuffer'
  ws = sock

  sock.onopen = () => {
    if (ws !== sock) return // 已被新会话替换
    fit()
  }

  sock.onmessage = (ev) => {
    if (ws !== sock) return
    // 二进制 = 终端输出；文本 = JSON 控制消息
    if (typeof ev.data === 'string') {
      let msg: { type?: string; code?: number; message?: string } = {}
      try {
        msg = JSON.parse(ev.data)
      } catch {
        return
      }
      if (msg.type === 'ready') {
        status.value = 'connected'
      } else if (msg.type === 'exit') {
        status.value = 'closed'
        term?.writeln(hint(t('docker.exec.exit', { code: msg.code ?? 0 })))
      } else if (msg.type === 'error') {
        status.value = 'error'
        term?.writeln(hint(t('docker.exec.error', { message: msg.message ?? '' }), 31))
      }
      return
    }
    term?.write(new Uint8Array(ev.data))
  }

  sock.onerror = () => {
    if (ws !== sock) return
    status.value = 'error'
    term?.writeln(hint(t('docker.exec.openFailed'), 31))
  }

  sock.onclose = () => {
    if (ws !== sock) return
    ws = null
    if (status.value === 'connecting') {
      status.value = 'error'
      term?.writeln(hint(t('docker.exec.openFailed'), 31))
    } else if (status.value !== 'error') {
      status.value = 'closed'
    }
  }
}

function disconnect() {
  const sock = ws
  ws = null
  if (!sock) return
  try {
    // 主动告知后端：关掉 stdin，容器内命令随之退出，不留孤儿进程
    if (sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify({ type: 'close' }))
  } catch {
    /* 已断开，忽略 */
  }
  sock.close()
}

function reconnect() {
  if (!props.containerId) return
  term?.clear()
  connect()
}

function onOpened() {
  nextTick(() => {
    initTerminal()
    connect()
  })
}

function onClosed() {
  disconnect()
  observer?.disconnect()
  observer = null
  term?.dispose()
  term = null
  fitAddon = null
  status.value = 'idle'
}

onBeforeUnmount(() => {
  disconnect()
  observer?.disconnect()
  term?.dispose()
})
</script>

<style scoped>
.exec-wrap {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.exec-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
}

.exec-toolbar__label {
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.exec-toolbar__spacer {
  flex: 1;
}

.exec-term {
  height: 60vh;
  padding: 6px;
  background: #1e1e1e;
  border-radius: 6px;
  overflow: hidden;
}
</style>
