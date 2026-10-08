<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Download, Setting, Bell } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import ZapPanel from './ZapPanel.vue'
import MirrorPanel from './MirrorPanel.vue'
import CachePanel from './CachePanel.vue'
import SecurityPanel from './SecurityPanel.vue'
import NotifyIndex from '../../notify/index.vue'

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
    key: 'mirror',
    label: t('mirrorCfg.title'),
    icon: Download,
    panel: MirrorPanel,
    hint: t('mirrorCfg.pillHint'),
  },
  {
    key: 'cache',
    label: t('zapCfg.cacheTitle'),
    icon: Setting,
    panel: CachePanel,
    hint: t('zapCfg.cacheSubtitle'),
  },
  {
    key: 'security',
    label: t('zapCfg.secTitle'),
    icon: Setting,
    panel: SecurityPanel,
    hint: t('zapCfg.secSubtitle'),
  },
  {
    key: 'notify',
    label: t('zapCfg.notifyTitle'),
    icon: Bell,
    panel: NotifyIndex,
    hint: t('notifyCfg.subtitle'),
  },
])
</script>

<script lang="ts">
export default { name: 'ZapConfig' }
</script>
