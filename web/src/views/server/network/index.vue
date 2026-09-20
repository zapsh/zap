<template>
  <NavPillPanels :tabs="tabs" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Dns, Link } from '@/icons'
import NavPillPanels from '@/components/NavPillPanels.vue'
import NetworkPanel from './NetworkPanel.vue'
import IpPanel from './IpPanel.vue'

const { t } = useI18n()

/**
 * 网络配置：网络设置 + IP 设置。
 * 主机名 / DNS resolver 与 IP 地址池是一套东西，合成一页（见 menus 75，旧 76 已停用）。
 */
const tabs = computed(() => [
  {
    key: 'network',
    label: t('serverNetwork.title'),
    icon: Link,
    panel: NetworkPanel,
    hint: t('serverGroups.hintNetwork'),
  },
  {
    key: 'ip',
    label: t('serverIp.title'),
    icon: Dns,
    panel: IpPanel,
    hint: t('serverGroups.hintIp'),
  },
])
</script>

<script lang="ts">
export default { name: 'ServerNetwork' }
</script>
