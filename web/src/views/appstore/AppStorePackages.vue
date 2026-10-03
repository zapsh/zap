<template>
  <div class="appstore-packages">
    <!-- 分类 + 搜索 + 任务队列入口 -->
    <div class="filter-bar">
      <el-radio-group v-model="activeCategory" size="small">
        <el-radio-button value="all">{{ t('appstore.catAll') }}</el-radio-button>
        <el-radio-button v-for="c in visibleCategories" :key="c.value" :value="c.value">{{
          c.label
        }}</el-radio-button>
      </el-radio-group>
      <div class="filter-bar__right">
        <el-input
          v-model="keyword"
          :placeholder="t('appstore.searchPlaceholder')"
          clearable
          style="width: 240px"
          size="small"
        >
          <template #prefix><el-icon><Search /></el-icon></template>
        </el-input>
        <el-button size="small" :icon="List" @click="openQueue">
          {{ t('appstore.queueBtn') }}
          <el-tag v-if="activeCount" size="small" type="warning" effect="dark" round>
            {{ activeCount }}
          </el-tag>
        </el-button>
      </div>
    </div>

    <!-- 包列表（表格形式） -->
    <div class="pkg-table-wrap" v-loading="loading">
      <el-table :data="filteredPackages" row-key="pkg_path" stripe style="width: 100%">
        <el-table-column label="名称" min-width="210">
          <template #default="{ row }">
            <div class="cell-name">
              <span class="cell-name__title">{{ row.name }}</span>
              <div class="cell-name__tags">
                <el-tag v-if="row.system_shared && !isAdmin" size="small" type="warning" effect="light">{{
                  t('appstore.tagSystemShared')
                }}</el-tag>
                <el-tag v-else-if="row.installed" size="small" type="success" effect="light">{{
                  (row.installed_instances || []).length > 1
                    ? `${t('appstore.tagInstalled')} × ${(row.installed_instances || []).length}`
                    : t('appstore.tagInstalled')
                }}</el-tag>
                <el-tag v-else size="small" type="info" effect="plain">{{ t('appstore.tagNotInstalled') }}</el-tag>
                <el-tag v-if="row.allow_multiple_instances" size="small" type="primary" effect="plain">{{
                  t('appstore.tagMultiVersion')
                }}</el-tag>
                <el-tag v-if="row.source === 'custom'" size="small" type="warning" effect="light">{{
                  t('appstore.tagCustom')
                }}</el-tag>
                <el-tag v-else-if="row.repo_id" size="small" type="info" effect="plain">{{ row.repo_id }}</el-tag>
              </div>
            </div>
          </template>
        </el-table-column>

        <el-table-column label="描述" min-width="300">
          <template #default="{ row }">
            <div class="cell-desc">
              <div class="cell-desc__title">{{ row.title || row.name }}</div>
              <div class="cell-desc__body">{{ row.description || t('appstore.noDescription') }}</div>
              <div v-if="depList(row).length || row.default_port" class="cell-desc__meta">
                <span v-if="depList(row).length">{{
                  t('appstore.depColon', { list: depList(row).join(t('appstore.depSep')) })
                }}</span>
                <span v-if="row.default_port">{{ t('appstore.portColon', { port: row.default_port }) }}</span>
              </div>
            </div>
          </template>
        </el-table-column>

        <el-table-column label="版本" width="300">
          <template #default="{ row }">
            <span v-if="row.versions && row.versions.length > 1" class="pkg-ver-sel">
              {{ t('appstore.versionColon') }}
              <el-select
                size="small"
                :style="{ width: versionGroups(row).length ? '210px' : '130px' }"
                :model-value="selVersion[row.pkg_path] || row.version"
                @change="onSelVersion(row, $event)"
              >
                <template v-if="versionGroups(row).length">
                  <el-option-group v-for="g in versionGroups(row)" :key="g.family" :label="g.label">
                    <el-option v-for="ver in g.versions" :key="ver" :label="ver" :value="ver">
                      <div class="ver-opt-row">
                        <span>{{ ver }}</span>
                        <el-tag :type="familyTagType(g.family)" size="small" effect="plain">{{ g.short }}</el-tag>
                      </div>
                    </el-option>
                  </el-option-group>
                </template>
                <el-option v-else v-for="ver in row.versions" :key="ver" :label="ver" :value="ver" />
              </el-select>
              <el-tag
                v-if="familyOf(row, curVersion(row))"
                class="ver-fam-tag"
                :type="familyTagType(familyOf(row, curVersion(row)))"
                size="small"
                effect="light"
                >{{ familyLabelOf(row, curVersion(row)) }}</el-tag
              >
            </span>
            <span v-else>{{ t('appstore.versionColon') }} <b>{{ row.version || '-' }}</b></span>
          </template>
        </el-table-column>

        <el-table-column label="已安装" width="170">
          <template #default="{ row }">
            <template v-if="row.installed">
              <div class="cell-installed">
                <b>{{ row.installed_version || '-' }}</b>
                <el-tag
                  v-if="familyOf(row, row.installed_version)"
                  size="small"
                  :type="familyTagType(familyOf(row, row.installed_version))"
                  effect="plain"
                  >{{ familyLabelOf(row, row.installed_version) }}</el-tag
                >
                <div v-if="row.upgraded_from" class="cell-installed__from">
                  {{ t('appstore.upgradedFrom', { from: row.upgraded_from }) }}
                </div>
              </div>
            </template>
            <span v-else class="muted">-</span>
          </template>
        </el-table-column>

        <el-table-column :label="t('common.operation')" width="300" fixed="right">
          <template #default="{ row }">
            <div class="cell-actions">
              <template v-if="!row.installed">
                <template v-if="actionEntries(row).length">
                  <el-button
                    v-for="[key, label] in actionEntries(row)"
                    :key="key"
                    size="small"
                    type="primary"
                    :disabled="!canOperatePkg(row)"
                    @click="handleInstall(row, key)"
                    >{{ label }}</el-button
                  >
                </template>
                <el-button
                  v-else
                  size="small"
                  type="primary"
                  :disabled="!canOperatePkg(row)"
                  @click="handleInstall(row)"
                  >{{ t('appstore.btnInstall') }}</el-button
                >
              </template>
              <template v-else>
                <el-button
                  v-if="!row.allow_multiple_instances && !actionEntries(row).length"
                  size="small"
                  type="primary"
                  plain
                  :disabled="!canOperatePkg(row)"
                  @click="handleInstall(row)"
                  >{{ t('appstore.btnReinstall') }}</el-button
                >
                <el-button
                  v-if="row.allow_multiple_instances && !actionEntries(row).length"
                  size="small"
                  type="primary"
                  plain
                  :disabled="!canOperatePkg(row)"
                  @click="handleInstall(row)"
                  >{{ t('appstore.btnInstallAgain') }}</el-button
                >
                <el-button
                  v-for="[key, label] in actionEntries(row)"
                  :key="key"
                  size="small"
                  type="success"
                  plain
                  :disabled="!canOperatePkg(row)"
                  @click="handleInstall(row, key)"
                  >{{ label }}</el-button
                >
                <el-button
                  v-if="!row.allow_multiple_instances"
                  size="small"
                  type="warning"
                  plain
                  :disabled="!canOperatePkg(row)"
                  @click="handleUpgrade(row)"
                  >{{ t('appstore.btnUpgrade') }}</el-button
                >
                <el-button
                  size="small"
                  type="danger"
                  plain
                  :disabled="!canOperatePkg(row)"
                  @click="handleUninstall(row)"
                  >{{ t('appstore.btnUninstall') }}</el-button
                >
              </template>
            </div>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('appstore.noPackages')" :image-size="80" />
        </template>
      </el-table>
    </div>

    <!-- 任务队列抽屉 -->
    <el-drawer v-model="queueVisible" :title="t('appstore.queueTitle')" size="72%" destroy-on-close>
      <TaskQueuePanel ref="queuePanelRef" kind="appstore" @changed="onQueueChanged" />
    </el-drawer>

    <!-- 安装/升级选项对话框 -->
    <el-dialog
      v-model="optDialogVisible"
      :title="optDialogTitle"
      width="620px"
      :close-on-click-modal="false"
    >
      <el-form v-if="optList.length" label-width="140px" label-position="left" @submit.prevent>
        <el-form-item
          v-for="o in optList"
          :key="o.name"
          :label="optLabel(o)"
          :required="!!o.required"
          :error="optFieldError[o.name]"
        >
          <template v-if="optType(o) === 'string'">
            <el-input
              v-model="optValues[o.name]"
              :placeholder="o.placeholder || ''"
              clearable
              maxlength="4096"
            />
          </template>
          <template v-else-if="optType(o) === 'number'">
            <el-input-number
              v-model="optValues[o.name]"
              :placeholder="o.placeholder || t('appstore.optNumberPlaceholder')"
              controls-position="right"
              style="width: 100%"
            />
          </template>
          <template v-else-if="optType(o) === 'bool'">
            <el-switch v-model="optValues[o.name]" />
          </template>
          <template v-else-if="optType(o) === 'select'">
            <el-select
              v-model="optValues[o.name]"
              style="width: 100%"
              clearable
              :placeholder="o.placeholder || t('appstore.optSelectPlaceholder')"
            >
              <el-option
                v-for="c in choicesOf(o)"
                :key="c.value"
                :label="c.label"
                :value="c.value"
              />
            </el-select>
          </template>
          <template v-else>
            <el-select
              v-model="optValues[o.name]"
              multiple
              collapse-tags
              style="width: 100%"
              :placeholder="o.placeholder || t('appstore.optMultiPlaceholder')"
            >
              <el-option
                v-for="c in choicesOf(o)"
                :key="c.value"
                :label="c.label"
                :value="c.value"
              />
            </el-select>
          </template>
          <div v-if="o.desc" class="opt-tip">{{ o.desc }}</div>
        </el-form-item>
      </el-form>
      <div v-if="optIntro" class="opt-intro">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ optIntro }}</span>
      </div>
      <template #footer>
        <el-button @click="optDialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="optSubmitting" @click="submitOptions">
          {{ optConfirmLabel }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 日志抽屉 -->
    <AppStoreLogDrawer ref="logDrawerRef" />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { List, Search, InfoFilled } from '@/icons'
import { useUserStore } from '@/stores/user'
import TaskQueuePanel from '@/components/TaskQueuePanel.vue'
import { getTasks } from '@/api/task'
import {
  getPackages,
  installPackage,
  uninstallPackage,
  upgradePackage,
  getRuns,
  type AppPackage,
  type AppOption,
  type AppChoice,
  type FormOptions,
  type RunItem,
  type VersionMeta,
} from '@/api/appstore'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'

const props = defineProps<{
  /** 仅展示这些分类（如市场页传 ['webapps','plugins']）；不传 = 全部 */
  categories?: string[]
  /** 多实例卸载时父页（完整商店）跳到「已安装」页签；市场页不传则仅提示 */
  onMultiInstanceUninstall?: () => void
}>()

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

const CAT_KEY: Record<string, string> = {
  infra: 'catInfra',
  application: 'catApplication',
  webapps: 'catWebapps',
  database: 'catDatabase',
  library: 'catLibrary',
  plugins: 'catPlugins',
  tools: 'catTools',
}
const ALL_CATS = Object.keys(CAT_KEY)

/**
 * 可见分类：
 * - 显式传了 categories（如市场页只给 webapps+plugins）则用它；
 * - 否则按角色收敛：admin 看全部；非 admin 给 webapps + plugins。
 *   插件安装目录由包的 roles 决定：仅声明了 admin 的包 → 系统目录（全用户共享）；
 *   声明了非 admin 角色的包 → 装到安装者自己的用户目录。故非 admin 也能在商店
 *   看到/安装插件，只是落到各自用户目录，与「上传插件」同一套隔离。
 * 再叠加包 app.yaml 的 roles 白名单（canViewPkg）做 scope 门禁。
 */
const effectiveCategories = computed<string[] | null>(() => {
  if (props.categories && props.categories.length) return props.categories
  return isAdmin.value ? null : ['webapps', 'plugins']
})

const visibleCategories = computed(() => {
  const cats = effectiveCategories.value ?? ALL_CATS
  return cats.map((v) => ({ value: v, label: t(`appstore.${CAT_KEY[v]}`) }))
})

/**
 * 当前用户能否浏览该包：admin 恒可见；app.yaml roles 空/未声明 = 仅 admin 可见；
 * 声明了 roles = admin + 命中白名单的角色可见。这是后端 `check_pkg_roles` 的前端镜像。
 */
function canViewPkg(pkg: AppPackage): boolean {
  if (isAdmin.value) return true
  const rs = pkg.roles
  if (!rs || rs.length === 0) return false
  return rs.some((r) => userStore.roles.includes(r))
}

/**
 * 当前用户能否对该包执行安装/升级/卸载（scope 控制）：
 * custom 包仅管理员；官方包按 roles 命中白名单方可操作。后端同逻辑二次校验。
 */
function canOperatePkg(pkg: AppPackage): boolean {
  // 系统级已装的插件对普通用户只读：不能安装 / 升级 / 卸载（共享，由管理员管理）
  if (pkg.system_shared && !isAdmin.value) return false
  if (pkg.source === 'custom') return isAdmin.value
  return canViewPkg(pkg)
}

// ── 包列表 ──────────────────────────────────────────────────

const packages = ref<AppPackage[]>([])
const loading = ref(false)
const activeCategory = ref('all')
const keyword = ref('')

const filteredPackages = computed(() => {
  // 先按包角色白名单过滤（admin 恒可见；roles 空 = 仅 admin；命中角色也可见）
  let list = packages.value.filter(canViewPkg)
  // 组件限定分类（admin 全量；非 admin 仅 webapps + plugins）
  if (effectiveCategories.value && effectiveCategories.value.length) {
    list = list.filter((p) => effectiveCategories.value!.includes(p.category))
  }
  if (activeCategory.value !== 'all') {
    list = list.filter((p) => p.category === activeCategory.value)
  }
  const kw = keyword.value.trim().toLowerCase()
  if (kw) {
    list = list.filter(
      (p) =>
        p.name.toLowerCase().includes(kw) ||
        (p.title || '').toLowerCase().includes(kw) ||
        (p.description || '').toLowerCase().includes(kw),
    )
  }
  return list
})

async function loadPackages() {
  loading.value = true
  try {
    const resp = await getPackages()
    packages.value = resp.data.packages || []
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.loadPkgsFailed'))
  } finally {
    loading.value = false
  }
}

// ── 版本 / 动作 / 依赖辅助 ─────────────────────────────────

const selVersion = ref<Record<string, string>>({})

function onSelVersion(pkg: AppPackage, v: string) {
  selVersion.value[pkg.pkg_path] = v
}

function curVersion(pkg: AppPackage): string {
  return selVersion.value[pkg.pkg_path] || pkg.version || ''
}

// ── 版本家族元数据 ──────────────────────────────────────────

function metaOf(pkg: AppPackage, ver?: string | null): VersionMeta | undefined {
  if (!ver || !pkg.version_meta) return undefined
  return pkg.version_meta[ver]
}

function familyOf(pkg: AppPackage, ver?: string | null): string {
  const f = metaOf(pkg, ver)?.family
  return typeof f === 'string' ? f.trim() : ''
}

function familyLabelOf(pkg: AppPackage, ver?: string | null): string {
  const f = familyOf(pkg, ver)
  if (!f) return ''
  return metaOf(pkg, ver)?.label || f
}

const FAMILY_TAG_TYPE: Record<string, 'primary' | 'success' | 'warning' | 'info'> = {
  mysql: 'primary',
  mariadb: 'success',
}
function familyTagType(f: string): 'primary' | 'success' | 'warning' | 'info' {
  return FAMILY_TAG_TYPE[f] || 'primary'
}

function verLabel(pkg: AppPackage, ver: string): string {
  const f = familyOf(pkg, ver)
  return f ? `${familyLabelOf(pkg, ver)} ${ver}` : `v${ver}`
}

function versionGroups(
  pkg: AppPackage,
): Array<{ family: string; label: string; short: string; versions: string[] }> {
  if (!pkg.version_meta) return []
  const groups: Array<{ family: string; label: string; short: string; versions: string[] }> = []
  for (const ver of pkg.versions || []) {
    const f = familyOf(pkg, ver)
    if (!f) return []
    const meta = metaOf(pkg, ver)
    const last = groups[groups.length - 1]
    if (last && last.family === f) last.versions.push(ver)
    else
      groups.push({
        family: f,
        label: meta?.group || meta?.label || f,
        short: meta?.label || f,
        versions: [ver],
      })
  }
  return groups.length > 1 ? groups : []
}

function actionEntries(pkg: AppPackage): Array<[string, string]> {
  const a = pkg.actions
  if (!a) return []
  return Object.entries(a).map(([k, label]) => [k, (label || '').trim() || k])
}

function actionLabel(pkg: AppPackage, key?: string): string {
  if (!key) return ''
  return actionEntries(pkg).find(([k]) => k === key)?.[1] || key
}

function depList(pkg: AppPackage): string[] {
  const d = pkg.dependencies as unknown
  if (!d) return pkg.deps || []
  if (Array.isArray(d)) return d as string[]
  return Object.entries(d as Record<string, string>).map(([n, v]) => (v ? `${n} ${v}` : n))
}

// ── 安装/升级选项 ───────────────────────────────────────────

type OptMode = 'install' | 'upgrade' | 'uninstall'

const optDialogVisible = ref(false)
const optMode = ref<OptMode>('install')
const optPkg = ref<AppPackage | null>(null)
const optActionKey = ref('')
const optList = ref<AppOption[]>([])
const optValues = ref<Record<string, any>>({})
const optFieldError = ref<Record<string, string>>({})
const optSubmitting = ref(false)
const optIntro = ref('')

function optModeName(m: OptMode, pkg: AppPackage): string {
  if (m === 'upgrade') return t('appstore.btnUpgrade')
  if (m === 'uninstall') return t('appstore.btnUninstall')
  if (!pkg.installed) return t('appstore.btnInstall')
  return pkg.allow_multiple_instances ? t('appstore.btnInstallAgain') : t('appstore.btnReinstall')
}

const optDialogTitle = computed(() => {
  const pkg = optPkg.value
  if (!pkg) return ''
  const extra =
    optMode.value === 'install' && optActionKey.value
      ? t('appstore.optActionExtra', { label: actionLabel(pkg, optActionKey.value) })
      : ''
  return t('appstore.optDialogTitle', {
    action: optModeName(optMode.value, pkg),
    name: pkg.title || pkg.name,
    extra,
  })
})

const optConfirmLabel = computed(() =>
  optMode.value === 'uninstall'
    ? t('appstore.confirmUninstallBtn')
    : optMode.value === 'upgrade'
      ? t('appstore.confirmUpgradeBtn')
      : t('appstore.confirmInstallBtn'),
)

function parseActionOptions(v: unknown): { items: AppOption[]; intro: string } {
  if (!v) return { items: [], intro: '' }
  if (Array.isArray(v)) return { items: v as AppOption[], intro: '' }
  const g = v as { items?: unknown; intro?: unknown }
  return {
    items: Array.isArray(g.items) ? (g.items as AppOption[]) : [],
    intro: typeof g.intro === 'string' ? g.intro : '',
  }
}

function actionOptions(
  pkg: AppPackage,
  actionKey?: string,
  strict = false,
): { items: AppOption[]; intro: string } {
  const o = pkg.options as unknown
  if (!o) return { items: [], intro: '' }
  if (Array.isArray(o)) return parseActionOptions(o)
  const m = o as Record<string, unknown>
  if (actionKey && m[actionKey] !== undefined) return parseActionOptions(m[actionKey])
  if (strict) return { items: [], intro: '' }
  if (m.install !== undefined) return parseActionOptions(m.install)
  const first = Object.keys(m)[0]
  return first ? parseActionOptions(m[first]) : { items: [], intro: '' }
}

function optionsFor(pkg: AppPackage, actionKey?: string, strict = false): AppOption[] {
  return actionOptions(pkg, actionKey, strict).items
}

function optionsIntro(pkg: AppPackage, actionKey?: string, strict = false): string {
  return actionOptions(pkg, actionKey, strict).intro
}

function hasOptionsDialog(pkg: AppPackage, actionKey?: string, strict = false): boolean {
  const { items, intro } = actionOptions(pkg, actionKey, strict)
  return items.length > 0 || !!intro
}

function optType(o: AppOption): AppOption['type'] {
  return o.type || 'string'
}

function optLabel(o: AppOption): string {
  return o.label || o.name
}

function choicesOf(o: AppOption): AppChoice[] {
  return (o.choices || []).map((c) =>
    typeof c === 'string' ? { label: c, value: c } : { label: c.label || c.value, value: c.value },
  )
}

function optDefault(o: AppOption): any {
  const d = o.default
  if (optType(o) === 'multiselect') {
    if (Array.isArray(d)) return d.map(String)
    if (typeof d === 'string' && d)
      return d
        .split(o.separator || ' ')
        .map((s) => s.trim())
        .filter(Boolean)
    return []
  }
  if (optType(o) === 'bool') return !!d
  if (optType(o) === 'number') return typeof d === 'number' ? d : undefined
  return typeof d === 'string' ? d : typeof d === 'number' ? String(d) : ''
}

function openOptionsDialog(pkg: AppPackage, actionKey: string | undefined, mode: OptMode) {
  const key = actionKey || (mode === 'install' ? undefined : mode)
  const strict = mode === 'uninstall'
  const defs = optionsFor(pkg, key, strict)
  const intro = optionsIntro(pkg, key, strict)
  if (!defs.length && !intro) return
  optMode.value = mode
  optPkg.value = pkg
  optActionKey.value = actionKey || ''
  optList.value = defs
  optIntro.value = intro
  const vals: Record<string, any> = {}
  for (const o of defs) vals[o.name] = optDefault(o)
  optValues.value = vals
  optFieldError.value = {}
  optSubmitting.value = false
  optDialogVisible.value = true
}

function collectOptions(): FormOptions | null {
  const out: FormOptions = {}
  const errors: Record<string, string> = {}
  for (const o of optList.value) {
    const type = optType(o)
    const name = o.name
    const v = optValues.value[name]
    if (type === 'multiselect') {
      const arr = (Array.isArray(v) ? v : []) as string[]
      if (o.required && !arr.length) errors[name] = t('appstore.errMultiRequired')
      else out[name] = arr.join(o.separator || ' ')
    } else if (type === 'bool') {
      out[name] = v ? 'true' : 'false'
    } else if (type === 'number') {
      if (o.required && (v === undefined || v === null || v === ''))
        errors[name] = t('appstore.errNumberRequired')
      else out[name] = v === undefined || v === null ? '' : String(v)
    } else if (type === 'select') {
      if (o.required && (v === undefined || v === null || v === ''))
        errors[name] = t('appstore.errSelectRequired')
      else out[name] = v === undefined || v === null ? '' : String(v)
    } else {
      const s = typeof v === 'string' ? v : v === undefined || v === null ? '' : String(v)
      if (o.required && !s.trim()) errors[name] = t('appstore.errTextRequired')
      else out[name] = s
    }
  }
  optFieldError.value = errors
  return Object.keys(errors).length ? null : out
}

async function submitOptions() {
  const opts = collectOptions()
  if (!opts) return
  optSubmitting.value = true
  try {
    const pkg = optPkg.value
    if (!pkg) return
    const ok =
      optMode.value === 'install'
        ? await doInstall(pkg, optActionKey.value || undefined, opts)
        : await (optMode.value === 'upgrade' ? doUpgrade(pkg, opts) : doUninstall(pkg, opts))
    if (ok) optDialogVisible.value = false
  } finally {
    optSubmitting.value = false
  }
}

// ── 安装 / 卸载 / 升级 ─────────────────────────────────────

async function handleInstall(pkg: AppPackage, actionKey?: string) {
  if (hasOptionsDialog(pkg, actionKey)) {
    openOptionsDialog(pkg, actionKey, 'install')
    return
  }
  await doInstall(pkg, actionKey)
}

async function doInstall(
  pkg: AppPackage,
  actionKey?: string,
  options?: FormOptions,
): Promise<boolean> {
  const ver = curVersion(pkg)
  const label = actionLabel(pkg, actionKey)
  const actName = pkg.installed ? t('appstore.actReinstall') : t('appstore.actInstall')
  try {
    const hint = !pkg.installed
      ? ''
      : pkg.allow_multiple_instances && pkg.versions && pkg.versions.length > 1
        ? t('appstore.hintMultiVersion')
        : t('appstore.hintOverwrite')
    await ElMessageBox.confirm(
      t('appstore.confirmBody', {
        action: actName,
        name: pkg.title || pkg.name,
        version: ver ? t('appstore.confirmVersionPart', { version: verLabel(pkg, ver) }) : '',
        actionPart: actionKey ? t('appstore.confirmActionPart', { label }) : '',
        hint,
      }),
      t('appstore.confirmTitle', { action: actName }),
      { type: 'info' },
    )
    const resp = await installPackage({
      pkg_path: pkg.pkg_path,
      source: pkg.source,
      repo_id: pkg.source === 'official' ? pkg.repo_id : undefined,
      version: ver,
      action: actionKey || undefined,
      options,
    })
    if (resp.data?.queued) {
      ElMessage.success(t('appstore.queued', { n: resp.data.position ?? 1 }))
      openQueue()
    } else {
      ElMessage.success(t('appstore.started', { action: actName }))
      logDrawerRef.value?.openDrawer(resp.data.run_id, `${actName} ${pkg.name}`)
    }
    trackRun(resp.data.run_id, `${pkg.title || pkg.name} ${actName}`)
    loadQueueCount()
    return true
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('appstore.startFailed', { action: actName }))
    return false
  }
}

async function handleUninstall(pkg: AppPackage) {
  if (hasOptionsDialog(pkg, 'uninstall', true)) {
    openOptionsDialog(pkg, undefined, 'uninstall')
    return
  }
  await doUninstall(pkg)
}

async function doUninstall(pkg: AppPackage, options?: FormOptions): Promise<boolean> {
  // 多实例包（多版本 PHP / 多站点 WordPress）不能只按包名卸：切到「已安装」按实例操作
  if ((pkg.installed_instances || []).length > 1) {
    ElMessage.warning(t('appstore.multiInstanceHint'))
    props.onMultiInstanceUninstall?.()
    return false
  }
  try {
    await ElMessageBox.confirm(
      t('appstore.uninstallConfirm', { name: pkg.title || pkg.name }),
      t('appstore.uninstallTitle'),
      { type: 'warning' },
    )
    const resp = await uninstallPackage({
      pkg_path: pkg.pkg_path,
      instance: (pkg.installed_instances || [])[0]?.instance,
      options,
    })
    ElMessage.success(t('appstore.uninstallStarted'))
    logDrawerRef.value?.openDrawer(resp.data.run_id, `${t('appstore.btnUninstall')} ${pkg.name}`)
    trackRun(resp.data.run_id, `${pkg.title || pkg.name} ${t('appstore.btnUninstall')}`)
    return true
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('appstore.uninstallFailed'))
    return false
  }
}

async function handleUpgrade(pkg: AppPackage) {
  if (hasOptionsDialog(pkg, 'upgrade')) {
    openOptionsDialog(pkg, undefined, 'upgrade')
    return
  }
  await doUpgrade(pkg)
}

async function doUpgrade(pkg: AppPackage, options?: FormOptions): Promise<boolean> {
  const ver = curVersion(pkg)
  if (pkg.installed && ver && ver === pkg.installed_version) {
    const redo = pkg.allow_multiple_instances
      ? t('appstore.btnInstallAgain')
      : t('appstore.btnReinstall')
    ElMessage.warning(t('appstore.upgradeSameVersion', { v: ver, redo }))
    return false
  }
  const instFam = familyOf(pkg, pkg.installed_version)
  const tgtFam = familyOf(pkg, ver)
  if (instFam && tgtFam && instFam !== tgtFam) {
    ElMessage.warning(
      t('appstore.crossFamily', {
        from: familyLabelOf(pkg, pkg.installed_version),
        to: familyLabelOf(pkg, ver),
      }),
    )
    return false
  }
  try {
    const fallbackHint = pkg.has_upgrade === false ? t('appstore.upgradeFallbackHint') : ''
    await ElMessageBox.confirm(
      t('appstore.upgradeConfirm', {
        name: pkg.title || pkg.name,
        to: verLabel(pkg, ver || pkg.version),
        from: verLabel(pkg, pkg.installed_version || ''),
        hint: fallbackHint,
      }),
      t('appstore.upgradeTitle'),
      { type: 'warning' },
    )
    const resp = await upgradePackage({
      pkg_path: pkg.pkg_path,
      source: pkg.source,
      repo_id: pkg.source === 'official' ? pkg.repo_id : undefined,
      version: ver || pkg.version,
      instance: (pkg.installed_instances || [])[0]?.instance,
      options,
    })
    if (resp.data?.queued) {
      ElMessage.success(t('appstore.queued', { n: resp.data.position ?? 1 }))
      openQueue()
    } else {
      ElMessage.success(t('appstore.upgradeStarted'))
      logDrawerRef.value?.openDrawer(resp.data.run_id, `${t('appstore.btnUpgrade')} ${pkg.name}`)
    }
    trackRun(resp.data.run_id, `${pkg.title || pkg.name} ${t('appstore.btnUpgrade')}`)
    loadQueueCount()
    return true
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('appstore.upgradeFailed'))
    return false
  }
}

// ── 任务队列入口 ────────────────────────────────────────────

const queueVisible = ref(false)
const queuePanelRef = ref<InstanceType<typeof TaskQueuePanel> | null>(null)
const activeCount = ref(0)

async function loadQueueCount() {
  try {
    const res = await getTasks({ kind: 'appstore', status: 'pending,running', page_size: 1 })
    activeCount.value = res.data?.total ?? 0
  } catch {
    // 角标只是提示
  }
}

function openQueue() {
  queueVisible.value = true
  queuePanelRef.value?.load()
  loadQueueCount()
}

function onQueueChanged() {
  loadQueueCount()
}

// ── 后台任务完成跟踪 ───────────────────────────────────────

const runPolls = new Map<string, number>()

function trackRun(runId: string, label: string) {
  if (runPolls.has(runId)) return
  const timer = window.setInterval(async () => {
    try {
      const resp = await getRuns({ page: 1, page_size: 50 })
      const item = (resp.data?.items || []).find((r: RunItem) => r.run_id === runId)
      if (!item || item.status === 'running' || item.status === 'pending') return
      window.clearInterval(timer)
      runPolls.delete(runId)
      loadQueueCount()
      if (item.status === 'success') {
        ElMessage.success(t('appstore.runSuccess', { label }))
      } else {
        const code = item.exit_code !== -1 ? t('appstore.exitCodePart', { code: item.exit_code }) : ''
        ElMessage.error(t('appstore.runFailed', { label, status: statusText(item.status), code }))
      }
      loadPackages()
    } catch {
      // 瞬时网络抖动，下一轮重试
    }
  }, 1500)
  runPolls.set(runId, timer)
}

onBeforeUnmount(() => {
  for (const t of runPolls.values()) window.clearInterval(t)
  runPolls.clear()
})

function statusText(s: string) {
  const map: Record<string, string> = {
    pending: t('appstore.statusPending'),
    running: t('appstore.statusRunning'),
    success: t('appstore.statusSuccess'),
    failed: t('appstore.statusFailed'),
    canceled: t('appstore.statusCanceled'),
  }
  return map[s] || s
}

const logDrawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)

onMounted(() => {
  loadPackages()
  loadQueueCount()
})

/** 供父页在「软件园」增删源后触发刷新 */
defineExpose({ reload: loadPackages })
</script>

<style scoped>
.appstore-packages {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.filter-bar__right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.pkg-table-wrap {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  overflow: hidden;
}

.opt-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
  margin-top: 2px;
  white-space: pre-line;
}

.opt-intro {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  font-size: 12.5px;
  color: var(--el-text-color-regular);
  line-height: 1.7;
  margin: 14px 0 2px;
  padding: 10px 12px;
  background: var(--el-fill-color-light);
  border-radius: 6px;
}
.opt-intro .el-icon {
  margin-top: 3px;
  flex: none;
}

.cell-name {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.cell-name__title {
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.cell-name__tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.cell-desc__title {
  font-size: 13px;
  color: var(--el-text-color-regular);
}
.cell-desc__body {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin: 2px 0;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.cell-desc__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.cell-installed {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  font-size: 12px;
}
.cell-installed__from {
  width: 100%;
  color: var(--el-text-color-secondary);
}

.cell-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  justify-content: flex-end;
}

.muted {
  color: var(--el-text-color-placeholder);
}

.ver-opt-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.ver-fam-tag {
  margin-left: 6px;
}
</style>
