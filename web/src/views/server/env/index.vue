<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { MagicStick, Refresh } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import RuntimePanel from './RuntimePanel.vue'
import EntitiesPanel from './EntitiesPanel.vue'

const { t } = useI18n()

/**
 * 运行环境：探测结果 + 运行实体同步。
 * 「同步运行环境」原本是独立菜单，收进来做第二个 pill（见 menus 77，旧 78 已停用）。
 */
const tabs = computed(() => [
  {
    key: 'runtime',
    label: t('serverEnv.runtimeTab'),
    icon: MagicStick,
    panel: RuntimePanel,
    hint: t('serverGroups.hintRuntime'),
  },
  {
    key: 'entities',
    label: t('serverEntities.syncTab'),
    icon: Refresh,
    panel: EntitiesPanel,
    hint: t('serverGroups.hintEntities'),
  },
])
</script>

<script lang="ts">
export default { name: 'ServerEnv' }
</script>
