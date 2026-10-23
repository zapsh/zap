<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="svc-conf-page">
    <!-- 服务配置：总览 + 各服务独立配置页 + 自定义服务（管理员自建）。
         固定页 + 自定义服务页较多时，统一用顶部横向 pill，溢出时在容器内滑动，不切侧边栏。 -->
    <NavPillPanels :tabs="tabs" layout="pill" />
  </div>
</template>

<script setup lang="ts">
import { computed, markRaw, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import NavPillPanels, { type PillTab } from '@/components/NavPillPanels.vue'
import Overview from './overview.vue'
import NginxConf from './nginx.vue'
import PhpConf from './php.vue'
import MysqlConf from './mysql.vue'
import PythonConf from './python.vue'
import NodejsConf from './nodejs.vue'
import GenericServiceConf from './GenericServiceConf.vue'
import CustomServiceManager from './CustomServiceManager.vue'
import { getServiceConfDefs, type ServiceConfDef } from '@/api/servicesConf'

const { t } = useI18n()
const route = useRoute()

/** 管理员自建的服务定义（不含内置的 php/mysql…，它们已在固定页里） */
const userDefs = ref<ServiceConfDef[]>([])

async function loadUserDefs() {
  try {
    const r = await getServiceConfDefs()
    userDefs.value = (r.data.items || []).filter((d) => d.origin === 'user')
  } catch {
    userDefs.value = []
  }
}

onMounted(loadUserDefs)
// 切换 tab 时同步刷新：直接往 /etc/zap/services 丢定义、或其它入口改动后，
// 侧边栏的自定义服务 tab 也能即时出现 / 消失（不必整页刷新）。
watch(() => route.query.tab, loadUserDefs)

/** 固定页 + 自定义服务页 + 管理页，交给 NavPillPanels 渲染（含自适应布局） */
const tabs = computed<PillTab[]>(() => {
  const base: PillTab[] = [
    { key: 'overview', label: t('serviceConf.overview'), panel: Overview },
    { key: 'nginx', label: t('serviceConf.nginx'), panel: NginxConf },
    { key: 'php', label: t('serviceConf.php'), panel: PhpConf },
    { key: 'mysql', label: t('serviceConf.mysql'), panel: MysqlConf },
    { key: 'python', label: t('serviceConf.pythonTab'), panel: PythonConf },
    { key: 'nodejs', label: t('serviceConf.nodejsTab'), panel: NodejsConf },
  ]
  for (const d of userDefs.value) {
    base.push({
      key: `svc:${d.key}`,
      label: d.label,
      panel: markRaw(GenericServiceConf),
      props: { service: d.key, label: d.label },
    })
  }
  base.push({
    key: 'manage',
    label: t('serviceConf.manage'),
    panel: markRaw(CustomServiceManager),
    on: { changed: loadUserDefs },
  })
  return base
})
</script>

<style scoped>
.svc-conf-page {
  padding: 16px;
}
</style>
