<template>
  <div class="svc-conf-page">
    <!-- 服务配置：各运行服务（应用商店安装）的配置页，用 nav pill 切换 -->
    <nav class="svc-pills">
      <button
        v-for="tab in tabs"
        :key="tab.key"
        type="button"
        class="svc-pill"
        :class="{ 'is-active': active === tab.key }"
        @click="active = tab.key"
      >
        <span>{{ tab.label }}</span>
      </button>
    </nav>

    <section class="svc-panel">
      <!-- 各服务页自带状态卡与操作区，这里只负责切换；key 保证切回来时重新拉取状态 -->
      <component :is="current.component" :key="active" />
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import NginxConf from '@/views/services/nginx/index.vue'
import PhpConf from '@/views/services/php/index.vue'
import MysqlConf from '@/views/services/mysql/index.vue'

const { t } = useI18n()

const active = ref('nginx')

const tabs = computed(() => [
  { key: 'nginx', label: t('serviceConf.nginx'), component: NginxConf },
  { key: 'php', label: t('serviceConf.php'), component: PhpConf },
  { key: 'mysql', label: t('serviceConf.mysql'), component: MysqlConf },
])

const current = computed(() => tabs.value.find((x) => x.key === active.value) ?? tabs.value[0])
</script>

<style scoped>
.svc-conf-page {
  padding: 16px;
}

/* 与容器管理一致的分段导航 */
.svc-pills {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px;
  margin-bottom: 12px;
  background: var(--el-fill-color-light);
  border-radius: 10px;
  overflow-x: auto;
}

.svc-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 16px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--el-text-color-regular);
  font-size: 13px;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.2s, color 0.2s;
}

.svc-pill:hover {
  color: var(--el-color-primary);
}

.svc-pill.is-active {
  background: var(--el-color-primary);
  color: #fff;
  font-weight: 500;
}
</style>
