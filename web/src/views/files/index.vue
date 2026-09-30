<template>
  <div class="files-page">
    <!-- 顶部菜单：本地存储 / 云存储 切换（nav pill 按钮组） -->
    <div class="files-topbar">
      <el-radio-group v-model="activeTab" class="files-switch">
        <el-radio-button value="local">
          <el-icon><Monitor /></el-icon>
          <span>{{ t('files.tabLocal') }}</span>
        </el-radio-button>
        <el-radio-button value="cloud">
          <el-icon><Cloud /></el-icon>
          <span>{{ t('files.tabCloud') }}</span>
        </el-radio-button>
      </el-radio-group>
    </div>

    <div class="files-body">
      <!-- 本地存储：服务器本地目录（原文件管理），常驻挂载保留当前目录 -->
      <LocalPane v-show="activeTab === 'local'" class="files-pane" />

      <!-- 云存储：对象存储（S3 / OSS / COS / S3 兼容），首次切到云存储时才挂载 -->
      <CloudPane v-if="cloudMounted" v-show="activeTab === 'cloud'" class="files-pane" />
    </div>
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
/** 云存储首访后才挂载，之后常驻（等价于原 el-tab-pane 的 lazy + keep-alive） */
const cloudMounted = ref(activeTab.value === 'cloud')

watch(activeTab, (value) => {
  sessionStorage.setItem(STORAGE_KEY, value)
  if (value === 'cloud') cloudMounted.value = true
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

/* 顶部菜单：本地/云 切换条 */
.files-topbar {
  margin-bottom: 10px;
}

/* 内容区：撑满剩余高度，子面板用 height: 100% 拿到确定高度 */
.files-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.files-pane {
  flex: 1;
  min-height: 0;
  height: 100%;
}

/* nav pill 样式：把 el-radio-button 渲染成独立的圆角药丸按钮 */
.files-switch {
  :deep(.el-radio-button__inner) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--el-border-color);
    border-radius: 999px;
    margin: 0 4px;
    box-shadow: none;
    transition:
      color 0.2s,
      background-color 0.2s,
      border-color 0.2s;
  }

  /* 去掉首尾按钮的特殊圆角（默认连成一段），统一成药丸 */
  :deep(.el-radio-button:first-child .el-radio-button__inner),
  :deep(.el-radio-button:last-child .el-radio-button__inner) {
    border-radius: 999px;
  }

  /* 选中态：主色填充成药丸 */
  :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) {
    color: #fff;
    background-color: var(--el-color-primary);
    border-color: var(--el-color-primary);
    box-shadow: none;
  }

  :deep(.el-radio-button__original-radio:focus-visible + .el-radio-button__inner) {
    box-shadow: 0 0 0 2px var(--el-color-primary-light-5);
  }
}
</style>
