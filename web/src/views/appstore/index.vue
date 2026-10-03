<template>
  <div class="appstore-page">
    <!-- 页内导航：应用商店是单一菜单，已安装 / 我的站点应用都在本页切换 -->
    <div class="view-tabs">
      <el-radio-group v-model="activeTab" size="default">
        <el-radio-button v-for="tab in tabs" :key="tab.value" :value="tab.value">{{ tab.label }}</el-radio-button>
      </el-radio-group>
      <div class="view-tabs__actions">
        <!-- 任务队列入口：编译/安装是排队的，这里看得到排到哪了 -->
        <el-button size="small" :icon="List" @click="openQueue">
          {{ t('appstore.queueBtn') }}
          <el-tag v-if="activeCount" size="small" type="warning" effect="dark" round>
            {{ activeCount }}
          </el-tag>
        </el-button>
        <el-button v-if="isAdmin" size="small" :icon="Goods" @click="repoDrawerRef?.open('list')">
          {{ t('appstore.manageRepos') }}
        </el-button>
        <el-button
          v-if="isAdmin"
          size="small"
          type="primary"
          :icon="Plus"
          @click="repoDrawerRef?.open('add')"
        >
          {{ t('appstore.addSource') }}
        </el-button>
      </div>
    </div>

    <!-- 软件园（Git 源）管理抽屉：列表 / 更新 / 删除 / 增加源 -->
    <RepoManageDrawer ref="repoDrawerRef" @changed="onRepoChanged" @log="onRepoLog" />

    <!-- 已安装实例（按实例操作：启停 / 卸载） -->
    <InstalledPane
      v-if="activeTab === 'installed'"
      @task="onPaneTask"
      :key="'installed'"
    />
    <InstalledPane v-if="activeTab === 'mine'" scope="mine" @task="onPaneTask" :key="'mine'" />
    <PluginsPane v-if="activeTab === 'plugins'" :key="'plugins'" />

    <!-- 应用商店（全部分类；安装/升级/卸载在组件内完成） -->
    <AppStorePackages
      ref="packagesRef"
      v-if="activeTab === 'store'"
      :on-multi-instance-uninstall="() => (activeTab = 'installed')"
    />

    <!-- 任务队列抽屉：应用商店自己的任务（安装 / 升级 / 脚本 / 仓库同步） -->
    <el-drawer v-model="queueVisible" :title="t('appstore.queueTitle')" size="72%" destroy-on-close>
      <TaskQueuePanel ref="queuePanelRef" kind="appstore" @changed="onQueueChanged" />
    </el-drawer>

    <!-- 日志抽屉（已安装 / 我的 / 插件 面板的任务共用） -->
    <AppStoreLogDrawer ref="logDrawerRef" />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { Goods, List, Plus } from '@/icons'
import { useUserStore } from '@/stores/user'
import TaskQueuePanel from '@/components/TaskQueuePanel.vue'
import { getTasks } from '@/api/task'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import InstalledPane from '@/views/appstore/installed.vue'
import PluginsPane from '@/views/appstore/plugins.vue'
import RepoManageDrawer from '@/views/appstore/RepoManageDrawer.vue'
import AppStorePackages from '@/views/appstore/AppStorePackages.vue'

const packagesRef = ref<InstanceType<typeof AppStorePackages> | null>(null)

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

/** 页签：站点应用相关（已安装 / 我的）对全员开放；插件管理全员可见
 *  —— 插件只装在系统目录、仅管理员可安装，普通用户只能使用，由插件页内按权限二次把关 */
const tabs = computed(() => {
  const list = [
    { value: 'store', label: t('appstore.tabStore') },
    { value: 'installed', label: t('appstore.tabInstalled') },
    { value: 'mine', label: t('appstore.tabMine') },
    { value: 'plugins', label: t('appstore.tabPlugins') },
  ]
  return list
})

/** 当前页签：store=应用商店；installed=已安装实例；mine=我的站点应用；plugins=插件管理 */
const activeTab = ref<'store' | 'installed' | 'mine' | 'plugins'>('store')

// 支持通过 ?tab=plugins 深链到「插件」页签（旧 /dev/plugins 入口重定向到这里）
const route = useRoute()
function syncTabFromQuery() {
  const q = route.query.tab
  if (q === 'store' || q === 'installed' || q === 'mine' || q === 'plugins') {
    activeTab.value = q
  }
}
syncTabFromQuery()
watch(() => route.query.tab, syncTabFromQuery)

/** 已安装面板提交任务后，在这里打开日志抽屉（与商店页共用一个抽屉） */
function onPaneTask(runId: string, title: string) {
  logDrawerRef.value?.openDrawer(runId, title)
}

// ── 任务队列入口（顶部按钮，覆盖全部页签）────────────────────
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

/** 队列里发生写操作（重跑 / 改脚本后重跑）→ 刷新角标 */
function onQueueChanged() {
  loadQueueCount()
}

const logDrawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
const repoDrawerRef = ref<InstanceType<typeof RepoManageDrawer> | null>(null)

/** 软件园抽屉里的异步任务（拉取 / 删除）也用同一个日志抽屉展示 */
function onRepoLog(runId: string, title: string) {
  logDrawerRef.value?.openDrawer(runId, title)
}

/** 软件园增删源后刷新当前商店包列表 */
function onRepoChanged() {
  packagesRef.value?.reload()
}

onMounted(() => {
  loadQueueCount()
})
</script>

<style scoped>
.appstore-page {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

/* 页内导航（应用商店单一菜单，已安装 / 我的站点应用在这里切） */
.view-tabs {
  margin: 12px 0 2px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.view-tabs__actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
