<!-- SPDX-License-Identifier: AGPL-3.0-only -->
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

    <el-table
      v-else
      v-loading="loading"
      :data="overview.items"
      size="small"
      stripe
      class="ov-table"
      :default-expand-all="false"
    >
      <el-table-column :label="t('servicesOverview.colService')" min-width="260">
        <template #default="{ row }">
          <div class="ov-svc">
            <span class="ov-name">{{ row.name }}</span>
            <el-tag v-if="row.version" size="small" type="info">{{ row.version }}</el-tag>
            <el-tag size="small" effect="plain">{{ row.category }}</el-tag>
            <el-tag v-if="row.instance && row.instance !== '-'" size="small" effect="plain">
              {{ row.instance }}
            </el-tag>
          </div>
          <div class="ov-unit mono">
            {{ row.svc }}
            <el-tag v-if="!row.exists" size="small" type="danger" effect="plain">
              {{ t('servicesOverview.unitMissing') }}
            </el-tag>
          </div>
        </template>
      </el-table-column>

      <el-table-column :label="t('servicesOverview.colState')" width="120">
        <template #default="{ row }">
          <el-tag size="small" :type="stateType(row.state)">{{ stateText(row.state) }}</el-tag>
        </template>
      </el-table-column>

      <el-table-column :label="t('servicesOverview.autostart')" width="130">
        <template #default="{ row }">
          <el-switch
            size="small"
            :model-value="row.enabled"
            :disabled="!overview.systemd || acting === row.svc"
            @change="(v: string | number | boolean) => toggleBoot(row, !!v)"
          />
        </template>
      </el-table-column>

      <el-table-column :label="t('common.operation')" width="220" fixed="right">
        <template #default="{ row }">
          <el-button
            size="small"
            type="success"
            plain
            :disabled="row.state === 'running' || busy(row)"
            @click="doControl(row, 'start')"
          >
            {{ t('servicesOverview.start') }}
          </el-button>
          <el-button
            size="small"
            type="warning"
            plain
            :disabled="row.state !== 'running' || busy(row)"
            @click="doControl(row, 'stop')"
          >
            {{ t('servicesOverview.stop') }}
          </el-button>
          <el-button
            size="small"
            type="primary"
            plain
            :disabled="busy(row)"
            @click="doControl(row, 'restart')"
          >
            {{ t('servicesOverview.restart') }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>
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

.ov-table {
  margin-top: 4px;
}

.ov-svc {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.ov-name {
  font-size: 14px;
  font-weight: 600;
}

.ov-unit {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  flex-wrap: wrap;
}

.mono {
  font-family: var(--el-font-family-mono, monospace);
}
</style>
