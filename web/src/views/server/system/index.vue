<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Cpu, Key, Setting, Timer } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import TimePanel from './TimePanel.vue'
import ServicesPanel from './ServicesPanel.vue'
import SshPanel from './SshPanel.vue'
import ProcessPanel from './ProcessPanel.vue'

const { t } = useI18n()

/**
 * 系统管理：时间 / 服务 / SSH / 进程。
 * 原先是四条独立菜单，现在合成一页，用 nav pill 切换（见 menus 71，旧 72/73/74 已停用）。
 * computed 包一层：切换语言时 pill 文案跟着变。
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
    key: 'services',
    label: t('serverServices.title'),
    icon: Setting,
    panel: ServicesPanel,
    hint: t('serverGroups.hintServices'),
  },
  {
    key: 'ssh',
    label: t('serverSsh.title'),
    icon: Key,
    panel: SshPanel,
    hint: t('serverGroups.hintSsh'),
  },
  {
    key: 'process',
    label: t('serverProcess.title'),
    icon: Cpu,
    panel: ProcessPanel,
    hint: t('serverGroups.hintProcess'),
  },
])
</script>

<script lang="ts">
export default { name: 'ServerSystem' }
</script>
