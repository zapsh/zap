<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="log-viewer">
    <el-row :gutter="12" class="log-viewer-row">
      <!-- 左：日志源清单 -->
      <el-col :xs="24" :sm="24" :md="6" :lg="5" :xl="5">
        <el-card shadow="never" class="src-card">
          <template #header>
            <div class="card-head">
              <span>{{ t('serverLogs.sources') }}</span>
              <div class="head-actions">
                <el-button text :icon="Plus" :title="t('serverLogs.addCustom')" @click="openAdd" />
                <el-button text :icon="Refresh" :loading="loadingSources" @click="reload" />
              </div>
            </div>
          </template>

          <el-input
            v-model="filter"
            :prefix-icon="Search"
            :placeholder="t('serverLogs.filterSource')"
            clearable
            size="small"
          />

          <el-scrollbar class="src-scroll">
            <div v-for="g in groups" :key="g.name" class="src-group">
              <div class="src-group-title">{{ g.name }}</div>
              <div
                v-for="s in g.items"
                :key="s.id"
                class="src-item"
                :class="{ active: current?.id === s.id }"
                :title="s.desc"
                @click="select(s)"
              >
                <div class="src-line">
                  <span class="src-name">{{ s.name }}</span>
                  <el-button
                    v-if="s.custom"
                    link
                    type="danger"
                    size="small"
                    class="src-del"
                    :icon="Delete"
                    :title="t('serverLogs.removeCustom')"
                    @click.stop="removeCustom(s)"
                  />
                </div>
                <div class="src-desc">{{ s.desc }}</div>
              </div>
            </div>
            <el-empty
              v-if="!groups.length"
              :description="t('serverLogs.noSource')"
              :image-size="56"
            />
          </el-scrollbar>

          <div v-if="!journalOk" class="src-tip">{{ t('serverLogs.noJournal') }}</div>
        </el-card>
      </el-col>

      <!-- 右：日志内容 -->
      <el-col :xs="24" :sm="24" :md="18" :lg="19" :xl="19">
        <el-card shadow="never" class="log-card" v-loading="loading">
          <template #header>
            <div class="log-toolbar">
              <div class="log-title">
                <span class="name">{{ current?.name || t('serverLogs.pickSource') }}</span>
                <span class="desc">{{ current?.desc || '' }}</span>
              </div>

              <div class="log-actions">
                <el-select v-model="lineCount" size="small" style="width: 118px" @change="load(true)">
                  <el-option
                    v-for="n in [200, 500, 1000, 2000]"
                    :key="n"
                    :label="`${n} ${t('serverLogs.lines')}`"
                    :value="n"
                  />
                </el-select>

                <el-input
                  v-model="keyword"
                  :prefix-icon="Search"
                  :placeholder="t('serverLogs.keyword')"
                  clearable
                  size="small"
                  style="width: 170px"
                  @keyup.enter="onKeyword"
                  @clear="onKeyword"
                />

                <!-- 关 = 全部照常显示、命中处高亮；开 = 只留匹配行（服务端过滤） -->
                <el-tooltip :content="t('serverLogs.filterModeTip')" placement="top">
                  <el-checkbox v-model="filterMode" size="small">
                    {{ t('serverLogs.filterMode') }}
                  </el-checkbox>
                </el-tooltip>

                <el-checkbox v-model="follow" size="small">
                  {{ t('serverLogs.follow') }}
                </el-checkbox>
                <el-checkbox v-model="autoRefresh" size="small">
                  {{ t('serverLogs.autoRefresh') }}
                </el-checkbox>
                <el-select
                  v-if="autoRefresh"
                  v-model="intervalSec"
                  size="small"
                  style="width: 88px"
                >
                  <el-option :label="`3s`" :value="3" />
                  <el-option :label="`5s`" :value="5" />
                  <el-option :label="`10s`" :value="10" />
                  <el-option :label="`30s`" :value="30" />
                </el-select>

                <el-button size="small" :icon="Refresh" @click="load(true)">
                  {{ t('common.refresh') }}
                </el-button>
                <el-button
                  size="small"
                  :icon="Download"
                  :disabled="!buf.length"
                  @click="download"
                >
                  {{ t('serverLogs.download') }}
                </el-button>
                <el-button size="small" :icon="Delete" :disabled="!buf.length" @click="buf = []">
                  {{ t('serverLogs.clearView') }}
                </el-button>
              </div>
            </div>
          </template>

          <div ref="boxRef" class="log-box">
            <!-- 高亮走 v-html：内容已逐字符转义，日志里的 < > & 不会被当标签解析 -->
            <pre class="log-content" v-html="renderedHtml"></pre>
            <div v-if="!buf.length && !loading" class="log-empty">
              {{ current ? t('serverLogs.empty') : t('serverLogs.pickSource') }}
            </div>
          </div>

          <div class="log-foot">
            <span>{{ t('serverLogs.lineCount', { n: buf.length }) }}</span>
            <span v-if="keyword.trim() && !filterMode" class="hit">
              {{ t('serverLogs.hitCount', { n: hitCount }) }}
            </span>
            <span v-if="current?.kind === 'file' && current.size">
              {{ formatBytes(current.size) }}
            </span>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 手动添加自定义日志路径 -->
    <el-dialog
      v-model="addVisible"
      :title="t('serverLogs.addCustom')"
      width="520px"
      append-to-body
    >
      <el-form label-width="92px">
        <el-form-item :label="t('serverLogs.customPath')" required>
          <el-input
            v-model="addForm.path"
            placeholder="/var/log/myapp/app.log"
            @keyup.enter="submitAdd"
          />
          <div class="form-tip">{{ t('serverLogs.customPathTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('serverLogs.customName')">
          <el-input
            v-model="addForm.name"
            :placeholder="t('serverLogs.customNameTip')"
            @keyup.enter="submitAdd"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="addVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="adding" @click="submitAdd">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Delete, Download, Plus, Refresh, Search } from '@/icons'
import {
  addCustomLogSource,
  getLogSources,
  readSystemLog,
  removeCustomLogSource,
  type LogSource,
} from '@/api/logs'

const { t } = useI18n()
const route = useRoute()

const sources = ref<LogSource[]>([])
const journalOk = ref(false)
const loadingSources = ref(false)
const current = ref<LogSource | null>(null)
const filter = ref('')
const keyword = ref('')
/** true = 只显示匹配行（交给服务端过滤）；false = 全量显示、命中处高亮 */
const filterMode = ref(true)
const lineCount = ref(200)
const autoRefresh = ref(false)
const intervalSec = ref(5)
const follow = ref(true)
const loading = ref(false)
const buf = ref<string[]>([])
const cursor = ref('')
const boxRef = ref<HTMLElement | null>(null)

const addVisible = ref(false)
const adding = ref(false)
const addForm = ref({ name: '', path: '' })
let timer: number | undefined

/** 前端保留的最大行数：长时间跟随刷新时不让内存无限增长 */
const MAX_KEEP = 5000

const groups = computed(() => {
  const kw = filter.value.trim().toLowerCase()
  const list = kw
    ? sources.value.filter(
        (s) =>
          s.name.toLowerCase().includes(kw) ||
          s.desc.toLowerCase().includes(kw) ||
          s.group.toLowerCase().includes(kw),
      )
    : sources.value
  const map = new Map<string, LogSource[]>()
  for (const s of list) {
    const arr = map.get(s.group) ?? []
    arr.push(s)
    map.set(s.group, arr)
  }
  return [...map.entries()].map(([name, items]) => ({ name, items }))
})

/** 高亮模式下的命中行数（过滤模式下就是总行数，不必单列） */
const hitCount = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return 0
  return buf.value.filter((l) => l.toLowerCase().includes(kw)).length
})

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

/**
 * 渲染成 HTML：无关键字 / 过滤模式时只做转义；高亮模式下把命中片段包进 `<mark>`。
 *
 * 必须自己转义 —— 日志内容完全不可信（可能含 `<script>` 之类），直接拼 innerHTML
 * 就是一个存储型 XSS。
 */
const renderedHtml = computed(() => {
  const kw = keyword.value.trim()
  if (!kw || filterMode.value) {
    return escapeHtml(buf.value.join('\n'))
  }
  const lower = kw.toLowerCase()
  const escKw = escapeHtml(kw)
  return buf.value
    .map((line) => {
      const l = line.toLowerCase()
      if (!l.includes(lower)) return escapeHtml(line)
      let out = ''
      let i = 0
      for (;;) {
        const idx = l.indexOf(lower, i)
        if (idx < 0) {
          out += escapeHtml(line.slice(i))
          break
        }
        out += escapeHtml(line.slice(i, idx)) + '<mark>' + escKw + '</mark>'
        i = idx + kw.length
      }
      return out
    })
    .join('\n')
})

/**
 * 从 URL 里挑初始日志源，供其它页面跳进来时直达（如 Nginx 运行状态的「查看日志」）。
 *
 * - `?source=<id>`：直接用完整 id（允许是尚未被探测到的路径）
 * - `?pick=nginx-error`：自动挑 nginx 错误日志，挑不到退回 Web 组第一条
 */
function pickFromQuery(list: LogSource[]): LogSource | undefined {
  const src = route.query.source as string | undefined
  if (src) {
    const found = list.find((s) => s.id === src)
    if (found) return found
    const path = src.replace(/^file:/, '')
    return { id: src, name: path, group: '自定义', kind: 'file', desc: path, size: 0 }
  }
  if (route.query.pick === 'nginx-error') {
    return (
      list.find((s) => /nginx/i.test(s.desc) && /error/i.test(s.desc)) ??
      list.find((s) => s.group === 'Web') ??
      list[0]
    )
  }
  return undefined
}

async function loadSources(prefer?: (list: LogSource[]) => LogSource | undefined) {
  loadingSources.value = true
  try {
    const resp = await getLogSources()
    sources.value = resp.data?.sources ?? []
    journalOk.value = !!resp.data?.journal
    const target = prefer?.(sources.value) ?? sources.value[0]
    if (target) select(target)
    else current.value = null
  } catch (e: any) {
    ElMessage.error(e?.message || t('serverLogs.loadSourcesFailed'))
  } finally {
    loadingSources.value = false
  }
}

function reload() {
  const keepId = current.value?.id
  void loadSources(keepId ? (list) => list.find((s) => s.id === keepId) : pickFromQuery)
}

function select(s: LogSource) {
  stopTimer()
  current.value = s
  buf.value = []
  cursor.value = ''
  void load(true)
}

/** 关键字回车 / 清空：过滤模式下要重新拉（服务端过滤），高亮模式纯前端无需请求 */
function onKeyword() {
  if (filterMode.value) void load(true)
}

/**
 * 读取当前日志源。
 *
 * `reset = true` 表示重新取末尾 N 行（换源 / 改行数 / 改关键字 / 手动刷新）；
 * 否则带上 cursor 只取新增内容追加到末尾（自动刷新）。
 */
async function load(reset = false) {
  if (!current.value) return
  loading.value = true
  try {
    const resp = await readSystemLog({
      source: current.value.id,
      cursor: reset ? '' : cursor.value,
      lines: lineCount.value,
      // 只有"仅显示匹配行"才让服务端过滤；高亮模式要全量内容
      keyword: filterMode.value ? keyword.value.trim() : '',
    })
    const d = resp.data
    const incoming = d?.lines ?? []
    cursor.value = d?.cursor ?? ''
    // dmesg 之类不支持增量的源每次返回整段，直接替换而不是追加
    if (reset || !cursor.value) {
      buf.value = incoming
    } else if (incoming.length) {
      buf.value = [...buf.value, ...incoming].slice(-MAX_KEEP)
    }
    if (reset || follow.value) await scrollBottom()
  } catch (e: any) {
    ElMessage.error(e?.message || t('serverLogs.readFailed'))
    stopTimer()
  } finally {
    loading.value = false
  }
}

async function scrollBottom() {
  await nextTick()
  const el = boxRef.value
  if (el) el.scrollTop = el.scrollHeight
}

function startTimer() {
  stopTimer()
  if (!autoRefresh.value) return
  timer = window.setInterval(() => void load(false), intervalSec.value * 1000)
}

function stopTimer() {
  if (timer !== undefined) {
    window.clearInterval(timer)
    timer = undefined
  }
}

function download() {
  if (!buf.value.length) return
  const name = (current.value?.name || 'log').replace(/[^\w.-]+/g, '_')
  const blob = new Blob([buf.value.join('\n')], { type: 'text/plain;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${name}.log`
  a.click()
  URL.revokeObjectURL(url)
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

// ── 自定义日志源 ────────────────────────────────────────────

function openAdd() {
  addForm.value = { name: '', path: '' }
  addVisible.value = true
}

async function submitAdd() {
  const path = addForm.value.path.trim()
  if (!path) {
    ElMessage.warning(t('serverLogs.customPathRequired'))
    return
  }
  adding.value = true
  try {
    await addCustomLogSource(addForm.value.name.trim(), path)
    ElMessage.success(t('serverLogs.addCustomOk'))
    addVisible.value = false
    const id = `file:${path}`
    await loadSources((list) => list.find((s) => s.id === id))
  } catch (e: any) {
    ElMessage.error(e?.message || t('serverLogs.addCustomFailed'))
  } finally {
    adding.value = false
  }
}

async function removeCustom(s: LogSource) {
  const path = s.id.replace(/^file:/, '')
  try {
    await ElMessageBox.confirm(
      t('serverLogs.removeConfirm', { path }),
      t('serverLogs.removeCustom'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await removeCustomLogSource(path)
    ElMessage.success(t('serverLogs.removeCustomOk'))
    if (current.value?.id === s.id) current.value = null
    await loadSources((list) =>
      current.value ? list.find((x) => x.id === current.value?.id) : undefined,
    )
  } catch (e: any) {
    ElMessage.error(e?.message || t('serverLogs.removeCustomFailed'))
  }
}

watch([autoRefresh, intervalSec], () => {
  if (autoRefresh.value) startTimer()
  else stopTimer()
})

// 过滤模式切换：开→交给服务端过滤，关→拉全量再前端高亮，两种都要重新取
watch(filterMode, () => void load(true))

onMounted(() => void loadSources(pickFromQuery))
onUnmounted(stopTimer)
</script>

<style scoped>
.log-viewer {
  padding: 12px;
}
.log-viewer-row {
  align-items: stretch;
}

/* ── 左侧日志源 ── */
.src-card {
  height: 100%;
}
.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}
.head-actions {
  display: flex;
  align-items: center;
}
.src-scroll {
  margin-top: 8px;
  max-height: calc(100vh - 260px);
}
.src-group {
  margin-bottom: 6px;
}
.src-group-title {
  padding: 6px 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.src-item {
  padding: 6px 8px;
  border-radius: 6px;
  cursor: pointer;
  overflow: hidden;
}
.src-item:hover {
  background: var(--el-fill-color-light);
}
.src-item.active {
  background: var(--el-color-primary-light-9);
}
.src-line {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
}
.src-name {
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.src-del {
  flex: none;
  padding: 0;
  height: 16px;
}
.src-desc {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.src-tip {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-color-warning);
}

/* ── 右侧日志 ── */
.log-card {
  height: 100%;
}
.log-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  align-items: center;
  justify-content: space-between;
}
.log-title {
  display: flex;
  flex-direction: column;
  min-width: 180px;
}
.log-title .name {
  font-weight: 600;
  font-size: 14px;
}
.log-title .desc {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
.log-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
}
.log-box {
  position: relative;
  height: calc(100vh - 300px);
  min-height: 320px;
  overflow: auto;
  background: var(--el-fill-color-darker);
  border-radius: 6px;
  padding: 10px 12px;
}
.log-content {
  margin: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre;
  color: var(--el-text-color-primary);
}
.log-content :deep(mark) {
  background: var(--el-color-warning-light-5);
  color: inherit;
  padding: 0 1px;
  border-radius: 2px;
}
.log-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.log-foot {
  display: flex;
  gap: 14px;
  padding-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.log-foot .hit {
  color: var(--el-color-warning);
}
.form-tip {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.4;
}
</style>
