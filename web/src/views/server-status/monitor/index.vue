<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="monitor-page">
    <el-alert
      type="info"
      :closable="false"
      show-icon
      :title="t('statusMonitor.hint')"
      style="margin-bottom: 16px"
    />
    <SystemMonitor show-refresh-hint />
  </div>
</template>

<script setup lang="ts">
/**
 * 服务器状态 → Server Monitor：CPU / 内存 / 系统负载 / 网络实时图表。
 *
 * 与首页「系统信息」是同一个组件（`@/components/SystemMonitor.vue`），
 * 数据来自 `/system/status`，默认每 5 秒刷新一次。
 */
import { useI18n } from 'vue-i18n'
import SystemMonitor from '@/components/SystemMonitor.vue'

const { t } = useI18n()
</script>
