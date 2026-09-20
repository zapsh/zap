<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useUserStore } from '@/stores/user'
import { InfoFilled, Refresh } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import AboutPanel from './AboutPanel.vue'
import UpdatePanel from './UpdatePanel.vue'

const { t } = useI18n()
const userStore = useUserStore()

/**
 * 「系统设置 → About ZAP」：版本/文档 + 系统更新。
 * 原先是两条独立菜单（menus 30 / 27），现在合成一页，用 nav pill 切换。
 * About ZAP 对所有角色开放（文档谁都能看），但「系统更新」的接口只认 admin，
 * 所以非管理员不带该页签（只剩一个标签时 NavPillPanels 不渲染导航条），免得点进去一串 403。
 * computed 包一层：切换语言时 pill 文案跟着变。
 */
const tabs = computed(() => [
  {
    key: 'about',
    label: t('menu.about'),
    icon: InfoFilled,
    panel: AboutPanel,
    hint: t('aboutGroups.hintAbout'),
  },
  ...(userStore.roles.includes('admin')
    ? [
        {
          key: 'update',
          label: t('menu.update'),
          icon: Refresh,
          panel: UpdatePanel,
          hint: t('aboutGroups.hintUpdate'),
        },
      ]
    : []),
])
</script>

<script lang="ts">
export default { name: 'SystemAbout' }
</script>
