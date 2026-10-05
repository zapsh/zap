<template>
  <el-card shadow="never" class="table-card">
    <div class="toolbar">
      <div class="toolbar-left">
        <el-button
          type="primary"
          :icon="Plus"
          :disabled="!allowed"
          :title="allowed ? '' : t('site.appNotAllowed')"
          @click="openWizard"
          >{{ t('site.appNew') }}</el-button
        >
        <span v-if="!allowed" class="apps-hint">{{ t('site.appNotAllowed') }}</span>
        <el-input
          v-model="keyword"
          :placeholder="t('site.appSearchPh')"
          clearable
          style="width: 240px"
          :prefix-icon="Search"
        />
        <span v-if="quotaText" class="apps-hint">{{ quotaText }}</span>
        <el-tag
          v-if="filterSiteName"
          closable
          size="small"
          type="info"
          @close="clearSiteFilter"
        >
          {{ t('site.appOnlySite') }}: {{ filterSiteName }}
        </el-tag>
      </div>
      <div class="toolbar-right">
        <el-button :icon="Refresh" :loading="loading" @click="load">{{ t('common.refresh') }}</el-button>
      </div>
    </div>

    <el-table v-loading="loading" :data="filtered" border stripe>
      <el-table-column :label="t('site.appColApp')" min-width="150">
        <template #default="{ row }">
          <div class="app-cell">
            <span class="app-name">{{ row.name }}</span>
            <span class="app-dir">{{ row.workdir }}</span>
            <span v-if="row.git_commit" class="app-ver">@{{ row.git_commit }}</span>
          </div>
        </template>
      </el-table-column>
      <el-table-column :label="t('site.appColSite')" min-width="130" show-overflow-tooltip>
        <template #default="{ row }">{{ row.site_name }}</template>
      </el-table-column>
      <el-table-column :label="t('site.appColType')" width="140">
        <template #default="{ row }">
          <span>{{ typeLabel(row.app_type) }}</span>
          <span v-if="row.runtime_version" class="app-ver">
            {{ row.runtime_version }}
          </span>
        </template>
      </el-table-column>
      <el-table-column :label="t('site.appColMount')" width="110" show-overflow-tooltip>
        <template #default="{ row }">
          <span v-if="row.mount_path" class="app-port">{{ mountLabel(row) }}</span>
          <span v-else>-</span>
        </template>
      </el-table-column>
      <el-table-column :label="t('site.appColPort')" width="90" align="center">
        <template #default="{ row }">
          <span v-if="row.port" class="app-port">{{ row.port }}</span>
          <span v-else>-</span>
        </template>
      </el-table-column>
      <el-table-column :label="t('site.appColState')" width="120" align="center">
        <template #default="{ row }">
          <span class="dot" :class="row.active ? 'dot-green' : 'dot-gray'" />
          {{ stateLabel(row) }}
        </template>
      </el-table-column>
      <el-table-column :label="t('site.appColDeployStatus')" width="110" align="center">
        <template #default="{ row }">
          <el-tag :type="deployStatusType(row)" size="small" effect="light">
            {{ deployStatusLabel(row) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('common.operation')" width="260" fixed="right">
        <template #default="{ row }">
          <el-button link type="success" :disabled="row.active" @click="act(row, 'start')">
            {{ t('site.appStart') }}
          </el-button>
          <el-button link type="warning" :disabled="!row.active" @click="act(row, 'stop')">
            {{ t('site.appStop') }}
          </el-button>
          <el-button link type="primary" @click="act(row, 'restart')">
            {{ t('site.appRestart') }}
          </el-button>
          <el-button link type="primary" @click="updateRow(row)">
            {{ t('site.appRedeploy') }}
          </el-button>
          <el-button link @click="openLog(row)">{{ t('site.appLog') }}</el-button>
          <el-button link @click="openEdit(row)">{{ t('common.edit') }}</el-button>
          <el-button link type="danger" @click="removeRow(row)">{{ t('common.delete') }}</el-button>
        </template>
      </el-table-column>
      <template #empty>
        <el-empty :image-size="80" :description="t('site.appEmpty')" />
      </template>
    </el-table>

    <!-- 部署向导 -->
    <AppDeployWizard
      v-model="wizardVisible"
      :site-id="wizardSiteId"
      :initial="wizardInitial"
      @done="load"
    />

    <!-- 日志 -->
    <el-drawer v-model="logVisible" :title="logTitle" size="60%">
      <pre class="log-box">{{ logText }}</pre>
    </el-drawer>

    <!-- 更新是后台长任务：打开任务日志抽屉实时查看进度 -->
    <AppStoreLogDrawer ref="deployLogDrawer" :simple="true" :stoppable="true" />
  </el-card>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useUserStore } from '@/stores/user'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Refresh, Search } from '@/icons'
import { http } from '@/utils/request'
import { getAppRuntimes, gitUpdateSiteApp, listAllApps, removeSiteApp, siteAppAction } from '@/api/site'
import type { AllAppItem } from '@/api/site'
import AppDeployWizard from './AppDeployWizard.vue'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'

const { t } = useI18n()
const route = useRoute()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

// 从站点行点「应用」进来时带 `?site=<id>`：列表只显示该站点的应用
const filterSiteId = ref(Number(route.query.site) || 0)

const list = ref<AllAppItem[]>([])
const loading = ref(false)
const keyword = ref('')
const wizardVisible = ref(false)
const wizardSiteId = ref(0)
const wizardInitial = ref<Record<string, unknown> | null>(null)
const logVisible = ref(false)
const logText = ref('')
const logTitle = ref('')
const deployLogDrawer = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
const quota = ref<{ port_min: number; port_max: number }>({ port_min: 0, port_max: 0 })
const allowed = ref(true)

const filterSiteName = computed(
  () => list.value.find((a) => a.site_id === filterSiteId.value)?.site_name || '',
)

const filtered = computed(() => {
  let rows = list.value
  if (filterSiteId.value) rows = rows.filter((a) => a.site_id === filterSiteId.value)
  const k = keyword.value.trim().toLowerCase()
  if (!k) return rows
  return rows.filter(
    (a) =>
      a.name.toLowerCase().includes(k) ||
      (a.site_name || '').toLowerCase().includes(k) ||
      (a.workdir || '').toLowerCase().includes(k),
  )
})

const quotaText = computed(() => {
  const { port_min: lo, port_max: hi } = quota.value
  if (lo > 0 && hi > 0) return t('site.appPortRangeHint', { min: lo, max: hi })
  return ''
})

function typeLabel(v: string) {
  if (v === 'python') return 'Python'
  if (v === 'nodejs') return 'Node.js'
  if (v === 'static') return t('site.appTypeStatic')
  return v
}

/** 挂载点按 nginx 写法显示：精确 `= /api`、优先前缀 `^~ /api` */
function mountLabel(r: AllAppItem) {
  if (!r.mount_path) return '-'
  if (r.mount_mode === 'exact') return `= ${r.mount_path}`
  if (r.mount_mode === 'prefer') return `^~ ${r.mount_path}`
  return r.mount_path
}

function stateLabel(r: AllAppItem) {
  if (r.active) return t('site.appRunning')
  if (r.running) return t('site.appDown')
  return t('site.appStopped')
}

/** 部署状态徽标文字（历史记录 deploy_status 为空按 success 处理） */
function deployStatusLabel(r: AllAppItem) {
  const s = r.deploy_status || 'success'
  if (s === 'deploying') return t('site.appDeploying')
  if (s === 'failed') return t('site.appDeployFailed')
  if (s === 'pending') return t('site.appDeployPending')
  return t('site.appDeploySuccess')
}

function deployStatusType(r: AllAppItem) {
  const s = r.deploy_status || 'success'
  if (s === 'deploying') return 'warning'
  if (s === 'failed') return 'danger'
  return 'success'
}

async function load() {
  loading.value = true
  try {
    const res = await listAllApps()
    list.value = ((res as any)?.data || []) as AllAppItem[]
  } catch {
    list.value = []
  } finally {
    loading.value = false
  }
}

async function loadQuota() {
  try {
    const res = await getAppRuntimes()
    const d = (res as any)?.data || {}
    quota.value = { port_min: d.port_min || 0, port_max: d.port_max || 0 }
    allowed.value = d.allowed !== false
  } catch {
    /* 忽略：拿不到就不提示端口段 */
  }
}

function openWizard() {
  wizardInitial.value = null
  // 从某个站点进来时，向导直接落在该站点上
  wizardSiteId.value = filterSiteId.value
  wizardVisible.value = true
}

function clearSiteFilter() {
  filterSiteId.value = 0
}

function openEdit(row: AllAppItem) {
  wizardInitial.value = { ...row } as unknown as Record<string, unknown>
  wizardSiteId.value = row.site_id
  wizardVisible.value = true
}

async function act(row: AllAppItem, action: string) {
  try {
    await siteAppAction(row.site_id, row.name, action)
    ElMessage.success(t('site.appActionDone'))
    load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('site.appActionFailed'))
  }
}

async function updateRow(row: AllAppItem) {
  try {
    const res = await gitUpdateSiteApp(row.site_id, row.name)
    const taskId = (res as any)?.data?.task_id
    if (taskId) {
      ElMessage.success(t('site.appGitUpdateStarted'))
      deployLogDrawer.value?.openDrawer(
        taskId,
        t('site.appGitUpdateTaskTitle', { name: row.name }),
      )
      return
    }
    ElMessage.success(t('site.appGitUpdateDone'))
    load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('site.appGitUpdateFailed'))
  }
}

async function openLog(row: AllAppItem) {
  // 有后台任务号时直接打开实时日志抽屉（部署 / 更新进度与报错都在这里）
  if (row.task_id) {
    deployLogDrawer.value?.openDrawer(
      row.task_id,
      t('site.appLogTaskTitle', { name: row.name }),
    )
    return
  }
  logTitle.value = `${row.site_name} / ${row.name}`
  try {
    const res = await http.get<{ code: number; data: { lines: string[] } }>('/site/app/log', {
      params: { site_id: row.site_id, name: row.name, lines: 300 },
    })
    logText.value = (res.data?.lines || []).join('\n') || t('site.appLogEmpty')
  } catch {
    logText.value = t('site.appLogEmpty')
  }
  logVisible.value = true
}

async function removeRow(row: AllAppItem) {
  try {
    await ElMessageBox.confirm(
      t('site.appRemoveConfirm', { name: row.name }),
      t('common.deleteConfirm'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  await removeSiteApp(row.site_id, row.name)
  ElMessage.success(t('common.deleteSuccess'))
  load()
}

onMounted(() => {
  load()
  loadQuota()
})
</script>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 14px;
}
.toolbar-left {
  display: flex;
  align-items: center;
  gap: 10px;
}
.apps-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.app-cell {
  display: flex;
  flex-direction: column;
  line-height: 1.35;
}
.app-name {
  font-weight: 600;
}
.app-dir,
.app-ver,
.app-port {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.app-ver {
  margin-left: 4px;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
  margin-right: 5px;
}
.dot-green {
  background: var(--el-color-success);
}
.dot-gray {
  background: var(--el-color-info);
}
.log-box {
  margin: 0;
  padding: 10px;
  height: 100%;
  overflow: auto;
  font-size: 12px;
  line-height: 1.6;
  background: var(--el-fill-color-light);
  border-radius: 6px;
}
</style>
