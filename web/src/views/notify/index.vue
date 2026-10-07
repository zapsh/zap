<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Bell, Setting } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import { useUserStore } from '@/stores/user'
import NotifyTemplatePane from './NotifyTemplatePane.vue'
import NotifyChannelPane from './NotifyChannelPane.vue'
import NotifyTestPane from './NotifyTestPane.vue'

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

/**
 * 「通知设置」对 admin / user / reseller 都开放入口：
 * - 通知模板：所有人可自定义（reseller 仅影响自己名下用户）；
 * - 发信渠道 / 测试邮件：仅管理员可见（全局发信配置）。
 */
const tabs = computed(() => {
  const list = [
    {
      key: 'templates',
      label: t('notifyCfg.pillTemplates'),
      icon: Bell,
      panel: NotifyTemplatePane,
      hint: t('notifyCfg.templatesHint'),
    },
  ]
  if (isAdmin.value) {
    list.push(
      {
        key: 'channel',
        label: t('notifyCfg.pillChannel'),
        icon: Setting,
        panel: NotifyChannelPane,
        hint: t('notifyCfg.subtitle'),
      },
      {
        key: 'test',
        label: t('notifyCfg.pillTest'),
        icon: Setting,
        panel: NotifyTestPane,
        hint: t('notifyCfg.testMailHint'),
      },
    )
  }
  return list
})
</script>

<script lang="ts">
export default { name: 'NotifySettings' }
</script>
