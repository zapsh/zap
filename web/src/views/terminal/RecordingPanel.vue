<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="rec-panel">
    <div class="rec-bar">
      <el-button size="small" :disabled="loading" @click="load">
        {{ t('terminal.recRefresh') }}
      </el-button>
      <span class="rec-hint">{{ t('terminal.recHint') }}</span>
    </div>

    <div class="rec-body">
      <el-table :data="items" size="small" height="100%">
        <el-table-column :label="t('terminal.recTime')" width="170">
          <template #default="{ row }">
            {{ formatTime(row.started_at) }}
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.recConn')" min-width="180">
          <template #default="{ row }">
            <span>{{ row.conn_name || `#${row.conn_id}` }}</span>
            <span v-if="row.conn_host" class="rec-host">{{ row.conn_host }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.recDuration')" width="100">
          <template #default="{ row }">
            {{ formatDuration(row.duration_ms) }}
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.recSize')" width="100">
          <template #default="{ row }">
            {{ formatSize(row.size_bytes) }}
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.recAction')" width="160">
          <template #default="{ row }">
            <el-button link size="small" type="primary" @click="play(row)">
              {{ t('terminal.recPlay') }}
            </el-button>
            <el-button link size="small" type="danger" @click="doRemove(row)">
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <span>{{ loading ? t('terminal.recLoading') : t('terminal.recEmpty') }}</span>
        </template>
      </el-table>
    </div>

    <!-- 回放区：xterm 按录制时的节奏重放输出 -->
    <div v-if="showPlayer" class="rec-player">
      <div class="rec-player-head">
        <span>{{ t('terminal.recPlaying') }}：{{ currentName }}</span>
        <div class="rec-player-ctrl">
          <el-button size="small" @click="toggle">
            {{ playing ? t('terminal.recPause') : t('terminal.recResume') }}
          </el-button>
          <el-button size="small" @click="stop">{{ t('terminal.recStop') }}</el-button>
          <el-select v-model="speed" size="small" class="rec-speed" @change="onSpeedChange">
            <el-option v-for="s in speeds" :key="s" :label="`${s}x`" :value="s" />
          </el-select>
          <span class="rec-progress">{{ progressText }}</span>
          <el-button size="small" text @click="stop">{{ t('common.close') }}</el-button>
        </div>
      </div>
      <div ref="playerRef" class="rec-player-body" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { deleteRecording, getRecording, getRecordings, type SessionRecording } from '@/api/terminal'

const props = defineProps<{ connId?: number | null }>()

const { t } = useI18n()

const items = ref<SessionRecording[]>([])
const loading = ref(false)

// ── 回放 ────────────────────────────────────────────────────

interface CastEvent {
  /** 相对会话开始的秒数 */
  t: number
  type: 'o' | 'r' | string
  data: string
}

const showPlayer = ref(false)
const playing = ref(false)
const currentName = ref('')
const progressText = ref('')
const speeds = [0.5, 1, 2, 4, 8]
const speed = ref(1)

const playerRef = ref<HTMLElement | null>(null)
let term: Terminal | null = null
let events: CastEvent[] = []
let idx = 0
let total = 0
let timer: number | null = null

async function load() {
  loading.value = true
  try {
    const resp = await getRecordings(props.connId ?? undefined)
    items.value = resp.data || []
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.recLoadFailed'))
  } finally {
    loading.value = false
  }
}

async function play(row: SessionRecording) {
  stop()
  let content = ''
  try {
    const resp = await getRecording(row.id)
    content = resp.data?.content || ''
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.recPlayFailed'))
    return
  }
  const cast = parseCast(content)
  if (!cast.events.length) {
    ElMessage.warning(t('terminal.recPlayFailed'))
    return
  }
  currentName.value = `${row.conn_name || '#' + row.conn_id} ${formatTime(row.started_at)}`
  events = cast.events
  total = events[events.length - 1].t
  showPlayer.value = true
  // 等回放区挂载出来再建 xterm，否则容器还是 undefined
  await nextTick()
  initTerm(cast.cols, cast.rows)
  idx = 0
  playing.value = true
  step()
}

/** asciinema v2：首行是 header JSON，其余每行一个 [时间, 类型, 数据] */
function parseCast(text: string): { cols: number; rows: number; events: CastEvent[] } {
  const lines = text.split('\n').filter((l) => l.trim().length > 0)
  let cols = 80
  let rows = 24
  const out: CastEvent[] = []
  lines.forEach((line, i) => {
    if (i === 0 && line.trimStart().startsWith('{')) {
      try {
        const head = JSON.parse(line)
        cols = Number(head.width) || cols
        rows = Number(head.height) || rows
      } catch {
        /* 头行坏了就按默认尺寸回放 */
      }
      return
    }
    try {
      const arr = JSON.parse(line)
      if (Array.isArray(arr) && arr.length >= 3) {
        out.push({ t: Number(arr[0]) || 0, type: String(arr[1]), data: String(arr[2] ?? '') })
      }
    } catch {
      /* 跳过坏行，不中断整段回放 */
    }
  })
  return { cols, rows, events: out }
}

function initTerm(cols: number, rows: number) {
  term?.dispose()
  term = null
  const el = playerRef.value
  if (!el) return
  term = new Terminal({
    cols,
    rows,
    fontSize: 13,
    fontFamily: 'Menlo, Consolas, "Courier New", monospace',
    disableStdin: true,
    convertEol: false,
    scrollback: 10000,
    theme: { background: '#1e1e1e', foreground: '#d4d4d4' },
  })
  term.open(el)
}

function step() {
  if (timer) {
    window.clearTimeout(timer)
    timer = null
  }
  if (!playing.value || idx >= events.length) {
    if (idx >= events.length) playing.value = false
    return
  }
  const ev = events[idx]
  const prev = idx === 0 ? 0 : events[idx - 1].t
  // 长时间无输出的空档按 2s 顶格，否则一次回放要干等好几分钟
  const delay = Math.min(Math.max(0, (ev.t - prev) * 1000) / speed.value, 2000)
  timer = window.setTimeout(() => {
    timer = null
    if (!term) return
    if (ev.type === 'o') {
      term.write(ev.data)
    } else if (ev.type === 'r') {
      const [c, r] = ev.data.split('x').map((v) => Number(v))
      if (c > 0 && r > 0) term.resize(c, r)
    }
    idx += 1
    progressText.value = `${fmtClock(ev.t)} / ${fmtClock(total)}`
    step()
  }, delay)
}

function toggle() {
  if (idx >= events.length) return
  playing.value = !playing.value
  if (playing.value) step()
}

/** 改速度后按新节奏继续（已经播过的不回退） */
function onSpeedChange() {
  if (playing.value) step()
}

function stop() {
  if (timer) {
    window.clearTimeout(timer)
    timer = null
  }
  playing.value = false
  showPlayer.value = false
  term?.dispose()
  term = null
  events = []
  idx = 0
  total = 0
  progressText.value = ''
}

async function doRemove(row: SessionRecording) {
  try {
    await ElMessageBox.confirm(t('terminal.recDeleteConfirm'), t('common.tip'), {
      type: 'warning',
      confirmButtonText: t('common.delete'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  try {
    await deleteRecording(row.id)
    ElMessage.success(t('terminal.recDeleteDone'))
    if (showPlayer.value) stop()
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.recDeleteFailed'))
  }
}

// ── 格式化 ──────────────────────────────────────────────────

function formatTime(ts: number) {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

function fmtClock(sec: number) {
  const s = Math.max(0, Math.floor(sec))
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(Math.floor(s / 60))}:${p(s % 60)}`
}

function formatDuration(ms: number) {
  if (!ms) return '-'
  const sec = Math.round(ms / 1000)
  if (sec < 60) return `${sec}s`
  if (sec < 3600) return `${Math.floor(sec / 60)}m${fmtClock(sec % 60).replace('00:', '')}s`
  return `${Math.floor(sec / 3600)}h${Math.floor((sec % 3600) / 60)}m`
}

function formatSize(n: number) {
  if (!n) return '-'
  const units = ['B', 'KB', 'MB', 'GB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i += 1
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`
}

watch(
  () => props.connId,
  () => {
    stop()
    load()
  },
)

onMounted(load)
onBeforeUnmount(stop)
</script>

<style scoped>
.rec-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 8px;
}

.rec-bar {
  display: flex;
  align-items: center;
  gap: 10px;
}

.rec-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.rec-body {
  flex: 1;
  min-height: 200px;
  overflow: hidden;
}

.rec-host {
  margin-left: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.rec-player {
  border: 1px solid var(--el-border-color);
  border-radius: 4px;
  overflow: hidden;
  background: #1e1e1e;
}

.rec-player-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 6px 8px;
  background: var(--el-fill-color-light);
  font-size: 12px;
  color: var(--el-text-color-regular);
}

.rec-player-ctrl {
  display: flex;
  align-items: center;
  gap: 6px;
}

.rec-speed {
  width: 80px;
}

.rec-progress {
  font-family: ui-monospace, Menlo, Consolas, monospace;
}
</style>
