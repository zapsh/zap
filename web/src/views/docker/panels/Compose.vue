<template>
  <div class="docker-pane compose-pane">
    <el-alert
      v-if="env && !env.compose"
      type="info"
      :closable="false"
      class="compose-tip"
      :title="t('docker.env.composeOff')"
    />

    <div class="compose">
      <!-- 左：项目列表 -->
      <aside class="compose__side">
        <div class="compose__side-head">
          <el-button type="primary" size="small" :icon="Plus" @click="openCreate">
            Compose
          </el-button>
          <el-button size="small" :icon="Refresh" circle @click="reload" />
        </div>

        <el-input
          v-model="keyword"
          size="small"
          :prefix-icon="Search"
          clearable
          :placeholder="t('docker.common.search')"
        />

        <ul v-loading="loading" class="piles">
          <li
            v-for="p in filtered"
            :key="p.Name"
            class="pile"
            :class="{ 'is-active': p.Name === active }"
            @click="select(p.Name)"
          >
            <el-tag size="small" effect="dark" :type="statusType(p.Status)">
              {{ statusLabel(p.Status) }}
            </el-tag>
            <span class="pile__name">{{ p.Name }}</span>
          </li>
          <li v-if="!filtered.length" class="piles__empty">{{ t('docker.common.empty') }}</li>
        </ul>
      </aside>

      <!-- 右：项目详情 -->
      <section class="compose__detail">
        <template v-if="current">
          <header class="detail-head">
            <div class="detail-head__title">
              <h3>{{ current.Name }}</h3>
              <el-tag size="small" effect="dark" :type="statusType(current.Status)">
                {{ current.Status || '—' }}
              </el-tag>
            </div>
            <div class="detail-head__actions">
              <el-button
                size="small"
                type="primary"
                :icon="Play"
                :loading="busy === 'up'"
                @click="act('up')"
              >
                {{ t('docker.compose.up') }}
              </el-button>
              <el-button size="small" :loading="busy === 'stop'" @click="act('stop')">
                {{ t('docker.compose.stop') }}
              </el-button>
              <el-button
                size="small"
                :icon="Refresh"
                :loading="busy === 'restart'"
                @click="act('restart')"
              >
                {{ t('docker.compose.restart') }}
              </el-button>
              <el-button
                size="small"
                :icon="Download"
                :loading="busy === 'update'"
                @click="act('update')"
              >
                {{ t('docker.compose.update') }}
              </el-button>
              <el-button size="small" :loading="busy === 'pull'" @click="act('pull')">
                {{ t('docker.compose.pull') }}
              </el-button>
              <el-button
                size="small"
                :icon="Edit"
                :disabled="!current.Managed"
                @click="openEdit"
              >
                {{ t('docker.common.edit') }}
              </el-button>
              <el-button size="small" type="danger" :icon="Delete" @click="remove">
                {{ t('docker.compose.down') }}
              </el-button>
            </div>
          </header>

          <div class="detail-path mono">
            {{ current.ConfigFiles || '—' }}
            <el-tag v-if="!current.Managed" size="small" type="warning" effect="plain">
              {{ t('docker.compose.external') }}
            </el-tag>
          </div>

          <!-- 服务容器 -->
          <div class="card">
            <div class="card__head">
              <span class="card__title">{{ t('docker.compose.containers') }}</span>
              <span class="card__count">{{ projectContainers.length }}</span>
              <div class="card__spacer" />
              <el-button size="small" :icon="Refresh" @click="loadContainers">
                {{ t('docker.refresh') }}
              </el-button>
            </div>
            <div class="card__body">
              <div v-for="c in projectContainers" :key="c.ID" class="svc">
                <div class="svc__main">
                  <div class="svc__name">{{ shortName(c) }}</div>
                  <div class="svc__image mono">{{ c.Image }}</div>
                </div>
                <el-tag
                  size="small"
                  effect="dark"
                  :type="c.State === 'running' ? 'success' : 'info'"
                >
                  {{ c.State }}
                </el-tag>
                <el-button size="small" :disabled="c.State !== 'running'" @click="openShell(c)">
                  Shell
                </el-button>
              </div>
              <el-empty
                v-if="!projectContainers.length"
                :description="t('docker.compose.noContainers')"
                :image-size="50"
              />
            </div>
          </div>

          <!-- 日志 -->
          <div class="card">
            <div class="card__head">
              <span class="card__title">{{ t('docker.compose.logs') }}</span>
              <div class="card__spacer" />
              <el-select
                v-model="tail"
                size="small"
                :style="{ width: '120px' }"
                @change="loadLogs"
              >
                <el-option v-for="n in TAILS" :key="n" :value="n" :label="`tail ${n}`" />
              </el-select>
              <el-button size="small" :icon="Refresh" :loading="logLoading" @click="loadLogs">
                {{ t('docker.refresh') }}
              </el-button>
            </div>
            <pre ref="logRef" class="card__log mono">{{ log || t('docker.compose.noLogs') }}</pre>
          </div>

          <!-- 配置文件 -->
          <div class="card">
            <div class="card__head">
              <span class="card__title">compose.yaml</span>
              <div class="card__spacer" />
              <el-button size="small" :icon="Copy" @click="copyYaml">
                {{ t('docker.common.copy') }}
              </el-button>
            </div>
            <pre class="card__yaml mono">{{ yaml }}</pre>
          </div>
        </template>

        <el-empty v-else :description="t('docker.compose.selectHint')" />
      </section>
    </div>

    <!-- 新建 / 编辑（导入走同一个对话框：选文件填充内容） -->
    <ComposeFormDialog
      v-model="formVisible"
      :mode="formMode"
      :project="active"
      :content="editContent"
      :existing="rows.map((r) => r.Name)"
      @saved="onSaved"
    />
    <ExecTerminal v-model="execVisible" :container-id="execId" :container-name="execName" />
  </div>
</template>

<script setup lang="ts">
import { computed, inject, nextTick, onMounted, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Copy, Delete, Download, Edit, Play, Plus, Refresh, Search } from '@/icons'
import {
  composeAction,
  composeFile,
  composeLogs,
  composeRemove,
  listComposeProjects,
  listContainers,
  type DockerComposeProject,
  type DockerContainer,
  type DockerEnvStatus,
} from '@/api/docker'
import ComposeFormDialog from '../components/ComposeFormDialog.vue'
import ExecTerminal from '../components/ExecTerminal.vue'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()

/** 由父页面注入的 Docker 环境信息，用于提示 compose 插件是否可用 */
const env = inject<Ref<DockerEnvStatus | null>>('docker-env', ref(null))

/** 日志可选的尾部行数 */
const TAILS = [200, 1000, 3000]

const loading = ref(false)
const rows = ref<DockerComposeProject[]>([])
const keyword = ref('')
const active = ref('')

const containers = ref<DockerContainer[]>([])
const yaml = ref('')
const log = ref('')
const tail = ref(TAILS[0])
const logLoading = ref(false)

/** 正在执行的动作名（按钮各自转圈） */
const busy = ref('')

const formVisible = ref(false)
const formMode = ref<'create' | 'edit'>('create')
const editContent = ref('')

const execVisible = ref(false)
const execId = ref('')
const execName = ref('')

const logRef = ref<HTMLElement | null>(null)

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return rows.value
  return rows.value.filter((r) => `${r.Name} ${r.Status} ${r.ConfigFiles}`.toLowerCase().includes(kw))
})

const current = computed(() => rows.value.find((r) => r.Name === active.value) ?? null)

/** 该项目下的容器：容器列表已带 compose 项目名（`com.docker.compose.project` 标签） */
const projectContainers = computed(() =>
  containers.value.filter((c) => c.project && c.project === active.value),
)

function shortName(c: DockerContainer): string {
  return (c.Names || '').replace(/^\//, '') || c.ID.slice(0, 12)
}

// ── 列表 ─────────────────────────────────────────────────

async function load() {
  loading.value = true
  try {
    const resp = await listComposeProjects()
    rows.value = resp.data.items ?? []
    emit('count', rows.value.length)
    // 选中的项目可能已被删除 / 还没选过：退回第一个，保持右侧不空
    if (!rows.value.some((r) => r.Name === active.value)) {
      active.value = rows.value[0]?.Name ?? ''
    }
  } catch (e: any) {
    // compose 插件不可用时 `docker compose ls` 必然失败：静默成空列表，
    // 由上方的 alert 说明原因，避免每次开 tab 都弹错误
    rows.value = []
    if (env.value?.compose) ElMessage.error(e.message || t('docker.common.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function reload() {
  await load()
  if (active.value) {
    await loadContainers()
    await loadDetail()
  }
}

function select(name: string) {
  if (name === active.value) return
  active.value = name
}

/** 选中项目变化时拉取它的容器 / 日志 / 配置 */
watch(active, async (name) => {
  if (!name) {
    yaml.value = ''
    log.value = ''
    return
  }
  await Promise.all([loadContainers(), loadDetail()])
})

// ── 详情 ─────────────────────────────────────────────────

async function loadContainers() {
  try {
    const resp = await listContainers(true)
    containers.value = resp.data.items ?? []
  } catch {
    // 容器列表只影响详情里的服务卡片，失败时保持上一次内容
  }
}

async function loadDetail() {
  await Promise.all([loadYaml(), loadLogs()])
}

/** 打开某个服务容器的 exec 终端（复用容器面板那个抽屉） */
function openShell(c: DockerContainer) {
  execId.value = c.ID
  execName.value = shortName(c)
  execVisible.value = true
}

async function loadYaml() {
  if (!active.value) return
  try {
    const resp = await composeFile(active.value)
    yaml.value = resp.data.content
  } catch (e: any) {
    yaml.value = ''
    ElMessage.error(e.message || t('docker.common.loadFailed'))
  }
}

async function loadLogs() {
  if (!active.value) return
  logLoading.value = true
  try {
    const resp = await composeLogs(active.value, tail.value)
    log.value = resp.data.output ?? ''
    // 日志总是看最新的一行，拉完滚到底部
    await nextTick()
    logRef.value?.scrollTo({ top: logRef.value.scrollHeight })
  } catch (e: any) {
    // 项目还没起来 / 没有容器时 compose logs 必然报错：把原因写在日志区里就好，
    // 不当成"加载失败"弹提示（每次切项目都会弹一次，很吵）
    log.value = e.message || ''
  } finally {
    logLoading.value = false
  }
}

// ── 动作 ─────────────────────────────────────────────────

const DONE_KEYS: Record<string, string> = {
  up: 'docker.compose.upDone',
  stop: 'docker.compose.stopDone',
  restart: 'docker.compose.restartDone',
  pull: 'docker.compose.pulled',
  update: 'docker.compose.updated',
  start: 'docker.compose.upDone',
}

async function act(action: string) {
  const project = current.value
  if (!project) return
  busy.value = action
  try {
    await composeAction(project.Name, action)
    ElMessage.success(t(DONE_KEYS[action] ?? 'docker.common.actionFailed'))
    await load()
    await Promise.all([loadContainers(), loadDetail()])
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  } finally {
    busy.value = ''
  }
}

async function remove() {
  const project = current.value
  if (!project) return
  try {
    await ElMessageBox.confirm(
      t('docker.compose.downConfirm', { name: project.Name }),
      t('docker.compose.down'),
      { type: 'warning' },
    )
  } catch {
    return
  }

  busy.value = 'remove'
  try {
    await composeRemove(project.Name)
    ElMessage.success(t('docker.compose.downDone'))
    active.value = ''
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  } finally {
    busy.value = ''
  }
}

// ── 新建 / 编辑 ──────────────────────────────────────────

function openCreate() {
  formMode.value = 'create'
  editContent.value = ''
  formVisible.value = true
}

function openEdit() {
  if (!current.value) return
  formMode.value = 'edit'
  editContent.value = yaml.value
  formVisible.value = true
}

async function onSaved(project: string, start: boolean) {
  await load()
  active.value = project
  await loadContainers()
  await loadDetail()
  if (start) {
    await act('up')
  } else if (formMode.value === 'edit') {
    // 配置改了但容器还跑着旧参数：这里只说清楚，不擅自重建
    ElMessage.info(t('docker.compose.savedHint'))
  }
}

async function copyYaml() {
  try {
    await navigator.clipboard.writeText(yaml.value)
    ElMessage.success(t('docker.common.copySuccess'))
  } catch {
    ElMessage.error(t('docker.common.actionFailed'))
  }
}

// ── 状态展示 ─────────────────────────────────────────────

/** `running(2)` / `exited(1)` / `created`（受管目录里还没启动） */
function statusType(status: string): 'success' | 'warning' | 'danger' | 'info' {
  const s = (status || '').toLowerCase()
  if (s.startsWith('running')) return 'success'
  if (s.startsWith('created')) return 'info'
  if (s.includes('exited') || s.includes('dead')) return 'danger'
  return 'warning'
}

function statusLabel(status: string): string {
  const s = (status || '').toLowerCase()
  if (s.startsWith('created')) return t('docker.compose.notStarted')
  if (!s) return '—'
  return status
}

watch(() => props.refreshToken, reload)
onMounted(reload)
</script>

<style scoped>
.compose-tip {
  margin-bottom: 12px;
}

.compose {
  display: flex;
  gap: 12px;
  /* 与左侧列表 / 右侧详情卡片配合：面板高度随视口，过长时各自滚动 */
  height: calc(100vh - 320px);
  min-height: 420px;
}

.compose__side {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 240px;
  flex: 0 0 240px;
  padding: 10px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  background: var(--el-fill-color-blank);
}

.compose__side-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.compose__side-head .el-button:first-child {
  flex: 1;
}

.piles {
  flex: 1;
  margin: 0;
  padding: 0;
  list-style: none;
  overflow-y: auto;
}

.pile {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 8px;
  margin-bottom: 4px;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.2s;
}

.pile:hover {
  background: var(--el-fill-color-light);
}

.pile.is-active {
  background: var(--el-color-primary-light-9);
}

.pile__name {
  flex: 1;
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.piles__empty {
  padding: 16px 0;
  text-align: center;
  font-size: 13px;
  color: var(--el-text-color-secondary);
}

.compose__detail {
  flex: 1;
  min-width: 0;
  padding: 12px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  background: var(--el-fill-color-blank);
  overflow-y: auto;
}

.detail-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.detail-head__title {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.detail-head__title h3 {
  margin: 0;
  font-size: 20px;
  font-weight: 600;
}

.detail-head__actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.detail-head__actions .el-button + .el-button {
  margin-left: 0;
}

.detail-path {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 0 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}

.card {
  margin-bottom: 12px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  overflow: hidden;
}

.card__head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  background: var(--el-fill-color-light);
}

.card__title {
  font-size: 14px;
  font-weight: 600;
}

.card__count {
  padding: 0 6px;
  border-radius: 8px;
  background: var(--el-fill-color-dark);
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.card__spacer {
  flex: 1;
}

.card__body {
  padding: 10px;
}

.svc {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px;
  border-radius: 6px;
}

.svc + .svc {
  border-top: 1px solid var(--el-border-color-lighter);
}

.svc__main {
  flex: 1;
  min-width: 0;
}

.svc__name {
  font-size: 14px;
  font-weight: 500;
}

.svc__image {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card__log,
.card__yaml {
  margin: 0;
  padding: 10px;
  max-height: 320px;
  overflow: auto;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--el-fill-color-blank);
}

.card__log {
  color: var(--el-text-color-regular);
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
}
</style>
