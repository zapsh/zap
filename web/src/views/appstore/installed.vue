<template>
  <div class="installed-page">
    <!-- 页头 -->
    <el-card shadow="never" class="head-card">
      <div class="head-row">
        <div class="head-left">
          <el-icon :size="22" color="#409eff"><Box /></el-icon>
          <div>
            <div class="head-title">{{ t('appstoreInstalled.headTitle') }}</div>
            <div class="head-sub">
              {{ t('appstoreInstalled.headSub') }}
            </div>
          </div>
        </div>
        <div class="head-right">
          <span class="auto-tip">
            <el-switch v-model="autoRefresh" size="small" />
            {{ t('appstoreInstalled.autoRefresh') }}
          </span>
          <el-button :icon="Refresh" circle :disabled="loading" @click="load(true)" />
        </div>
      </div>
    </el-card>

    <!-- 列表 -->
    <el-card shadow="never" class="table-card">
      <div class="filter-bar">
        <el-select
          v-model="filterCategory"
          clearable
          :placeholder="t('appstoreInstalled.allCategories')"
          style="width: 150px"
          @change="keyword = keyword"
        >
          <el-option v-for="c in categories" :key="c" :label="catLabel(c)" :value="c" />
        </el-select>
        <el-select
          v-model="filterState"
          clearable
          :placeholder="t('appstoreInstalled.allStates')"
          style="width: 130px"
        >
          <el-option v-for="(m, key) in stateMeta" :key="key" :label="m.label" :value="key" />
        </el-select>
        <el-input
          v-model="keyword"
          clearable
          :placeholder="t('appstoreInstalled.searchPlaceholder')"
          style="width: 240px"
          :prefix-icon="Search"
        />
        <span class="count">{{
          t('appstoreInstalled.countInstances', { n: filtered.length })
        }}</span>
      </div>

      <el-table :data="filtered" v-loading="loading" stripe>
        <el-table-column :label="t('appstoreInstalled.colApp')" min-width="200">
          <template #default="{ row }">
            <div class="cell-name">
              <div class="app-name">{{ row.name }}</div>
              <div class="app-sub">
                <span class="mono">{{ row.instance }}</span>
                <span v-if="row.upgraded_from">{{
                  t('appstoreInstalled.upgradedFrom', { from: row.upgraded_from })
                }}</span>
              </div>
            </div>
          </template>
        </el-table-column>
        <el-table-column :label="t('appstoreInstalled.colCategory')" width="120">
          <template #default="{ row }">
            <el-tag size="small" effect="plain">{{ catLabel(row.category) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('appstoreInstalled.colOwner')" width="130">
          <template #default="{ row }">
            <template v-if="row.owner">
              <div class="mono">{{ row.owner }}</div>
              <div v-if="row.site_id" class="app-sub">
                {{ t('appstoreInstalled.siteTag', { id: row.site_id }) }}
              </div>
            </template>
            <el-tag v-else size="small" effect="plain" type="info">{{
              t('appstoreInstalled.scopeGlobal')
            }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="version" :label="t('appstoreInstalled.colVersion')" width="110" />
        <el-table-column label="expose" min-width="180">
          <template #default="{ row }">
            <span class="mono">{{ exposeOf(row) }}</span>
            <el-tag
              v-if="row.info.enabled === false"
              size="small"
              type="danger"
              effect="plain"
              style="margin-left: 6px"
              >{{ t('appstoreInstalled.disabledTag') }}</el-tag
            >
          </template>
        </el-table-column>
        <el-table-column :label="t('appstoreInstalled.colState')" width="110">
          <template #default="{ row }">
            <span class="state-cell">
              <i
                class="state-dot"
                :style="{ background: (stateMeta[row.state] || stateMeta.unknown).color }"
              />
              <span :style="{ color: (stateMeta[row.state] || stateMeta.unknown).color }">
                {{ (stateMeta[row.state] || stateMeta.unknown).label }}
              </span>
            </span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="300" fixed="right">
          <template #default="{ row }">
            <template v-if="canControl(row)">
              <template v-if="isAdmin">
                <el-button
                  size="small"
                  type="success"
                  plain
                  :disabled="busy === keyOf(row) || !canStart(row)"
                  @click="handleAction(row, 'start')"
                  >{{ t('appstoreInstalled.actStart') }}</el-button
                >
                <el-button
                  size="small"
                  type="warning"
                  plain
                  :disabled="busy === keyOf(row) || !canStop(row)"
                  @click="handleAction(row, 'stop')"
                  >{{ t('appstoreInstalled.actStop') }}</el-button
                >
                <el-button
                  size="small"
                  type="primary"
                  plain
                  :disabled="busy === keyOf(row) || !canRestart(row)"
                  @click="handleAction(row, 'restart')"
                  >{{ t('appstoreInstalled.actRestart') }}</el-button
                >
              </template>
              <el-tooltip v-else :content="t('appstoreInstalled.adminOnlyTooltip')" placement="top">
                <el-button size="small" type="primary" plain disabled>{{
                  t('appstoreInstalled.actControl')
                }}</el-button>
              </el-tooltip>
            </template>
            <el-tooltip v-else :content="t('appstoreInstalled.noSvcTooltip')" placement="top">
              <el-button size="small" type="primary" plain disabled>{{
                t('appstoreInstalled.actControl')
              }}</el-button>
            </el-tooltip>
            <el-button
              v-if="isAdmin"
              size="small"
              type="danger"
              plain
              :disabled="busy === keyOf(row)"
              @click="handleUninstall(row)"
              >{{ t('appstoreInstalled.actUninstall') }}</el-button
            >
            <el-button size="small" text type="primary" @click="showDetail(row)">{{
              t('appstoreInstalled.detail')
            }}</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('appstoreInstalled.empty')" :image-size="80" />
        </template>
      </el-table>
    </el-card>

    <!-- 详情抽屉 -->
    <el-drawer v-model="drawerVisible" :title="detailTitle" size="480px" destroy-on-close>
      <div v-if="current" class="detail-body">
        <div class="detail-state">
          <el-tag :type="(stateMeta[current.state] || stateMeta.unknown).tag" effect="dark">
            {{ (stateMeta[current.state] || stateMeta.unknown).label }}
          </el-tag>
          <el-tag v-if="current.info.enabled === false" type="danger">{{
            t('appstoreInstalled.disabledTag')
          }}</el-tag>
          <el-tag v-if="current.info.instance" type="primary" effect="plain">{{
            t('appstoreInstalled.instanceTag', { name: current.info.instance })
          }}</el-tag>
        </div>

        <el-descriptions :column="1" border size="small" class="detail-desc">
          <el-descriptions-item :label="t('appstoreInstalled.pkgPath')">{{
            current.pkg_path
          }}</el-descriptions-item>
          <el-descriptions-item :label="t('appstoreInstalled.colVersion')">{{
            current.version
          }}</el-descriptions-item>
          <el-descriptions-item :label="t('appstoreInstalled.colCategory')">{{
            catLabel(current.category)
          }}</el-descriptions-item>
          <el-descriptions-item :label="t('appstoreInstalled.source')">
            {{ current.source }}{{ current.repo_id ? ` / ${current.repo_id}` : '' }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('appstoreInstalled.installedAt')">{{
            fmtTime(current.installed_at)
          }}</el-descriptions-item>
          <el-descriptions-item :label="t('appstoreInstalled.lastRun')">{{
            current.run_id || '-'
          }}</el-descriptions-item>
        </el-descriptions>

        <!-- 配置文件快捷编辑（info.yaml: config_files 列表 / 兼容单值 config_file） -->
        <div v-if="editableFilesOf(current).length" class="cfg-block">
          <div class="cfg-head">
            <span class="info-title">{{ t('appstoreInstalled.configFiles') }}</span>
            <span v-if="!isAdmin" class="cfg-ro-tip">{{
              t('appstoreInstalled.adminOnlyEdit')
            }}</span>
          </div>
          <template v-if="editableFilesOf(current).length === 1">
            <div class="cfg-single">
              <div class="cfg-path mono">{{ editableFilesOf(current)[0].path }}</div>
              <el-dropdown
                v-if="isAdmin"
                trigger="click"
                @command="(c: string) => onEditCommand(editableFilesOf(current)[0], c)"
              >
                <el-button size="small" type="primary" plain>
                  {{ t('appstoreInstalled.editConfig')
                  }}<el-icon class="el-icon--right"><ArrowDown /></el-icon>
                </el-button>
                <template #dropdown>
                  <el-dropdown-menu>
                    <el-dropdown-item command="edit">{{
                      t('appstoreInstalled.editDirect')
                    }}</el-dropdown-item>
                    <el-dropdown-item command="backup">{{
                      t('appstoreInstalled.editAfterBackup')
                    }}</el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>
            </div>
          </template>
          <template v-else>
            <div v-for="f in editableFilesOf(current)" :key="f.path" class="cfg-item">
              <div class="cfg-item-main">
                <div class="cfg-item-label">{{ f.label }}</div>
                <div class="cfg-path mono">{{ f.path }}</div>
              </div>
              <el-dropdown
                v-if="isAdmin"
                trigger="click"
                @command="(c: string) => onEditCommand(f, c)"
              >
                <el-button size="small" type="primary" plain>
                  {{ t('appstoreInstalled.edit')
                  }}<el-icon class="el-icon--right"><ArrowDown /></el-icon>
                </el-button>
                <template #dropdown>
                  <el-dropdown-menu>
                    <el-dropdown-item command="edit">{{
                      t('appstoreInstalled.editDirect')
                    }}</el-dropdown-item>
                    <el-dropdown-item command="backup">{{
                      t('appstoreInstalled.editAfterBackup')
                    }}</el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>
            </div>
          </template>
        </div>

        <div class="info-title">{{ t('appstoreInstalled.infoTitle') }}</div>
        <el-empty
          v-if="!Object.keys(current.info || {}).length"
          :description="t('appstoreInstalled.noExtraInfo')"
          :image-size="50"
        />
        <div v-else class="info-grid">
          <template v-for="(v, k) in current.info" :key="k">
            <div v-if="k !== 'config_files'" class="info-row">
              <span class="info-key">{{ k }}</span>
              <span class="info-val mono">{{ fmtVal(v) }}</span>
            </div>
          </template>
        </div>

        <div v-if="isAdmin && canControl(current)" class="detail-actions">
          <el-button
            type="success"
            plain
            :disabled="busy === current.pkg_path || !canStart(current)"
            @click="handleAction(current, 'start')"
            >{{ t('appstoreInstalled.actStart') }}</el-button
          >
          <el-button
            type="warning"
            plain
            :disabled="busy === current.pkg_path || !canStop(current)"
            @click="handleAction(current, 'stop')"
            >{{ t('appstoreInstalled.actStop') }}</el-button
          >
          <el-button
            type="primary"
            plain
            :disabled="busy === current.pkg_path || !canRestart(current)"
            @click="handleAction(current, 'restart')"
            >{{ t('appstoreInstalled.actRestart') }}</el-button
          >
        </div>
        <p v-else-if="!canControl(current)" class="no-svc-tip">
          {{ t('appstoreInstalled.noSvcTip') }}
        </p>
      </div>
    </el-drawer>

    <!-- 配置文件编辑弹窗（支持 config_files 多文件 tab 切换） -->
    <el-dialog
      v-model="cfgVisible"
      :title="cfgTitle"
      width="820px"
      top="6vh"
      append-to-body
      destroy-on-close
      :before-close="requestClose"
    >
      <div class="cfg-editor-wrap">
        <el-tabs
          v-if="cfgTabs.length > 1"
          v-model="cfgActive"
          type="card"
          class="cfg-tabs"
          @tab-change="onTabChange"
        >
          <el-tab-pane v-for="t in cfgTabs" :key="t.path" :name="t.path">
            <template #label>
              <span>{{ tabLabel(t) }}</span>
            </template>
            <div class="cfg-pane">
              <div v-if="t.loading" v-loading="true" class="cfg-tab-loading" />
              <div v-else-if="t.error" class="cfg-error">{{ t.error }}</div>
              <template v-else-if="t.loaded">
                <div class="cfg-path-tip mono">{{ t.path }}</div>
                <CodeEditor
                  v-model="t.content"
                  class="cfg-editor"
                  :path="t.path"
                  :readonly="cfgSaving"
                  :placeholder="'# ' + t.path"
                />
              </template>
            </div>
          </el-tab-pane>
        </el-tabs>
        <div v-else-if="activeCfg" class="cfg-pane">
          <div v-if="activeCfg.loading" v-loading="true" class="cfg-tab-loading" />
          <div v-else-if="activeCfg.error" class="cfg-error">{{ activeCfg.error }}</div>
          <template v-else-if="activeCfg.loaded">
            <div class="cfg-path-tip mono">{{ activeCfg.path }}</div>
            <CodeEditor
              v-model="activeCfg.content"
              class="cfg-editor"
              :path="activeCfg.path"
              :readonly="cfgSaving"
              :placeholder="'# ' + activeCfg.path"
            />
          </template>
        </div>
        <div v-if="activeCfg" class="cfg-save-bar">
          <span v-if="dirtyCount === 0" class="cfg-hint">{{
            t('appstoreInstalled.cfgUnchanged')
          }}</span>
          <span v-else class="cfg-hint">{{
            t('appstoreInstalled.cfgDirty', { n: dirtyCount })
          }}</span>
          <el-button @click="requestClose()">{{ t('common.cancel') }}</el-button>
          <el-button
            v-if="cfgTabs.length > 1 && dirtyCount > 0"
            type="primary"
            :loading="cfgSaving"
            @click="saveAllCfg"
            >{{ t('appstoreInstalled.saveAll') }}</el-button
          >
          <el-button
            type="primary"
            :loading="cfgSaving"
            :disabled="!activeCfg.dirty"
            @click="saveCfg"
            >{{ t('common.save') }}</el-button
          >
        </div>
      </div>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { ArrowDown, Box, Refresh, Search } from '@/icons'
import { useUserStore } from '@/stores/user'
import {
  getInstalledApps,
  instanceAction,
  uninstallPackage,
  type InstalledApp,
} from '@/api/appstore'
import { readFile, writeFile } from '@/api/file'
import CodeEditor from '@/components/CodeEditor.vue'

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

// scope: all=全部可见（管理员）；mine=只看当前用户的站点应用
const props = withDefaults(defineProps<{ scope?: 'all' | 'mine' }>(), { scope: 'all' })
// 提交任务后把 run_id 交给外层（商店页据此打开日志抽屉）
const emit = defineEmits<{ (e: 'task', runId: string, title: string): void }>()

/** 实例的稳定标识：多实例时 pkg_path 不唯一，操作与加载态都以它为准 */
function keyOf(app: InstalledApp): string {
  return app.instance_key || `${app.pkg_path}@${app.instance || 'default'}`
}

const categoryLabels = computed<Record<string, string>>(() => ({
  infra: t('appstoreInstalled.catInfra'),
  application: t('appstoreInstalled.catApplication'),
  webapps: t('appstoreInstalled.catWebapps'),
  database: t('appstoreInstalled.catDatabase'),
  library: t('appstoreInstalled.catLibrary'),
}))
const CATEGORY_ORDER = ['infra', 'application', 'webapps', 'database', 'library']

const stateMeta = computed<
  Record<string, { label: string; tag: 'success' | 'info' | 'danger' | 'warning'; color: string }>
>(() => ({
  running: { label: t('appstoreInstalled.stateRunning'), tag: 'success', color: '#67c23a' },
  stopped: {
    label: t('appstoreInstalled.stateStopped'),
    tag: 'info',
    color: 'var(--el-text-color-secondary)',
  },
  failed: { label: t('appstoreInstalled.stateFailed'), tag: 'danger', color: '#f56c6c' },
  starting: { label: t('appstoreInstalled.stateStarting'), tag: 'warning', color: '#e6a23c' },
  stopping: { label: t('appstoreInstalled.stateStopping'), tag: 'warning', color: '#e6a23c' },
  unknown: {
    label: t('appstoreInstalled.stateUnknown'),
    tag: 'info',
    color: 'var(--el-text-color-placeholder)',
  },
}))

const actLabels = computed<Record<string, string>>(() => ({
  start: t('appstoreInstalled.actStart'),
  stop: t('appstoreInstalled.actStop'),
  restart: t('appstoreInstalled.actRestart'),
}))

const list = ref<InstalledApp[]>([])
const loading = ref(false)
const busy = ref('')
const keyword = ref('')
const filterCategory = ref('')
const filterState = ref('')
const autoRefresh = ref(true)
const drawerVisible = ref(false)
const current = ref<InstalledApp | null>(null)

let timer: ReturnType<typeof setInterval> | null = null

const categories = computed(() => {
  const set = new Set(list.value.map((i) => i.category).filter(Boolean))
  const rank = (c: string) => {
    const i = CATEGORY_ORDER.indexOf(c)
    return i === -1 ? 99 : i
  }
  return Array.from(set).sort((a, b) => rank(a) - rank(b))
})

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return list.value.filter((i) => {
    if (filterCategory.value && i.category !== filterCategory.value) return false
    if (filterState.value && i.state !== filterState.value) return false
    // 「我的站点应用」只看归属自己的站点类应用（后端已隔离，这里再兜一层）
    if (props.scope === 'mine' && !i.owner) return false
    if (!kw) return true
    const hay =
      `${i.name} ${i.instance} ${i.pkg_path} ${i.owner || ''} ${i.site_id || ''} ${i.info.install_dir || ''} ${i.info.expose || ''}`.toLowerCase()
    return hay.includes(kw)
  })
})

const detailTitle = computed(() => {
  if (!current.value) return ''
  return `${current.value.name} ${current.value.version} · ${current.value.instance}`
})

function catLabel(cat: string): string {
  return categoryLabels.value[cat] || cat || '-'
}

function exposeOf(app: InstalledApp): string {
  if (app.info.expose) return String(app.info.expose)
  if (app.info.port) return `tcp:${app.info.port}`
  return '-'
}

function fmtVal(v: unknown): string {
  if (typeof v === 'boolean') return v ? t('common.yes') : t('common.no')
  if (v === null || v === undefined || v === '') return '-'
  return String(v)
}

function fmtTime(ts: number | null | undefined): string {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

// 只有登记了 systemd 服务（svc_name）才允许面板启停
function canControl(app: InstalledApp): boolean {
  return !!app.info.svc_name
}
function canStart(app: InstalledApp): boolean {
  return ['stopped', 'failed', 'unknown'].includes(app.state)
}
function canStop(app: InstalledApp): boolean {
  return app.state !== 'stopped'
}
function canRestart(app: InstalledApp): boolean {
  return !['starting', 'stopping'].includes(app.state)
}

async function load(force = false) {
  if (force) loading.value = true
  try {
    const resp = await getInstalledApps()
    const items: InstalledApp[] = resp.data?.items || []
    // 保留抽屉里的引用以实时刷新
    list.value = items
    if (current.value) {
      current.value =
        items.find((i) => keyOf(i) === keyOf(current.value!)) || current.value
    }
  } catch (e: any) {
    ElMessage.error(e.message || t('appstoreInstalled.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function handleAction(app: InstalledApp, action: string) {
  try {
    const act = actLabels.value[action] || action
    await ElMessageBox.confirm(
      t('appstoreInstalled.confirmAction', { action: act, name: app.name, instance: app.instance }),
      t('appstoreInstalled.confirmTitle', { action: act }),
      { type: action === 'stop' ? 'warning' : 'info' },
    )
  } catch {
    return
  }
  busy.value = keyOf(app)
  try {
    const resp = await instanceAction({
      pkg_path: app.pkg_path,
      instance: app.instance,
      action,
    })
    const st = resp.data?.state
    const meta = stateMeta.value[st]
    ElMessage.success(
      t('appstoreInstalled.actionOk', {
        action: actLabels.value[action] || action,
        instance: app.instance,
        state: meta ? meta.label : st || t('appstoreInstalled.stateUnknown'),
      }),
    )
    await load()
  } catch (e: any) {
    ElMessage.error(
      e.message ||
        t('appstoreInstalled.actionFailed', { action: actLabels.value[action] || action }),
    )
  } finally {
    busy.value = ''
  }
}

async function handleUninstall(app: InstalledApp) {
  const label = `${app.name}${app.instance && app.instance !== app.name ? ` · ${app.instance}` : ''}`
  try {
    await ElMessageBox.confirm(
      t('appstore.uninstallConfirm', { name: label }),
      t('appstore.uninstallTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  busy.value = keyOf(app)
  try {
    const resp = await uninstallPackage({ pkg_path: app.pkg_path, instance: app.instance })
    ElMessage.success(t('appstoreInstalled.uninstallOk'))
    // 外层（商店页）会拿 run_id 打开日志抽屉；独立路由页则忽略
    if (resp.data?.run_id) emit('task', resp.data.run_id, `${t('appstoreInstalled.actUninstall')} ${label}`)
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.uninstallFailed'))
  } finally {
    busy.value = ''
  }
}

function showDetail(app: InstalledApp) {
  current.value = app
  drawerVisible.value = true
}

// ── 配置文件可视化编辑（info.yaml：config_files 列表 / 兼容 config_file，仅 admin 可读写）──
interface EditableFile {
  path: string
  label: string
}
interface CfgTab extends EditableFile {
  content: string
  original: string
  loaded: boolean
  loading: boolean
  dirty: boolean
  error: string
}

function fileBaseName(path: string): string {
  return path.split('/').filter(Boolean).pop() || path
}

/** 解析可编辑文件列表：优先 info.config_files（string | {path,label} 数组），回退 config_file（string | string[]） */
function editableFilesOf(app: InstalledApp | null): EditableFile[] {
  const raw = app?.info?.config_files ?? app?.info?.config_file
  if (!raw) return []
  const list = Array.isArray(raw) ? raw : [raw]
  const out: EditableFile[] = []
  for (const it of list) {
    if (typeof it === 'string') {
      if (it.trim()) out.push({ path: it, label: fileBaseName(it) })
    } else if (it && typeof it === 'object') {
      const p = (it as { path?: unknown }).path
      const lb = (it as { label?: unknown }).label
      if (typeof p === 'string' && p.trim()) {
        out.push({
          path: p,
          label: typeof lb === 'string' && lb.trim() ? lb : fileBaseName(p),
        })
      }
    }
  }
  return out
}

const cfgVisible = ref(false)
const cfgActive = ref('')
const cfgTabs = ref<CfgTab[]>([])
const cfgSaving = ref(false)

const activeCfg = computed(() => cfgTabs.value.find((t) => t.path === cfgActive.value) || null)
const dirtyCount = computed(() => cfgTabs.value.filter((t) => t.dirty).length)
const cfgTitle = computed(() => {
  const name =
    cfgTabs.value.length > 1
      ? t('appstoreInstalled.cfgFileCount', { n: cfgTabs.value.length })
      : activeCfg.value?.label || 'file'
  return t('appstoreInstalled.cfgTitle', { name })
})

function tabLabel(t: CfgTab): string {
  return t.dirty ? `${t.label} ●` : t.label
}

function onTabChange(name: string | number) {
  void loadTab(String(name))
}

// 任一 tab 内容变化(来自 CodeMirror 编辑)时刷新脏标记
watch(
  () => cfgTabs.value.map((t) => t.content),
  () => {
    for (const t of cfgTabs.value) t.dirty = t.content !== t.original
  },
)

function openCfgEditor(files: EditableFile[], activePath?: string) {
  cfgTabs.value = files.map((f) => ({
    ...f,
    content: '',
    original: '',
    loaded: false,
    loading: false,
    dirty: false,
    error: '',
  }))
  cfgActive.value = activePath || files[0]?.path || ''
  cfgVisible.value = true
  if (cfgActive.value) void loadTab(cfgActive.value)
}

function tsStamp(): string {
  const d = new Date()
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
}

/** 把服务器当前文件内容复制为 <path>.bak.<时间戳>，返回是否成功（false 表示用户放弃） */
async function backupPath(path: string): Promise<boolean> {
  let content = ''
  try {
    const resp = await readFile(path)
    content = resp.data?.content ?? ''
    if (content.includes('\u0000')) {
      ElMessage.warning(t('appstoreInstalled.binWarn'))
      return true
    }
  } catch (e: any) {
    try {
      await ElMessageBox.confirm(
        t('appstoreInstalled.readFailedBackup'),
        t('appstoreInstalled.notice'),
        {
          type: 'warning',
          confirmButtonText: t('appstoreInstalled.continueEdit'),
          cancelButtonText: t('common.cancel'),
        },
      )
    } catch {
      return false
    }
    return true
  }
  const bakPath = `${path}.bak.${tsStamp()}`
  try {
    await writeFile(bakPath, content)
    ElMessage.success(t('appstoreInstalled.backupOk', { path: bakPath }))
    return true
  } catch (e: any) {
    ElMessage.error(t('appstoreInstalled.backupFailed', { msg: e?.message || '' }))
    return false
  }
}

/** 编辑入口命令：edit=直接打开；backup=先备份当前文件再打开（多文件时激活该 tab） */
async function onEditCommand(target: EditableFile, cmd: string) {
  if (!current.value) return
  if (cmd === 'backup' && !(await backupPath(target.path))) return
  const files = editableFilesOf(current.value)
  openCfgEditor(files, files.length > 1 ? target.path : undefined)
}

async function loadTab(path: string) {
  const tab = cfgTabs.value.find((t) => t.path === path)
  if (!tab || tab.loaded || tab.loading) return
  tab.loading = true
  tab.error = ''
  try {
    const resp = await readFile(path)
    const content = resp.data?.content ?? ''
    if (content.includes('\u0000')) {
      tab.error = t('appstoreInstalled.binFile')
      return
    }
    tab.content = content
    tab.original = content
    tab.loaded = true
  } catch (e: any) {
    tab.error = e.message || t('appstoreInstalled.readCfgFailed')
  } finally {
    tab.loading = false
  }
}

async function saveTab(t: CfgTab) {
  await writeFile(t.path, t.content)
  t.original = t.content
  t.dirty = false
}

async function saveCfg() {
  const tab = activeCfg.value
  if (!tab) return
  cfgSaving.value = true
  try {
    await saveTab(tab)
    finishSave()
  } catch (e: any) {
    ElMessage.error(e.message || t('appstoreInstalled.saveFailed'))
  } finally {
    cfgSaving.value = false
  }
}

async function saveAllCfg() {
  const dirtyTabs = cfgTabs.value.filter((t) => t.dirty)
  cfgSaving.value = true
  try {
    for (const t of dirtyTabs) await saveTab(t)
    finishSave()
  } catch (e: any) {
    ElMessage.error(e.message || t('appstoreInstalled.saveFailed'))
  } finally {
    cfgSaving.value = false
  }
}

function finishSave() {
  const left = cfgTabs.value.filter((t) => t.dirty)
  if (left.length === 0) {
    ElMessage.success(t('appstoreInstalled.cfgSaved'))
    cfgVisible.value = false
    ElMessage.info({
      message: t('appstoreInstalled.cfgRestartTip'),
      duration: 3500,
    })
  } else {
    ElMessage.success(t('appstoreInstalled.cfgSavedLeft', { n: left.length }))
  }
}

/** 关闭前如有未保存修改需确认（用于取消按钮与 dialog 关闭拦截） */
async function requestClose(done?: () => void) {
  if (dirtyCount.value > 0) {
    try {
      await ElMessageBox.confirm(
        t('appstoreInstalled.unsavedClose'),
        t('appstoreInstalled.unsavedTitle'),
        {
          type: 'warning',
          confirmButtonText: t('appstoreInstalled.discard'),
          cancelButtonText: t('appstoreInstalled.continueEdit'),
        },
      )
    } catch {
      return
    }
  }
  if (done) done()
  else cfgVisible.value = false
}

onMounted(() => {
  load(true)
  timer = setInterval(() => {
    if (autoRefresh.value && !busy.value) load()
  }, 3000)
})

onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.installed-page {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 2px;
}
.head-card {
  border-radius: 10px;
}
.head-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
}
.head-left {
  display: flex;
  align-items: center;
  gap: 10px;
}
.head-title {
  font-size: 16px;
  font-weight: 600;
}
.head-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}
.head-right {
  display: flex;
  align-items: center;
  gap: 14px;
}
.auto-tip {
  font-size: 13px;
  color: var(--el-text-color-secondary);
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.table-card {
  border-radius: 10px;
}
.filter-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.count {
  margin-left: auto;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.cell-name .app-name {
  font-weight: 600;
}
.cell-name .app-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
  font-size: 12px;
}
.state-cell {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
}
.state-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
  background: var(--el-text-color-placeholder);
}
.detail-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.detail-state {
  display: flex;
  gap: 8px;
}
.info-title {
  font-size: 13px;
  font-weight: 600;
  border-left: 3px solid var(--el-color-primary);
  padding-left: 8px;
}
.info-grid {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  overflow: hidden;
}
.info-row {
  display: flex;
  font-size: 13px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.info-row:last-child {
  border-bottom: none;
}
.info-key {
  width: 130px;
  flex-shrink: 0;
  padding: 8px 10px;
  background: var(--el-fill-color-light);
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
.info-val {
  flex: 1;
  padding: 8px 10px;
  word-break: break-all;
}
.detail-actions {
  display: flex;
  gap: 10px;
  margin-top: 4px;
}
.no-svc-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  background: var(--el-fill-color-light);
  border-radius: 8px;
  padding: 8px 10px;
  line-height: 1.6;
}
.cfg-block {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  background: var(--el-bg-color);
}
.cfg-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.cfg-path {
  font-size: 12px;
  word-break: break-all;
  color: var(--el-text-color-secondary);
}
.cfg-editor-wrap {
  min-height: 200px;
}
.cfg-error {
  color: var(--el-color-danger);
  font-size: 13px;
  padding: 20px 0;
  text-align: center;
}
.cfg-path-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 8px;
  word-break: break-all;
}
.cfg-editor {
  height: min(48vh, 460px);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  overflow: hidden;
}
.cfg-save-bar {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 10px;
  margin-top: 12px;
}
.cfg-hint {
  margin-right: auto;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.cfg-ro-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.cfg-single {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 8px;
}
.cfg-single .cfg-path {
  flex: 1;
  margin-top: 0;
}
.cfg-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  margin-top: 8px;
}
.cfg-item-main {
  flex: 1;
  min-width: 0;
}
.cfg-item-label {
  font-size: 13px;
  color: var(--el-text-color-primary);
  margin-bottom: 2px;
}
.cfg-tab-loading {
  min-height: 60px;
  display: flex;
  align-items: center;
  justify-content: center;
}
</style>
