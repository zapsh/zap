<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'
import { Search } from '@/icons'

interface ServiceItem {
  name: string
  load: string
  active: string
  sub: string
  description: string
}

const { t } = useI18n()

const services = ref<ServiceItem[]>([])
const loading = ref(false)
const actingName = ref('')
const filter = ref('')

const filteredServices = computed(() => {
  if (!filter.value) return services.value
  const f = filter.value.toLowerCase()
  return services.value.filter(
    (s) => s.name.toLowerCase().includes(f) || s.description.toLowerCase().includes(f),
  )
})

async function loadServices() {
  loading.value = true
  try {
    const res = await http.get<{ code: number; data: { services: ServiceItem[] } }>(
      '/system/config/services',
    )
    services.value = res.data?.services ?? []
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

function activeType(active: string): 'success' | 'danger' | 'warning' | 'info' {
  if (active === 'active') return 'success'
  if (active === 'failed') return 'danger'
  if (active === 'inactive') return 'info'
  return 'warning'
}

/** systemd 的状态是有限枚举，翻不出来的原样回显 */
function activeLabel(active: string): string {
  if (active === 'active') return t('serverServices.active')
  if (active === 'failed') return t('serverServices.failed')
  if (active === 'inactive') return t('serverServices.stopped')
  return active
}

function actionLabel(action: string): string {
  switch (action) {
    case 'start':
      return t('serverServices.start')
    case 'stop':
      return t('serverServices.stop')
    case 'restart':
      return t('serverServices.restart')
    case 'reload':
      return t('serverServices.reload')
    case 'enable':
      return t('serverServices.enableBoot')
    case 'disable':
      return t('serverServices.disableBoot')
    default:
      return action
  }
}

async function doAction(row: ServiceItem, action: string) {
  const label = actionLabel(action)
  try {
    await ElMessageBox.confirm(
      t('serverServices.actionConfirm', { name: row.name, action: label }),
      t('common.tip'),
      { type: action === 'stop' || action === 'disable' ? 'warning' : 'info' },
    )
  } catch {
    return
  }
  actingName.value = row.name
  try {
    const res = await http.post<{ code: number; message: string; data: { status?: string } }>(
      '/system/config/services/action',
      { name: row.name, action },
    )
    ElMessage.success(res.message ?? t('serverServices.actionSuccess', { action: label }))
    // 动作完成后刷新列表，保持状态最新
    await loadServices()
  } catch {
    /* handled */
  } finally {
    actingName.value = ''
  }
}

onMounted(loadServices)
</script>

<template>
  <div class="services-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>{{ t('serverServices.title') }}</span>
          <div class="header-actions">
            <el-input
              v-model="filter"
              :placeholder="t('serverServices.searchPlaceholder')"
              clearable
              style="width: 240px"
            >
              <template #prefix>
                <el-icon><Search /></el-icon>
              </template>
            </el-input>
            <el-button type="primary" :loading="loading" @click="loadServices">
              {{ t('common.refresh') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-table
        :data="filteredServices"
        v-loading="loading"
        stripe
        style="width: 100%"
        :empty-text="t('serverServices.empty')"
      >
        <el-table-column
          prop="name"
          :label="t('serverServices.serviceName')"
          min-width="220"
          show-overflow-tooltip
        />
        <el-table-column
          prop="description"
          :label="t('common.description')"
          min-width="260"
          show-overflow-tooltip
        />
        <el-table-column prop="load" :label="t('serverServices.load')" width="90" align="center">
          <template #default="{ row }">
            <el-tag size="small" :type="row.load === 'loaded' ? 'success' : 'info'">
              {{ row.load }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="active" :label="t('common.status')" width="100" align="center">
          <template #default="{ row }">
            <el-tag size="small" :type="activeType(row.active)">
              {{ activeLabel(row.active) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column
          prop="sub"
          :label="t('serverServices.subState')"
          width="110"
          align="center"
        />
        <el-table-column :label="t('common.operation')" width="320" align="center" fixed="right">
          <template #default="{ row }">
            <el-button
              size="small"
              type="success"
              :loading="actingName === row.name"
              :disabled="row.active === 'active' || !!actingName"
              @click="doAction(row, 'start')"
            >
              {{ t('serverServices.start') }}
            </el-button>
            <el-button
              size="small"
              type="danger"
              :loading="actingName === row.name"
              :disabled="row.active !== 'active' || !!actingName"
              @click="doAction(row, 'stop')"
            >
              {{ t('serverServices.stop') }}
            </el-button>
            <el-button
              size="small"
              type="warning"
              :loading="actingName === row.name"
              :disabled="!!actingName"
              @click="doAction(row, 'restart')"
            >
              {{ t('serverServices.restart') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>
  </div>
</template>

<style scoped>
.services-container {
  padding: 0;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}
</style>
