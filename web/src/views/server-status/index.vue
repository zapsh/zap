<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
/**
 * 服务器状态：一个路由下 7 个面板，页内 nav pill 切换（与站点、系统管理同一套）。
 *
 * pill 的 key 同时是地址栏 ?tab= 的值，旧链接 / 首页快捷入口照旧直达：
 * `?tab=monitor`（服务器状态卡片）、`?tab=disk`、`?tab=network`（首页磁盘 / 网络）。
 * computed 包一层：切换语言时 pill 文案跟着变。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  Connection,
  Cpu,
  DataLine,
  HardDrive,
  InfoFilled,
  Memory,
  Monitor,
  Odometer,
} from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import InfoPage from './info/index.vue'
import MonitorPage from './monitor/index.vue'
import LoadPage from './load/index.vue'
import CpuPage from './cpu/index.vue'
import MemoryPage from './memory/index.vue'
import DiskPage from './disk/index.vue'
import NetworkPage from './network/index.vue'

const { t } = useI18n()

const tabs = computed(() => [
  { key: 'info', label: t('statusTabs.info'), icon: InfoFilled, panel: InfoPage },
  { key: 'monitor', label: t('statusTabs.monitor'), icon: DataLine, panel: MonitorPage },
  { key: 'load', label: t('statusTabs.load'), icon: Odometer, panel: LoadPage },
  { key: 'cpu', label: t('statusTabs.cpu'), icon: Cpu, panel: CpuPage },
  { key: 'memory', label: t('statusTabs.memory'), icon: Memory, panel: MemoryPage },
  { key: 'disk', label: t('statusTabs.disk'), icon: HardDrive, panel: DiskPage },
  { key: 'network', label: t('statusTabs.network'), icon: Connection, panel: NetworkPage },
])
</script>

<script lang="ts">
export default { name: 'ServerStatusIndex' }
</script>

<style scoped>
.pill-panels {
  min-height: 60vh;
}
</style>
