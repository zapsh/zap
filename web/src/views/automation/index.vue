<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Document, Timer } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import ScriptsPanel from './ScriptsPanel.vue'
import CronPanel from './CronPanel.vue'

const { t } = useI18n()

/**
 * 自动化脚本：自定义脚本 + 计划任务。
 * 原先是两条独立菜单（menus 101 / 102），现在合成一页，用 nav pill 切换。
 * computed 包一层：切换语言时 pill 文案跟着变。
 */
const tabs = computed(() => [
  {
    key: 'scripts',
    label: t('automationScripts.title'),
    icon: Document,
    panel: ScriptsPanel,
    hint: t('automationGroups.hintScripts'),
  },
  {
    key: 'cron',
    label: t('automationCron.title'),
    icon: Timer,
    panel: CronPanel,
    hint: t('automationGroups.hintCron'),
  },
])
</script>

<script lang="ts">
export default { name: 'AutomationScripts' }
</script>
