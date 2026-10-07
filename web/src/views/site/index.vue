<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Folder, Monitor } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import SiteListPanel from './SiteListPanel.vue'
import AppManagerPanel from './AppManagerPanel.vue'

const { t } = useI18n()

/**
 * 站点页 = 站点列表 + 应用管理两个 pill。
 * 应用不再挂在某个站点下面：从站点行点「应用」会带 `?tab=apps&site=<id>` 跳进来，
 * 直接定位到该站点（见 SiteListPanel 的行操作）。
 */
const tabs = computed(() => [
  {
    key: 'sites',
    label: t('site.navSites'),
    icon: Monitor,
    panel: SiteListPanel,
    hint: t('site.navSitesHint'),
  },
  {
    key: 'apps',
    label: t('site.navApps'),
    icon: Folder,
    panel: AppManagerPanel,
    hint: t('site.navAppsHint'),
  },
])
</script>

<script lang="ts">
export default { name: 'SiteIndex' }
</script>
