<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useUserStore } from '@/stores/user'
import { Setting, User } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import MyBackup from './my.vue'
import BackupSettings from './Settings.vue'

const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles?.includes('admin') === true)

/**
 * 备份页 = 我的备份 + 备份设置两个 pill（站点同款：独立面板 + ?tab= 路由）。
 * 「备份设置」仅管理员可见，非管理员只看到「我的备份」一个面板（导航自动隐藏）。
 */
const tabs = computed(() => {
  const list = [
    {
      key: 'my',
      label: '我的备份',
      icon: User,
      panel: MyBackup,
      hint: '个人保留份数与备份归档，可自助还原',
    },
  ]
  if (isAdmin.value) {
    list.push({
      key: 'settings',
      label: '备份设置',
      icon: Setting,
      panel: BackupSettings,
      hint: '存储目录 / 立即备份 / 计划任务 / 备份策略',
    })
  }
  return list
})
</script>

<script lang="ts">
export default { name: 'BackupIndex' }
</script>
