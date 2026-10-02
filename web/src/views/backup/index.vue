<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useUserStore } from '@/stores/user'
import { Setting, User, Timer } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import MyBackup from './my.vue'
import JobTasks from './jobs.vue'
import BackupSettings from './Settings.vue'

const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles?.includes('admin') === true)

/**
 * 备份页 = 我的备份 + 备份任务 + 备份设置三个 pill（站点同款：独立面板 + ?tab= 路由）。
 * - 「我的备份」「备份任务」对所有登录角色可见：普通用户可自助管理自己的定时备份任务。
 * - 「备份任务」是否真正可用，由「备份策略 → 允许用户定时备份」开关控制（后端 scope + 前端提示）。
 * - 「备份设置」仍仅管理员可见（存储目录 / 立即备份 / 备份策略等全局配置）。
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
    {
      key: 'jobs',
      label: '备份任务',
      icon: Timer,
      panel: JobTasks,
      hint: '定时备份自己的目录或数据库（受策略开关约束）',
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
