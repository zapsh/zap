<template>
  <div class="files-page">
    <NavPillPanels :tabs="tabs" />
  </div>
</template>

<script setup lang="ts">
/**
 * 文件管理入口：本地存储 / 云存储 两个 pill（站点同款：独立面板 + ?tab= 路由）。
 *
 * - 本地存储 = 服务器本地目录管理（`LocalPane.vue`，即原文件管理页）；
 * - 云存储 = 对象存储管理（`CloudPane.vue`，基于 opendal，可配置多套存储）。
 *
 * 两块是独立的组件与接口，互不影响：本地走高权限的 zapexec，云存储走 S3 协议的 HTTP API。
 * 切到云存储才首次挂载 `CloudPane`，之后由 NavPillPanels 的 KeepAlive 常驻缓存（等价于
 * 原 el-tab-pane 的 lazy + keep-alive），切走再回来实例不销毁、当前目录还在。
 */
import { computed, onActivated, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { Cloud, Monitor } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import LocalPane from './LocalPane.vue'
import CloudPane from './CloudPane.vue'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()

/**
 * 记忆上次停留的 tab：从菜单/链接重新进入文件管理时，恢复走之前的面板（本地/云），
 * 而不是每次都回到默认。用 sessionStorage + ?tab= 路由：
 * - 在文件管理内切换 pill 时，把当前 tab 存进 sessionStorage；
 * - 重新进入（且 URL 没显式带 ?tab=）时，回写进路由，NavPillPanels 据此恢复。
 * 仅在 /files 路径下读写，避免与其它页的 ?tab= 串味。
 */
const STORAGE_KEY = 'files-active-tab'

watch(
  () => route.query.tab,
  (tab) => {
    if (route.path.startsWith('/files') && tab) {
      sessionStorage.setItem(STORAGE_KEY, String(tab))
    }
  },
)

onActivated(() => {
  if (!route.path.startsWith('/files')) return
  const saved = sessionStorage.getItem(STORAGE_KEY)
  // URL 显式带了 ?tab= 就尊重它；只有没带时才用记忆值回写
  if (saved && !route.query.tab) {
    router.replace({ query: { ...route.query, tab: saved } })
  }
})

const tabs = computed(() => [
  {
    key: 'local',
    label: t('files.tabLocal'),
    icon: Monitor,
    panel: LocalPane,
    hint: t('files.tabLocal'),
  },
  {
    key: 'cloud',
    label: t('files.tabCloud'),
    icon: Cloud,
    panel: CloudPane,
    hint: t('files.tabCloud'),
  },
])
</script>

<script lang="ts">
// 组件名要和 keep-alive 白名单对得上（include 按组件 name 匹配，不是路由 name）；
// 文件管理是常驻存活的页面，切走再回来实例不销毁、当前目录还在。见 stores/tags.ts。
export default { name: 'FileManager' }
</script>

<style scoped lang="scss">
/* 页面整体高度与布局内容区对齐（布局：100vh - 状态栏/头部 - 内边距） */
.files-page {
  height: calc(100vh - 110px);
  display: flex;
  flex-direction: column;
}

/* NavPillPanels 撑满页面；导航占自身高度，面板 flex:1 占满剩余空间，
   两栏布局靠面板内部 height:100% 拿到确定高度 */
:deep(.pill-panels) {
  flex: 1;
  min-height: 0;
}
:deep(.pill-panels) > .panel-nav {
  flex: none;
}
:deep(.pill-panels) > :last-child {
  flex: 1;
  min-height: 0;
}
</style>
