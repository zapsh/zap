<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-config-provider :locale="elementLocale">
    <router-view />
  </el-config-provider>
</template>

<script setup lang="ts">
import { useLocale } from '@/composables/useLocale'

const { elementLocale } = useLocale()
</script>

<style>
html,
body {
  margin: 0;
  padding: 0;
  height: 100%;
  font-family:
    -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
}

#app {
  height: 100%;
}

/* 滚动条样式 */
::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

/* 用 Element Plus 变量，浅色 / 深色主题自动适配 */
::-webkit-scrollbar-track {
  background: var(--el-fill-color-light);
  border-radius: 3px;
}

::-webkit-scrollbar-thumb {
  background: var(--el-border-color-darker);
  border-radius: 3px;
}

::-webkit-scrollbar-thumb:hover {
  background: var(--el-text-color-secondary);
}
</style>
