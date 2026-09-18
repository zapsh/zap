<template>
  <div class="docker-page">
    <!-- 顶部：环境状态 + 全局操作 -->
    <header class="docker-topbar">
      <div class="docker-topbar__title">
        <el-icon :size="18"><Box /></el-icon>
        <h3>{{ t('docker.title') }}</h3>
        <el-tag size="small" round effect="dark" :type="envType">{{ envText }}</el-tag>
        <el-tag v-if="env && env.daemon && !env.compose" size="small" round effect="plain" type="warning">
          {{ t('docker.env.composeOff') }}
        </el-tag>
      </div>
      <div class="docker-topbar__actions">
        <el-tooltip :content="t('docker.autoRefreshTip')" placement="bottom">
          <el-checkbox v-model="autoRefresh" size="small">{{ t('docker.autoRefresh') }}</el-checkbox>
        </el-tooltip>
        <el-button size="small" :icon="Refresh" @click="reload">{{ t('docker.refresh') }}</el-button>
      </div>
    </header>

    <el-alert
      v-if="envHint"
      :title="envHint"
      type="warning"
      :closable="false"
      show-icon
      class="docker-alert"
    />

    <!-- 中段：nav pill 导航 -->
    <nav class="docker-pills">
      <button
        v-for="tab in tabs"
        :key="tab.key"
        type="button"
        class="docker-pill"
        :class="{ 'is-active': active === tab.key }"
        @click="active = tab.key"
      >
        <span>{{ tab.label }}</span>
        <span class="docker-pill__count">{{ counts[tab.key] ?? 0 }}</span>
      </button>
    </nav>

    <!-- 下方：功能区 -->
    <section class="docker-panel">
      <component
        :is="currentPanel.component"
        :refresh-token="token"
        @count="(n: number) => (counts[active] = n)"
        @refresh="reload"
      />
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Box, Refresh } from '@/icons'
import { dockerStatus, type DockerEnvStatus } from '@/api/docker'
import Containers from './panels/Containers.vue'
import Images from './panels/Images.vue'
import Volumes from './panels/Volumes.vue'
import Networks from './panels/Networks.vue'
import Compose from './panels/Compose.vue'

const { t } = useI18n()

const active = ref('containers')
const token = ref(0)
const autoRefresh = ref(false)
const counts = reactive<Record<string, number>>({})

const env = ref<DockerEnvStatus | null>(null)
/** Compose 面板用它判断插件是否可用 */
provide('docker-env', env)

const tabs = computed(() => [
  { key: 'containers', label: t('docker.tabs.containers'), component: Containers },
  { key: 'images', label: t('docker.tabs.images'), component: Images },
  { key: 'volumes', label: t('docker.tabs.volumes'), component: Volumes },
  { key: 'networks', label: t('docker.tabs.networks'), component: Networks },
  { key: 'compose', label: t('docker.tabs.compose'), component: Compose },
])

const currentPanel = computed(() => tabs.value.find((p) => p.key === active.value) ?? tabs.value[0])

const envText = computed(() => {
  if (!env.value) return t('docker.env.unknown')
  if (!env.value.installed) return t('docker.env.missing')
  if (!env.value.daemon) return t('docker.env.stopped')
  return t('docker.env.ready', { version: env.value.version || '' })
})

const envType = computed(() => (env.value?.daemon ? 'success' : 'danger'))

const envHint = computed(() => {
  if (!env.value) return ''
  if (!env.value.installed) return t('docker.env.hintMissing')
  if (!env.value.daemon) {
    const detail = env.value.error?.trim()
    return detail ? `${t('docker.env.hintStopped')}（${detail}）` : t('docker.env.hintStopped')
  }
  return ''
})

let timer: ReturnType<typeof setInterval> | undefined

function reload() {
  token.value += 1
  loadEnv()
}

async function loadEnv() {
  try {
    env.value = (await dockerStatus()).data
  } catch {
    // 探测接口失败时保持上一次状态；各面板自身会给出错误提示
    if (!env.value) env.value = null
  }
}

watch(autoRefresh, (v) => {
  if (timer) clearInterval(timer)
  timer = v ? setInterval(() => (token.value += 1), 5000) : undefined
})

onMounted(loadEnv)
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.docker-page {
  padding: 16px;
}

.docker-topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.docker-topbar__title {
  display: flex;
  align-items: center;
  gap: 8px;
}

.docker-topbar__title h3 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}

.docker-topbar__actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.docker-alert {
  margin-bottom: 12px;
}

/* Docker Desktop 风格的分段导航 */
.docker-pills {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px;
  margin-bottom: 12px;
  background: var(--el-fill-color-light);
  border-radius: 10px;
  overflow-x: auto;
}

.docker-pill {
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

.docker-pill:hover {
  color: var(--el-color-primary);
}

.docker-pill.is-active {
  background: var(--el-color-primary);
  color: #fff;
  font-weight: 500;
}

.docker-pill__count {
  min-width: 20px;
  padding: 0 6px;
  border-radius: 8px;
  background: var(--el-fill-color);
  font-size: 12px;
  text-align: center;
  color: var(--el-text-color-secondary);
}

.docker-pill.is-active .docker-pill__count {
  background: rgba(255, 255, 255, 0.24);
  color: #fff;
}
</style>
