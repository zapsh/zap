<template>
  <div class="appstore-page">
    <!-- 多 Git 源管理卡片 -->
    <el-card shadow="never" class="repo-card">
      <div class="repo-head">
        <div class="repo-title-wrap">
          <el-icon :size="24" color="#409eff"><Goods /></el-icon>
          <div class="repo-detail">
            <div class="repo-title">{{ t('appstore.repoTitle') }}</div>
            <div class="repo-sub">{{ t('appstore.repoSub') }}</div>
          </div>
        </div>
        <div class="repo-head__actions">
          <!-- 任务队列入口：编译/安装是排队的，这里看得到排到哪了 -->
          <el-button size="small" :icon="List" @click="openQueue">
            {{ t('appstore.queueBtn') }}
            <el-tag v-if="activeCount" size="small" type="warning" effect="dark" round>
              {{ activeCount }}
            </el-tag>
          </el-button>
          <el-button type="primary" :disabled="!isAdmin" @click="showAddDialog = true">
            {{ t('appstore.addSource') }}
          </el-button>
        </div>
      </div>

      <div class="repo-list">
        <div v-for="r in repos" :key="r.id" class="repo-item">
          <div class="repo-item-left">
            <div class="repo-item-name">
              {{ r.name || r.id }}
              <el-tag v-if="r.builtin" size="small" type="primary" effect="plain">{{
                t('appstore.tagBuiltin')
              }}</el-tag>
              <el-tag v-if="!r.exists" size="small" type="danger" effect="plain">{{
                t('appstore.tagDirMissing')
              }}</el-tag>
            </div>
            <div class="repo-item-url">{{ r.url }}</div>
            <div class="repo-item-meta">
              <span>id: {{ r.id }}</span>
              <span v-if="r.version">{{ t('appstore.versionColon') }} {{ r.version }}</span>
              <span v-if="r.commit">commit: {{ r.commit.slice(0, 7) }}</span>
              <span>{{ t('appstore.updatedAtColon', { time: fmtTime(r.updated_at) }) }}</span>
            </div>
          </div>
          <div class="repo-item-actions">
            <el-button
              size="small"
              type="primary"
              plain
              :disabled="!isAdmin"
              @click="handleUpdateRepo(r)"
              >{{ t('appstore.update') }}</el-button
            >
            <el-button
              v-if="!r.builtin"
              size="small"
              type="danger"
              plain
              :disabled="!isAdmin"
              @click="handleRemoveRepo(r)"
              >{{ t('common.delete') }}</el-button
            >
          </div>
        </div>
        <el-empty v-if="repos.length === 0" :description="t('appstore.noRepo')" :image-size="60" />
      </div>
    </el-card>

    <!-- 页内导航：应用商店是单一菜单，已安装 / 我的站点应用都在本页切换 -->
    <div class="view-tabs">
      <el-radio-group v-model="activeTab" size="default">
        <el-radio-button value="store">{{ t('appstore.tabStore') }}</el-radio-button>
        <el-radio-button value="installed">{{ t('appstore.tabInstalled') }}</el-radio-button>
        <el-radio-button value="mine">{{ t('appstore.tabMine') }}</el-radio-button>
      </el-radio-group>
    </div>

    <!-- 已安装实例（按实例操作：启停 / 卸载） -->
    <InstalledPane
      v-if="activeTab === 'installed'"
      @task="onPaneTask"
      :key="'installed'"
    />
    <InstalledPane v-if="activeTab === 'mine'" scope="mine" @task="onPaneTask" :key="'mine'" />

    <!-- 分类 + 搜索 -->
    <div v-if="activeTab === 'store'" class="filter-bar">
      <el-radio-group v-model="activeCategory" size="small">
        <el-radio-button value="all">{{ t('appstore.catAll') }}</el-radio-button>
        <el-radio-button value="infra">{{ t('appstore.catInfra') }}</el-radio-button>
        <el-radio-button value="application">{{ t('appstore.catApplication') }}</el-radio-button>
        <el-radio-button value="webapps">{{ t('appstore.catWebapps') }}</el-radio-button>
        <el-radio-button value="database">{{ t('appstore.catDatabase') }}</el-radio-button>
        <el-radio-button value="library">{{ t('appstore.catLibrary') }}</el-radio-button>
      </el-radio-group>
      <el-input
        v-model="keyword"
        :placeholder="t('appstore.searchPlaceholder')"
        clearable
        style="width: 240px"
        size="small"
      >
        <template #prefix
          ><el-icon><Search /></el-icon
        ></template>
      </el-input>
    </div>

    <!-- 包列表 -->
    <div v-if="activeTab === 'store'" class="pkg-grid" v-loading="loading">
      <el-card v-for="pkg in filteredPackages" :key="pkg.pkg_path" shadow="hover" class="pkg-card">
        <div class="pkg-head">
          <div class="pkg-name">
            {{ pkg.name }}
            <el-tag v-if="pkg.installed" size="small" type="success" effect="light">{{
              (pkg.installed_instances || []).length > 1
                ? `${t('appstore.tagInstalled')} × ${(pkg.installed_instances || []).length}`
                : t('appstore.tagInstalled')
            }}</el-tag>
            <el-tag v-else size="small" type="info" effect="plain">{{
              t('appstore.tagNotInstalled')
            }}</el-tag>
            <el-tag
              v-if="pkg.allow_multiple_instances"
              size="small"
              type="primary"
              effect="plain"
              >{{ t('appstore.tagMultiVersion') }}</el-tag
            >
          </div>
          <div class="pkg-tags">
            <el-tag v-if="pkg.source === 'custom'" size="small" type="warning" effect="light">{{
              t('appstore.tagCustom')
            }}</el-tag>
            <el-tag v-else-if="pkg.repo_id" size="small" type="info" effect="plain">{{
              pkg.repo_id
            }}</el-tag>
          </div>
        </div>
        <div class="pkg-title">{{ pkg.title || pkg.name }}</div>
        <div class="pkg-desc">{{ pkg.description || t('appstore.noDescription') }}</div>
        <div class="pkg-meta">
          <span v-if="pkg.versions && pkg.versions.length > 1" class="pkg-ver-sel">
            {{ t('appstore.versionColon') }}
            <el-select
              size="small"
              :style="{ width: versionGroups(pkg).length ? '210px' : '130px' }"
              :model-value="selVersion[pkg.pkg_path] || pkg.version"
              @change="onSelVersion(pkg, $event)"
            >
              <template v-if="versionGroups(pkg).length">
                <el-option-group v-for="g in versionGroups(pkg)" :key="g.family" :label="g.label">
                  <el-option v-for="ver in g.versions" :key="ver" :label="ver" :value="ver">
                    <div class="ver-opt-row">
                      <span>{{ ver }}</span>
                      <el-tag :type="familyTagType(g.family)" size="small" effect="plain">
                        {{ g.short }}
                      </el-tag>
                    </div>
                  </el-option>
                </el-option-group>
              </template>
              <el-option v-else v-for="ver in pkg.versions" :key="ver" :label="ver" :value="ver" />
            </el-select>
            <el-tag
              v-if="familyOf(pkg, curVersion(pkg))"
              class="ver-fam-tag"
              :type="familyTagType(familyOf(pkg, curVersion(pkg)))"
              size="small"
              effect="light"
              >{{ familyLabelOf(pkg, curVersion(pkg)) }}</el-tag
            >
          </span>
          <span v-else
            >{{ t('appstore.versionColon') }} <b>{{ pkg.version || '-' }}</b></span
          >
          <span v-if="depList(pkg).length" class="pkg-deps">{{
            t('appstore.depColon', { list: depList(pkg).join(t('appstore.depSep')) })
          }}</span>
          <span v-if="pkg.default_port" class="pkg-port">{{
            t('appstore.portColon', { port: pkg.default_port })
          }}</span>
        </div>
        <div v-if="pkg.installed" class="pkg-installed-meta">
          {{ t('appstore.installedVersionColon') }}
          <b>{{ pkg.installed_version || '-' }}</b>
          <el-tag
            v-if="familyOf(pkg, pkg.installed_version)"
            size="small"
            :type="familyTagType(familyOf(pkg, pkg.installed_version))"
            effect="plain"
            >{{ familyLabelOf(pkg, pkg.installed_version) }}</el-tag
          >
          <span v-if="pkg.upgraded_from">{{
            t('appstore.upgradedFrom', { from: pkg.upgraded_from })
          }}</span>
        </div>
        <div class="pkg-actions">
          <template v-if="!pkg.installed">
            <template v-if="actionEntries(pkg).length">
              <el-button
                v-for="[key, label] in actionEntries(pkg)"
                :key="key"
                size="small"
                type="primary"
                :disabled="!canOperatePkg(pkg)"
                @click="handleInstall(pkg, key)"
                >{{ label }}</el-button
              >
            </template>
            <el-button
              v-else
              size="small"
              type="primary"
              :disabled="!canOperatePkg(pkg)"
              @click="handleInstall(pkg)"
              >{{ t('appstore.btnInstall') }}</el-button
            >
          </template>
          <template v-else>
            <el-button
              v-if="!pkg.allow_multiple_instances && !actionEntries(pkg).length"
              size="small"
              type="primary"
              plain
              :disabled="!canOperatePkg(pkg)"
              @click="handleInstall(pkg)"
              >{{ t('appstore.btnReinstall') }}</el-button
            >
            <el-button
              v-if="pkg.allow_multiple_instances && !actionEntries(pkg).length"
              size="small"
              type="primary"
              plain
              :disabled="!canOperatePkg(pkg)"
              @click="handleInstall(pkg)"
              >{{ t('appstore.btnInstallAgain') }}</el-button
            >
            <el-button
              v-for="[key, label] in actionEntries(pkg)"
              :key="key"
              size="small"
              type="success"
              plain
              :disabled="!canOperatePkg(pkg)"
              @click="handleInstall(pkg, key)"
              >{{ label }}</el-button
            >
            <el-button
              v-if="!pkg.allow_multiple_instances"
              size="small"
              type="warning"
              plain
              :disabled="!canOperatePkg(pkg)"
              @click="handleUpgrade(pkg)"
              >{{ t('appstore.btnUpgrade') }}</el-button
            >
            <el-button
              size="small"
              type="danger"
              plain
              :disabled="!canOperatePkg(pkg)"
              @click="handleUninstall(pkg)"
              >{{ t('appstore.btnUninstall') }}</el-button
            >
          </template>
        </div>
      </el-card>
    </div>
    <el-empty
      v-if="!loading && filteredPackages.length === 0"
      :description="t('appstore.noPackages')"
    />

    <!-- 运行记录 -->
    <el-card shadow="never" class="runs-card">
      <template #header>
        <div class="runs-header">
          <span>{{ t('appstore.runsTitle') }}</span>
          <el-button size="small" text @click="loadRuns">{{ t('appstore.refresh') }}</el-button>
        </div>
      </template>
      <el-table :data="runs" size="small" v-loading="runsLoading">
        <el-table-column prop="action" :label="t('appstore.colAction')" width="130" />
        <el-table-column
          prop="pkg"
          :label="t('appstore.colTarget')"
          min-width="180"
          show-overflow-tooltip
        />
        <el-table-column prop="username" :label="t('appstore.colInitiator')" width="110" />
        <el-table-column :label="t('appstore.colStatus')" width="100">
          <template #default="{ row }">
            <el-tag :type="statusType(row.status)" size="small">{{
              statusText(row.status)
            }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="exit_code" :label="t('appstore.colExitCode')" width="90" />
        <el-table-column :label="t('appstore.colStartedAt')" width="170">
          <template #default="{ row }">{{ fmtTime(row.started_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="170" fixed="right">
          <template #default="{ row }">
            <el-button size="small" text type="primary" @click="viewRunLog(row)">{{
              t('appstore.viewLog')
            }}</el-button>
            <el-button
              v-if="isAdmin && row.status === 'failed'"
              size="small"
              text
              type="danger"
              :loading="retryId === row.run_id"
              @click="handleRetryRun(row)"
            >
              {{ t('appstore.retry') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <el-pagination
        v-model:current-page="runPage"
        :page-size="20"
        :total="runTotal"
        layout="prev, pager, next, total"
        small
        style="margin-top: 10px; justify-content: flex-end"
        @current-change="loadRuns"
      />
    </el-card>

    <!-- 添加源对话框 -->
    <el-dialog v-model="showAddDialog" :title="t('appstore.addRepoTitle')" width="520px">
      <el-form label-width="80px" @submit.prevent>
        <el-form-item :label="t('appstore.nameLabel')" required>
          <el-input
            v-model="addForm.name"
            :placeholder="t('appstore.namePlaceholder')"
            maxlength="32"
          />
        </el-form-item>
        <el-form-item :label="t('appstore.gitUrlLabel')" required>
          <el-input v-model="addForm.url" placeholder="https://github.com/org/repo.git" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showAddDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="adding" @click="handleAddRepo">{{
          t('appstore.addAndPull')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 任务队列抽屉：应用商店自己的任务（安装 / 升级 / 脚本 / 仓库同步） -->
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
import { Goods, List, Search, InfoFilled } from '@/icons'
import { useUserStore } from '@/stores/user'
import TaskQueuePanel from '@/components/TaskQueuePanel.vue'
import { getTasks } from '@/api/task'
import {
  getRepos,
  addRepo,
  removeRepo,
  updateRepo,
  getPackages,
  installPackage,
  uninstallPackage,
  upgradePackage,
  getRuns,
  retryRun,
  getRunFiles,
  type AppPackage,
  type AppOption,
  type AppChoice,
  type FormOptions,
  type RepoSource,
  type RunItem,
  type VersionMeta,
} from '@/api/appstore'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import InstalledPane from '@/views/appstore/installed.vue'

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

/** 当前页签：store=应用商店；installed=已安装实例；mine=我的站点应用 */
const activeTab = ref<'store' | 'installed' | 'mine'>('store')

/** 已安装面板提交任务后，在这里打开日志抽屉（与商店页共用一个抽屉） */
function onPaneTask(runId: string, title: string) {
  logDrawerRef.value?.openDrawer(runId, title)
}

/**
 * 当前用户能否浏览该包（应用商店列表可见）：
 * admin 恒可见；app.yaml roles 空/未声明 = 仅 admin 可见；
 * 声明了 roles = admin + 命中白名单的角色可见。
 */
function canViewPkg(pkg: AppPackage): boolean {
  if (isAdmin.value) return true
  const rs = pkg.roles
  if (!rs || rs.length === 0) return false
  return rs.some((r) => userStore.roles.includes(r))
}

/**
 * 当前用户能否对该包执行安装/升级/卸载：
 * custom 包仅管理员；官方包按 roles 判定（可见即命中白名单，可操作）。
 */
function canOperatePkg(pkg: AppPackage): boolean {
  if (pkg.source === 'custom') return isAdmin.value
  return canViewPkg(pkg)
}

// ── Git 源（多源）───────────────────────────────────────────

const repos = ref<RepoSource[]>([])
const updatingId = ref('')
const adding = ref(false)
const showAddDialog = ref(false)
const addForm = ref({ name: '', url: '' })

async function loadRepos() {
  try {
    const resp = await getRepos()
    repos.value = resp.data.repos || []
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.loadReposFailed'))
  }
}

async function handleAddRepo() {
  const name = addForm.value.name.trim()
  const url = addForm.value.url.trim()
  if (!name) {
    ElMessage.warning(t('appstore.nameRequired'))
    return
  }
  if (!url) {
    ElMessage.warning(t('appstore.urlRequired'))
    return
  }
  adding.value = true
  try {
    const resp = await addRepo({ name, url })
    ElMessage.success(t('appstore.addStarted'))
    showAddDialog.value = false
    addForm.value = { name: '', url: '' }
    logDrawerRef.value?.openDrawer(resp.data.run_id, t('appstore.addRepoLogTitle'))
    setTimeout(() => {
      loadRepos()
      loadPackages()
    }, 3000)
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.addFailed'))
  } finally {
    adding.value = false
  }
}

async function handleUpdateRepo(r: RepoSource) {
  updatingId.value = r.id
  try {
    const resp = await updateRepo({ id: r.id })
    ElMessage.success(t('appstore.updateStarted'))
    logDrawerRef.value?.openDrawer(
      resp.data.run_id,
      t('appstore.updateLogTitle', { name: r.name || r.id }),
    )
    setTimeout(() => {
      loadRepos()
      loadPackages()
    }, 3000)
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.updateFailed'))
  } finally {
    updatingId.value = ''
  }
}

async function handleRemoveRepo(r: RepoSource) {
  try {
    await ElMessageBox.confirm(
      t('appstore.removeConfirm', { name: r.name || r.id }),
      t('appstore.removeTitle'),
      { type: 'warning' },
    )
    const resp = await removeRepo({ id: r.id })
    ElMessage.success(resp.message || t('appstore.removed'))
    loadRepos()
    loadPackages()
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('appstore.removeFailed'))
  }
}

// ── 包列表 ──────────────────────────────────────────────────

const packages = ref<AppPackage[]>([])
const loading = ref(false)
const activeCategory = ref('all')
const keyword = ref('')

const filteredPackages = computed(() => {
  // 按包角色白名单过滤：admin 恒可见；roles 空/未声明 = 仅 admin 可见；命中角色也可见
  let list = packages.value.filter(canViewPkg)
  if (activeCategory.value !== 'all') {
    list = list.filter((p) => p.category === activeCategory.value)
  }
  const kw = keyword.value.trim().toLowerCase()
  if (kw) {
    list = list.filter(
      (p) =>
        p.name.toLowerCase().includes(kw) ||
        p.title.toLowerCase().includes(kw) ||
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

/** 每张卡片当前选择的安装/升级目标版本（app.yaml version 数组中的一项） */
const selVersion = ref<Record<string, string>>({})

function onSelVersion(pkg: AppPackage, v: string) {
  selVersion.value[pkg.pkg_path] = v
}

/** 卡片当前生效的版本：优先用户下拉选择，其次包默认版本 */
function curVersion(pkg: AppPackage): string {
  return selVersion.value[pkg.pkg_path] || pkg.version || ''
}

// ── 版本家族元数据（app.yaml version_meta，任意包可用）──────────────────

/** version_meta 中某版本的元数据；未声明 / 版本不在其中时返回 undefined */
function metaOf(pkg: AppPackage, ver?: string | null): VersionMeta | undefined {
  if (!ver || !pkg.version_meta) return undefined
  return pkg.version_meta[ver]
}

/** 某版本的家族标识（family 字段）；未声明 / 为空时返回空串（= 无家族，不分组） */
function familyOf(pkg: AppPackage, ver?: string | null): string {
  const f = metaOf(pkg, ver)?.family
  return typeof f === 'string' ? f.trim() : ''
}

/** 家族展示名（无内置映射，全部来自 version_meta）：label 缺省回退 family 原文 */
function familyLabelOf(pkg: AppPackage, ver?: string | null): string {
  const f = familyOf(pkg, ver)
  if (!f) return ''
  return metaOf(pkg, ver)?.label || f
}

/** 家族在标签 / 选中态里的配色：已知家族专属色，其余默认主色 */
const FAMILY_TAG_TYPE: Record<string, 'primary' | 'success' | 'warning' | 'info'> = {
  mysql: 'primary',
  mariadb: 'success',
}
function familyTagType(f: string): 'primary' | 'success' | 'warning' | 'info' {
  return FAMILY_TAG_TYPE[f] || 'primary'
}

/** 确认文案用：家族在则 “MySQL 8.0.46”，否则回退 “v8.0.46” */
function verLabel(pkg: AppPackage, ver: string): string {
  const f = familyOf(pkg, ver)
  return f ? `${familyLabelOf(pkg, ver)} ${ver}` : `v${ver}`
}

/**
 * 合并入口（多个家族共享一个下拉）时按家族分组；保持 versions 原顺序。
 * 元数据缺失 / 只有一个家族时返回空（回退普通扁平下拉）。
 */
function versionGroups(
  pkg: AppPackage,
): Array<{ family: string; label: string; short: string; versions: string[] }> {
  if (!pkg.version_meta) return []
  const groups: Array<{ family: string; label: string; short: string; versions: string[] }> = []
  for (const ver of pkg.versions || []) {
    const f = familyOf(pkg, ver)
    if (!f) return [] // 某个版本缺家族元数据 → 整组回退扁平，避免误标
    const meta = metaOf(pkg, ver)
    const last = groups[groups.length - 1]
    if (last && last.family === f) last.versions.push(ver)
    else
      groups.push({
        family: f,
        // 组名 = meta.group，缺省回退短名/家族原文
        label: meta?.group || meta?.label || f,
        // 选项内小标签 = meta.label（短名），缺省回退家族原文
        short: meta?.label || f,
        versions: [ver],
      })
  }
  return groups.length > 1 ? groups : []
}

/** app.yaml actions 的动作键 -> 按钮文案（键值与文案均去空格兜底） */
function actionEntries(pkg: AppPackage): Array<[string, string]> {
  const a = pkg.actions
  if (!a) return []
  return Object.entries(a).map(([k, label]) => [k, (label || '').trim() || k])
}

function actionLabel(pkg: AppPackage, key?: string): string {
  if (!key) return ''
  return actionEntries(pkg).find(([k]) => k === key)?.[1] || key
}

/** 依赖展示：映射（openssl 1.1.1w）或旧式数组 */
function depList(pkg: AppPackage): string[] {
  const d = pkg.dependencies as unknown
  if (!d) return pkg.deps || []
  if (Array.isArray(d)) return d as string[]
  return Object.entries(d as Record<string, string>).map(([n, v]) => (v ? `${n} ${v}` : n))
}

// ── 安装/升级选项（app.yaml options 动态表单）───────────────

type OptMode = 'install' | 'upgrade' | 'uninstall'

const optDialogVisible = ref(false)
const optMode = ref<OptMode>('install')
const optPkg = ref<AppPackage | null>(null)
const optActionKey = ref('')
const optList = ref<AppOption[]>([])
/** 表单值:type=string/select 存 string;number 存 number;bool 存 boolean;multiselect 存 string[] */
const optValues = ref<Record<string, any>>({})
const optFieldError = ref<Record<string, string>>({})
const optSubmitting = ref(false)
/** 整组介绍（app.yaml options.<动作>.intro，展示在选项表单最下方） */
const optIntro = ref('')

/** 对话框动作中文名（已安装时：多版本=再次安装，单版本=重装） */
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

/** 解析单个动作键下的选项定义：兼容「选项数组」与「{ items, intro } 对象」两种写法 */
function parseActionOptions(v: unknown): { items: AppOption[]; intro: string } {
  if (!v) return { items: [], intro: '' }
  if (Array.isArray(v)) return { items: v as AppOption[], intro: '' }
  const g = v as { items?: unknown; intro?: unknown }
  return {
    items: Array.isArray(g.items) ? (g.items as AppOption[]) : [],
    intro: typeof g.intro === 'string' ? g.intro : '',
  }
}

/** 命中当前动作键的选项定义（含整组介绍）；顶层数组作用于所有动作。
 * strict=true 时不回退 install 键（卸载场景：未声明 options.uninstall 即视为无卸载选项） */
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

/** 包在该动作下需要填写的选项（缺省回退到 install 键;顶层数组作用于所有动作） */
function optionsFor(pkg: AppPackage, actionKey?: string, strict = false): AppOption[] {
  return actionOptions(pkg, actionKey, strict).items
}

/** 该组选项的整体介绍（app.yaml options.<动作>.intro，展示在选项表单最下方） */
function optionsIntro(pkg: AppPackage, actionKey?: string, strict = false): string {
  return actionOptions(pkg, actionKey, strict).intro
}

/** 该动作是否需要弹选项对话框：有可填选项，或声明了整组介绍（仅介绍也可作为动作说明弹出） */
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
  // 未指定动作键时按模式取键：upgrade/uninstall 用同名键（缺省回退 options.install），
  // install 保持空键（由 actionOptions 回退到 options.install）。
  // 注意：不能把 undefined 传给卸载——strict 模式下会被判为「无卸载选项」而直接返回。
  const key = actionKey || (mode === 'install' ? undefined : mode)
  // 卸载不默认复用安装选项：仅在声明了 options.uninstall（或顶层数组）时弹出
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

/** 校验并归一化为提交载荷（字符串化）;失败返回 null 并标出错误字段 */
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
  // 该动作定义了选项或整组介绍 → 先弹窗（收集选项 / 展示说明）再安装
  if (hasOptionsDialog(pkg, actionKey)) {
    openOptionsDialog(pkg, actionKey, 'install')
    return
  }
  await doInstall(pkg, actionKey)
}

/** 真正发起安装;成功返回 true（关闭选项对话框） */
async function doInstall(
  pkg: AppPackage,
  actionKey?: string,
  options?: FormOptions,
): Promise<boolean> {
  const ver = curVersion(pkg)
  const label = actionLabel(pkg, actionKey)
  const actName = pkg.installed ? t('appstore.actReinstall') : t('appstore.actInstall')
  try {
    // 已安装时区分提示：多版本强调选版本；单版本「重装」会覆盖当前安装
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
    // 编译是排队的：已有编译在跑时不弹日志（还没有输出），改为打开队列看位次
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
  // 包声明了卸载选项/介绍（app.yaml options.uninstall）→ 先弹窗再卸载
  if (hasOptionsDialog(pkg, 'uninstall', true)) {
    openOptionsDialog(pkg, undefined, 'uninstall')
    return
  }
  await doUninstall(pkg)
}

/** 真正发起卸载;成功返回 true（关闭选项对话框） */
async function doUninstall(pkg: AppPackage, options?: FormOptions): Promise<boolean> {
  // 多实例包（多版本 PHP / 多站点 WordPress）不能只按包名卸：切到「已安装」按实例操作
  if ((pkg.installed_instances || []).length > 1) {
    ElMessage.warning(t('appstore.multiInstanceHint'))
    activeTab.value = 'installed'
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
  // 该包定义了升级选项/介绍（或缺省复用安装定义）→ 先弹窗再升级
  if (hasOptionsDialog(pkg, 'upgrade')) {
    openOptionsDialog(pkg, undefined, 'upgrade')
    return
  }
  await doUpgrade(pkg)
}

/** 真正发起升级;成功返回 true */
async function doUpgrade(pkg: AppPackage, options?: FormOptions): Promise<boolean> {
  const ver = curVersion(pkg)
  // 升级目标与当前已装版本一致时直接拦截(后端亦拒绝),避免无意义的重复执行
  if (pkg.installed && ver && ver === pkg.installed_version) {
    // 多版本包提供「再次安装」，单版本包提供「重装」
    const redo = pkg.allow_multiple_instances
      ? t('appstore.btnInstallAgain')
      : t('appstore.btnReinstall')
    ElMessage.warning(t('appstore.upgradeSameVersion', { v: ver, redo }))
    return false
  }
  // 合并入口（version_meta 声明家族）：跨家族不能升级（mariadb → mysql 等），须先卸载再装目标版本
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
    // 未提供 upgrade.sh 时后端按「uninstall → install」兜底执行，需提示数据备份
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
    // 升级同样是编译任务，可能要排队
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
/** 未结束（排队中 + 进行中）的应用商店任务数，用作入口角标 */
const activeCount = ref(0)

async function loadQueueCount() {
  try {
    const res = await getTasks({ kind: 'appstore', status: 'pending,running', page_size: 1 })
    activeCount.value = res.data?.total ?? 0
  } catch {
    // 角标只是提示，失败不打扰主流程
  }
}

function openQueue() {
  queueVisible.value = true
  queuePanelRef.value?.load()
  loadQueueCount()
}

/** 队列里发生写操作（重跑 / 改脚本后重跑）→ 角标与运行记录一起刷新 */
function onQueueChanged() {
  loadQueueCount()
  loadRuns()
}

// ── 后台任务完成跟踪:终态提示 + 自动刷新列表 ───────────────
const runPolls = new Map<string, number>()

/** 轮询 run 直到终态:成功/失败给出提示,并刷新应用商店包列表(安装状态等) */
function trackRun(runId: string, label: string) {
  if (runPolls.has(runId)) return
  const timer = window.setInterval(async () => {
    try {
      const resp = await getRuns({ page: 1, page_size: 50 })
      const item = (resp.data?.items || []).find((r: RunItem) => r.run_id === runId)
      // pending = 排队等编译槽位，running = 仍在执行，两者都不是终态
      if (!item || item.status === 'running' || item.status === 'pending') return
      window.clearInterval(timer)
      runPolls.delete(runId)
      loadQueueCount()
      if (item.status === 'success') {
        ElMessage.success(t('appstore.runSuccess', { label }))
      } else {
        const code =
          item.exit_code !== -1 ? t('appstore.exitCodePart', { code: item.exit_code }) : ''
        ElMessage.error(t('appstore.runFailed', { label, status: statusText(item.status), code }))
      }
      loadPackages()
    } catch {
      // 瞬时网络 / 后端抖动,下一轮重试
    }
  }, 1500)
  runPolls.set(runId, timer)
}

onBeforeUnmount(() => {
  for (const t of runPolls.values()) window.clearInterval(t)
  runPolls.clear()
})

// ── 运行记录 ────────────────────────────────────────────────

const runs = ref<RunItem[]>([])
const runsLoading = ref(false)
const runPage = ref(1)
const runTotal = ref(0)
const retryId = ref('')

async function loadRuns() {
  runsLoading.value = true
  try {
    const resp = await getRuns({ page: runPage.value, page_size: 20 })
    runs.value = resp.data.items || []
    runTotal.value = resp.data.total || 0
  } catch {
    // ignore
  } finally {
    runsLoading.value = false
  }
}

function viewRunLog(row: RunItem) {
  logDrawerRef.value?.openDrawer(row.run_id, `${row.action} ${row.pkg}`)
}

/** 失败运行的快捷重跑：先探测快照是否存在，再复用其内容以新运行记录执行 */
async function handleRetryRun(row: RunItem) {
  if (retryId.value) return
  try {
    const files = await getRunFiles(row.run_id)
    if (!(files.data?.files || []).length) {
      ElMessage.warning(t('appstore.retryNoFiles'))
      return
    }
    await ElMessageBox.confirm(
      t('appstore.retryConfirm', { action: row.action }),
      t('appstore.retryTitle'),
      { type: 'warning' },
    )
  } catch (e: any) {
    if (e !== 'cancel' && e?.message) ElMessage.warning(e.message)
    return
  }
  retryId.value = row.run_id
  try {
    const resp = await retryRun(row.run_id)
    ElMessage.success(t('appstore.retryStarted'))
    logDrawerRef.value?.openDrawer(
      resp.data.run_id,
      `${row.action} ${row.pkg}${t('appstore.retrySuffix')}`,
    )
    setTimeout(loadRuns, 1500)
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('appstore.retryFailed'))
  } finally {
    retryId.value = ''
  }
}

function statusType(s: string): 'info' | 'success' | 'danger' | 'warning' {
  if (s === 'success') return 'success'
  if (s === 'failed') return 'danger'
  if (s === 'running') return 'warning'
  if (s === 'pending') return 'info'
  return 'info'
}

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

// ── 工具 ────────────────────────────────────────────────────

function fmtTime(ts: number | null | undefined): string {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

const logDrawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)

onMounted(() => {
  loadRepos()
  loadPackages()
  loadRuns()
  loadQueueCount()
})
</script>

<style scoped>
.appstore-page {
  display: flex;
  flex-direction: column;
  gap: 14px;
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

.repo-card {
  margin-bottom: 4px;
}

.repo-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
}

.repo-title-wrap {
  display: flex;
  align-items: center;
  gap: 12px;
}

.repo-head__actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.repo-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}

.repo-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-top: 4px;
}

.repo-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.repo-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  background: var(--el-bg-color-page);
}

.repo-item-left {
  min-width: 0;
}

.repo-item-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  display: flex;
  align-items: center;
  gap: 6px;
}

.repo-item-url {
  font-size: 12px;
  color: var(--el-text-color-regular);
  margin-top: 4px;
  word-break: break-all;
}

.repo-item-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.repo-item-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

/* 页内导航（应用商店单一菜单，已安装 / 我的站点应用在这里切） */
.view-tabs {
  margin: 12px 0 2px;
}

.filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.pkg-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 12px;
  min-height: 120px;
}

.pkg-card {
  border-radius: 8px;
}

.pkg-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}

.pkg-name {
  font-size: 15px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  display: flex;
  align-items: center;
  gap: 6px;
}

.pkg-tags {
  display: flex;
  gap: 4px;
}

.pkg-title {
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.pkg-desc {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin: 6px 0;
  min-height: 32px;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.pkg-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 6px;
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

.pkg-installed-meta {
  font-size: 12px;
  color: #67c23a;
  margin-bottom: 8px;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.pkg-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  border-top: 1px solid var(--el-border-color-lighter);
  padding-top: 10px;
}

.runs-card {
  margin-top: 4px;
}

.runs-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}
</style>
