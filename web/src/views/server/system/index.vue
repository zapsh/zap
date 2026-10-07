<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Key, Timer } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import TimePanel from './TimePanel.vue'
import SshPanel from './SshPanel.vue'

const { t } = useI18n()

/**
 * 系统管理：服务器时间 / SSH 服务。
 *
 * 系统服务与进程管理已抽离到「服务器状态」下（监控归监控、配置归配置），
 * 这里只留与本机设置相关的两项。computed 包一层：切换语言时 pill 文案跟着变。
 */
const tabs = computed(() => [
  {
    key: 'time',
    label: t('serverTime.title'),
    icon: Timer,
    panel: TimePanel,
    hint: t('serverGroups.hintTime'),
  },
  {
    key: 'ssh',
    label: t('serverSsh.title'),
    icon: Key,
    panel: SshPanel,
    hint: t('serverGroups.hintSsh'),
  },
])
</script>

<script lang="ts">
export default { name: 'ServerSystem' }
</script>
