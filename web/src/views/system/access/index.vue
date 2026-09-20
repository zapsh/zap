<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Lock, User } from '@/icons'
import { useUserStore } from '@/stores/user'
import NavPillPanels from '@/components/NavPillPanels.vue'
import UsersPanel from './UsersPanel.vue'
import RolesPanel from './RolesPanel.vue'

const { t } = useI18n()
const userStore = useUserStore()

/** 角色管理只有 admin 能进：reseller 只看到用户（客户）面板 */
const isAdmin = computed(() => userStore.roles.includes('admin'))

/**
 * 用户 + 角色合成一页，页内 nav pill 切换（见 menus 21，旧 22 已停用）。
 * computed 包一层：切换语言时 pill 文案跟着变，tabs 增减也由 NavPillPanels 自动纠正。
 */
const tabs = computed(() => {
  const list: { key: string; label: string; icon: unknown; panel: unknown; hint: string }[] = [
    {
      key: 'users',
      label: t('users.title'),
      icon: User,
      panel: UsersPanel,
      hint: t('users.titleHint'),
    },
  ]
  if (isAdmin.value) {
    list.push({
      key: 'roles',
      label: t('roles.title'),
      icon: Lock,
      panel: RolesPanel,
      hint: t('roles.titleHint'),
    })
  }
  return list as never
})
</script>

<script lang="ts">
export default { name: 'SystemAccess' }
</script>
