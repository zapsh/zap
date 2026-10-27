<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!--
  本地终端（编辑器内嵌）：
  - 后端 /terminal/local/ws 会用「当前登录用户对应的 Linux 账号」启动一个 PTY shell，
    前端只负责 xterm 渲染与按键转发，不参与任何权限提升。
  - 组件挂载即建立一条会话，卸载（关闭面板）即断开并回收 shell。
-->
<template>
  <div ref="host" class="local-term" />
</template>

<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import '@xterm/xterm/css/xterm.css'
import { useI18n } from 'vue-i18n'
import { getToken } from '@/utils/auth'
import { wsUrl } from '@/utils/base'
import { attachClipboardShortcuts } from '@/utils/termClipboard'

const props = withDefaults(
  defineProps<{
    /** 起始工作目录；后端会校验必须落在用户家目录内，非法则回退家目录 */
    cwd?: string
    /** 所在浮窗是否可见：从最小化/其它路由切回来时重新 fit 一次 */
    active?: boolean
  }>(),
  { cwd: '', active: true },
)

const { t } = useI18n()
const host = ref<HTMLElement | null>(null)

let term: Terminal | null = null
let fitAddon: FitAddon | null = null
let ws: WebSocket | null = null
let observer: ResizeObserver | null = null
let disposed = false
/** 服务端已给出明确原因（如套餐未开启）：关闭时不再补一句容易误解的「已断开」 */
let gotError = false

function buildUrl(cols: number, rows: number) {
  const token = encodeURIComponent(getToken())
  const cwd = encodeURIComponent(props.cwd || '')
  return wsUrl(`/api/terminal/local/ws?token=${token}&cwd=${cwd}&cols=${cols}&rows=${rows}`)
}

function fitNow() {
  if (!term || !fitAddon) return
  try {
    fitAddon.fit()
  } catch {
    // 容器尚未布局完成：忽略，等下一次 resize / rAF
  }
}

function handleTextFrame(raw: string) {
  if (!term) return
  if (raw.startsWith('{')) {
    try {
      const msg = JSON.parse(raw) as { type?: string; message?: string }
      if (msg.type === 'error') {
        gotError = true
        term.writeln('\r\n\x1b[31m' + (msg.message || 'error') + '\x1b[0m')
        return
      }
      if (msg.type) return
    } catch {
      // 不是 JSON，按普通终端输出处理
    }
  }
  term.write(raw)
}

function connect() {
  if (!term) return
  fitNow()
  const socket = new WebSocket(buildUrl(term.cols || 80, term.rows || 24))
  socket.binaryType = 'arraybuffer'
  ws = socket

  socket.onopen = () => {
    fitNow()
    term?.focus()
  }
  socket.onmessage = (ev: MessageEvent) => {
    if (typeof ev.data === 'string') handleTextFrame(ev.data)
    else if (ev.data instanceof ArrayBuffer) term?.write(new Uint8Array(ev.data))
  }
  socket.onclose = () => {
    if (disposed || !term || gotError) return
    term.writeln('\r\n\x1b[33m' + t('fileEditor.terminalClosed') + '\x1b[0m')
  }
}

function initTerm() {
  if (!host.value) return
  term = new Terminal({
    cursorBlink: true,
    cursorStyle: 'bar',
    fontSize: 13,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
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
  term.open(host.value)
  attachClipboardShortcuts(term)

  term.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(data)
  })
  term.onResize(({ cols, rows }) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'resize', cols, rows }))
    }
  })

  // 窗口拖拽 / 最大化 / 底部面板高度变化都要重新 fit
  observer = new ResizeObserver(() => requestAnimationFrame(fitNow))
  observer.observe(host.value)

  requestAnimationFrame(fitNow)
  if (document.fonts?.ready) document.fonts.ready.then(() => requestAnimationFrame(fitNow))
}

onMounted(async () => {
  await nextTick()
  initTerm()
  connect()
})

onBeforeUnmount(() => {
  disposed = true
  observer?.disconnect()
  observer = null
  try {
    ws?.close()
  } catch {
    // 已断开
  }
  ws = null
  term?.dispose()
  term = null
  fitAddon = null
})

watch(
  () => props.active,
  (visible) => {
    if (visible) requestAnimationFrame(fitNow)
  },
)
</script>

<style scoped>
.local-term {
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #1e1e1e;
}

.local-term :deep(.xterm) {
  height: 100%;
  padding: 4px 6px;
}

.local-term :deep(.xterm-viewport) {
  background-color: transparent !important;
}
</style>
