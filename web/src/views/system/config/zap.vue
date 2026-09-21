<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Bell, Setting } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import ZapPanel from './ZapPanel.vue'
import NotifyPanel from './NotifyPanel.vue'

const { t } = useI18n()

/**
 * 「系统设置 → Zap 设置」：面板自身配置 + 通知渠道，用 nav pill 切换。
 *
 * 原「基础设置」页（menus `basic-config`）已下线：其中的 Mail 挪到「通知设置」，
 * 建站默认网络与联系信息不再提供界面入口（已存配置仍在 server_env.yaml）。
 * computed 包一层：切换语言时 pill 文案跟着变。
 */
const tabs = computed(() => [
  {
    key: 'zap',
    label: t('zapCfg.title'),
    icon: Setting,
    panel: ZapPanel,
    hint: t('zapCfg.pillHint'),
  },
  {
    key: 'notify',
    label: t('notifyCfg.title'),
    icon: Bell,
    panel: NotifyPanel,
    hint: t('notifyCfg.pillHint'),
  },
])
</script>

<script lang="ts">
export default { name: 'ZapConfig' }
</script>
