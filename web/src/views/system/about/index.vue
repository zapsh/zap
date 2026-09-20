<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { InfoFilled, Refresh } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import AboutPanel from './AboutPanel.vue'
import UpdatePanel from './UpdatePanel.vue'

const { t } = useI18n()

/**
 * 「系统设置 → About ZAP」：版本/文档 + 系统更新。
 * 原先是两条独立菜单（menus 30 / 27），现在合成一页，用 nav pill 切换。
 * 两个页签对所有角色开放：版本信息谁都能看；「系统更新」只读（状态与升级日志
 * 的后端门禁已放宽到登录用户），执行类操作（检查 / 升级 / 保存配置）在面板里
 * 按 admin 判断并禁用，非管理员看得见但点不动。
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
  {
    key: 'update',
    label: t('menu.update'),
    icon: Refresh,
    panel: UpdatePanel,
    hint: t('aboutGroups.hintUpdate'),
  },
])
</script>

<script lang="ts">
export default { name: 'SystemAbout' }
</script>
