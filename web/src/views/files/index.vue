<template>
  <div class="files-page">
    <el-tabs v-model="activeTab" class="files-tabs">
      <!-- 本地存储：服务器本地目录（原文件管理） -->
      <el-tab-pane name="local">
        <template #label>
          <span class="files-tab-label">
            <el-icon><Monitor /></el-icon>
            {{ t('files.tabLocal') }}
          </span>
        </template>
        <LocalPane />
      </el-tab-pane>

      <!-- 云存储：对象存储（S3 / OSS / COS / S3 兼容），首次切换时才挂载 -->
      <el-tab-pane name="cloud" lazy>
        <template #label>
          <span class="files-tab-label">
            <el-icon><Cloud /></el-icon>
            {{ t('files.tabCloud') }}
          </span>
        </template>
        <CloudPane />
      </el-tab-pane>
    </el-tabs>
  </div>
</template>

<script setup lang="ts">
/**
 * 文件管理入口：本地存储 / 云存储 两块。
 *
 * - 本地存储 = 服务器本地目录管理（`LocalPane.vue`，即原文件管理页）；
 * - 云存储 = 对象存储管理（`CloudPane.vue`，基于 opendal，可配置多套存储）。
 *
 * 两块是独立的组件与接口，互不影响：本地走高权限的 zapexec，云存储走 S3 协议的 HTTP API。
 */
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { Cloud, Monitor } from '@/icons'

import CloudPane from './CloudPane.vue'
import LocalPane from './LocalPane.vue'

const { t } = useI18n()

type TabName = 'local' | 'cloud'
const STORAGE_KEY = 'files:active-tab'

/** 记住上次停留的标签页：从别处回到文件管理时不用重新切一次 */
function resolveInitialTab(): TabName {
  return sessionStorage.getItem(STORAGE_KEY) === 'cloud' ? 'cloud' : 'local'
}

const activeTab = ref<TabName>(resolveInitialTab())

watch(activeTab, (value) => {
  sessionStorage.setItem(STORAGE_KEY, value)
})
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

.files-tabs {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;

  :deep(.el-tabs__header) {
    margin-bottom: 10px;
  }

  /* 让标签页内容撑满剩余高度，子面板才能用 height: 100% 拿到确定高度 */
  :deep(.el-tabs__content) {
    flex: 1;
    min-height: 0;
  }

  :deep(.el-tab-pane) {
    height: 100%;
  }
}

.files-tab-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
}
</style>
