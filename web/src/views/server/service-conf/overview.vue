<template>
  <div v-loading="loading" class="svc-overview">
    <div class="ov-head">
      <span class="ov-desc">{{ t('servicesOverview.desc') }}</span>
      <el-button size="small" :loading="loading" @click="load">
        {{ t('common.refresh') }}
      </el-button>
    </div>

    <!-- 平台没有 systemd 时，状态与自启都没法管，直接降级提示 -->
    <el-alert
      v-if="!overview.systemd"
      class="ov-alert"
      type="warning"
      show-icon
      :closable="false"
      :title="t('servicesOverview.noSystemd')"
    />

    <el-empty
      v-if="!loading && !overview.items.length"
      :description="t('servicesOverview.empty')"
    />

    <div v-else class="ov-grid">
      <el-card
        v-for="item in overview.items"
        :key="item.key"
        shadow="never"
        class="ov-card"
      >
        <div class="ov-card-head">
          <div class="ov-title">
            <span class="ov-name">{{ item.name }}</span>
            <el-tag v-if="item.version" size="small" type="info">{{ item.version }}</el-tag>
            <el-tag size="small" effect="plain">{{ item.category }}</el-tag>
            <el-tag v-if="item.instance && item.instance !== '-'" size="small" effect="plain">
              {{ item.instance }}
            </el-tag>
          </div>
          <el-tag size="small" :type="stateType(item.state)">
            {{ stateText(item.state) }}
          </el-tag>
        </div>

        <div class="ov-unit">
          <span class="mono">{{ item.svc }}</span>
          <el-tag v-if="!item.exists" size="small" type="danger" effect="plain">
            {{ t('servicesOverview.unitMissing') }}
          </el-tag>
        </div>

        <div class="ov-foot">
          <div class="ov-boot">
            <span class="ov-boot-label">{{ t('servicesOverview.autostart') }}</span>
            <el-switch
              size="small"
              :model-value="item.enabled"
              :disabled="!overview.systemd || acting === item.svc"
              @change="(v: string | number | boolean) => toggleBoot(item, !!v)"
            />
          </div>
          <div class="ov-actions">
            <el-button
              size="small"
              type="success"
              plain
              :disabled="item.state === 'running' || busy(item)"
              @click="doControl(item, 'start')"
            >
              {{ t('servicesOverview.start') }}
            </el-button>
            <el-button
              size="small"
              type="warning"
              plain
              :disabled="item.state !== 'running' || busy(item)"
              @click="doControl(item, 'stop')"
            >
              {{ t('servicesOverview.stop') }}
            </el-button>
            <el-button
              size="small"
              type="primary"
              plain
              :disabled="busy(item)"
              @click="doControl(item, 'restart')"
            >
              {{ t('servicesOverview.restart') }}
            </el-button>
          </div>
        </div>
      </el-card>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import {
  controlService,
  getServicesOverview,
  setServiceBoot,
  type ServiceItem,
  type ServiceState,
  type ServicesOverview,
} from '@/api/services'

const { t } = useI18n()

const loading = ref(false)
/** 正在操作的服务 unit（避免同一张卡上并发点击） */
const acting = ref('')
const overview = reactive<ServicesOverview>({ items: [], systemd: true })

function busy(item: ServiceItem) {
  return acting.value === item.svc || !overview.systemd
}

function stateType(state: ServiceState) {
  if (state === 'running') return 'success'
  if (state === 'failed') return 'danger'
  if (state === 'starting' || state === 'stopping') return 'warning'
  return 'info'
}

function stateText(state: ServiceState) {
  const key = `servicesOverview.state.${state}`
  const text = t(key)
  // t 找不到键时会回显键名，这里兜底成 unknown
  return text === key ? t('servicesOverview.state.unknown') : text
}

async function load() {
  loading.value = true
  try {
    const res = await getServicesOverview()
    overview.items = res.data.items ?? []
    overview.systemd = res.data.systemd !== false
  } catch {
    /* 拦截器已提示 */
  } finally {
    loading.value = false
  }
}

async function doControl(item: ServiceItem, action: 'start' | 'stop' | 'restart') {
  acting.value = item.svc
  try {
    await controlService(item.svc, action)
    ElMessage.success(t('servicesOverview.actionOk'))
  } catch {
    /* 拦截器已提示 */
  } finally {
    acting.value = ''
    load()
  }
}

async function toggleBoot(item: ServiceItem, enable: boolean) {
  acting.value = item.svc
  try {
    await setServiceBoot(item.svc, enable)
    ElMessage.success(t('servicesOverview.bootOk'))
  } catch {
    /* 拦截器已提示 */
  } finally {
    acting.value = ''
    load()
  }
}

onMounted(load)
</script>

<style scoped>
.svc-overview {
  min-height: 200px;
}

.ov-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}

.ov-desc {
  font-size: 13px;
  color: var(--el-text-color-secondary);
}

.ov-alert {
  margin-bottom: 12px;
}

.ov-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 12px;
}

.ov-card {
  border-radius: 8px;
}

.ov-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.ov-title {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.ov-name {
  font-size: 15px;
  font-weight: 600;
}

.ov-unit {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 0 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.ov-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.ov-boot {
  display: flex;
  align-items: center;
  gap: 6px;
}

.ov-boot-label {
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.ov-actions {
  display: flex;
  gap: 6px;
}

.mono {
  font-family: var(--el-font-family-mono, monospace);
}
</style>
