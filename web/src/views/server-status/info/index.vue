<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div>
    <!-- 第一行：系统 + 处理器 -->
    <el-row :gutter="20">
      <el-col :xs="24" :sm="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ t('statusInfo.system') }}</span>
              <span class="card-header-icon"
                ><el-icon><Monitor /></el-icon
              ></span>
            </div>
          </template>
          <div class="kv-grid">
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.hostname') }}</span
              ><span class="kv-value">{{ info.host_name || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.os') }}</span
              ><span class="kv-value">{{ info.os_name || '-' }} {{ info.os_version || '' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.kernel') }}</span
              ><span class="kv-value">{{ info.kernel_version || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.arch') }}</span
              ><span class="kv-value">{{ info.arch || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.vendor') }}</span
              ><span class="kv-value">{{ info.vendor || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.product') }}</span
              ><span class="kv-value">{{ info.product || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.uptime') }}</span
              ><span class="kv-value">{{ info.uptime || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.bootTime') }}</span
              ><span class="kv-value">{{ info.boot_time || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.currentTime') }}</span
              ><span class="kv-value">{{ info.current_time || '-' }}</span>
            </div>
          </div>
        </el-card>
      </el-col>
      <el-col :xs="24" :sm="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ t('statusInfo.processor') }}</span>
              <span class="card-header-icon"
                ><el-icon><Cpu /></el-icon
              ></span>
            </div>
          </template>
          <div class="kv-grid">
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.model') }}</span
              ><span class="kv-value cpu-model">{{ info.cpu?.model || '-' }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.physicalCores') }}</span
              ><span class="kv-value">{{
                t('statusInfo.coreCount', { n: info.cpu?.physical_cores ?? '-' })
              }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.logicalCores') }}</span
              ><span class="kv-value">{{
                t('statusInfo.coreCount', { n: info.cpu?.logical_cores ?? '-' })
              }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.frequency') }}</span
              ><span class="kv-value">{{ fmtFreq(info.cpu?.frequency_mhz) }}</span>
            </div>
            <div class="kv-item">
              <span class="kv-label">{{ t('statusInfo.usage') }}</span>
              <span class="kv-value">
                <el-progress
                  :percentage="usagePct(info.cpu?.usage)"
                  :color="usageColor(info.cpu?.usage)"
                  :stroke-width="10"
                />
              </span>
            </div>
          </div>
          <div class="load-row">
            <div class="load-cell">
              <div class="load-value">{{ fmtLoad(info.cpu?.loadavg_one) }}</div>
              <div class="load-label">Load Avg 1M</div>
            </div>
            <div class="load-cell">
              <div class="load-value">{{ fmtLoad(info.cpu?.loadavg_five) }}</div>
              <div class="load-label">Load Avg 5M</div>
            </div>
            <div class="load-cell">
              <div class="load-value">{{ fmtLoad(info.cpu?.loadavg_fifteen) }}</div>
              <div class="load-label">Load Avg 15M</div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 第二行：当前内存使用 -->
    <el-row :gutter="20" class="mt-4">
      <el-col :xs="24">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ t('statusInfo.memTitle') }}</span>
              <span class="card-header-icon"
                ><el-icon><DataLine /></el-icon
              ></span>
            </div>
          </template>
          <div class="usage-block">
            <div class="usage-bar-row">
              <span class="usage-label">{{ t('statusInfo.memUsage') }}</span>
              <el-progress
                :percentage="info.memory_usage?.usage_pct ?? 0"
                :color="usageColor(info.memory_usage?.usage_pct)"
                :stroke-width="14"
              />
            </div>
            <div class="stat-row mt-2">
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.used || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.used') }}</div>
              </div>
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.available || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.available') }}</div>
              </div>
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.free || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.free') }}</div>
              </div>
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.total || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.total') }}</div>
              </div>
            </div>
          </div>
          <div class="swap-block mt-2">
            <div class="swap-title">{{ t('statusInfo.swap') }}</div>
            <div class="stat-row">
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.swap_used || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.used') }}</div>
              </div>
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.swap_free || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.free') }}</div>
              </div>
              <div class="stat-cell">
                <div class="stat-cell-value">
                  {{ formatBytes(info.memory_usage?.swap_total || 0, 1) }}
                </div>
                <div class="stat-cell-label">{{ t('statusInfo.swapTotal') }}</div>
              </div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 第三行：物理磁盘 + 当前磁盘使用 -->
    <el-row :gutter="20" class="mt-4">
      <el-col :xs="24" :sm="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ t('statusInfo.diskTitle') }}</span>
              <span class="card-header-icon"
                ><el-icon><Box /></el-icon
              ></span>
            </div>
          </template>
          <el-table
            v-if="info.physical_disks?.length"
            :data="info.physical_disks"
            size="small"
            border
            max-height="280"
          >
            <el-table-column prop="device" :label="t('statusInfo.device')" min-width="90" />
            <el-table-column :label="t('statusInfo.model')" min-width="140" show-overflow-tooltip>
              <template #default="{ row }">{{ row.model || '-' }}</template>
            </el-table-column>
            <el-table-column :label="t('statusInfo.capacity')" min-width="90">
              <template #default="{ row }">{{
                row.size ? formatBytes(row.size, 1) : '-'
              }}</template>
            </el-table-column>
            <el-table-column prop="interface" :label="t('statusInfo.iface')" min-width="90" />
            <el-table-column :label="t('statusInfo.diskType')" min-width="70">
              <template #default="{ row }">
                <el-tag :type="row.rotational ? 'warning' : 'success'" size="small">
                  {{ row.rotational ? 'HDD' : 'SSD' }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>
          <el-empty
            v-else
            :description="t('statusInfo.noPhysicalDisk')"
            :image-size="60"
            class="mt-2"
          />
        </el-card>
      </el-col>
      <el-col :xs="24" :sm="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ t('statusInfo.diskUsageTitle') }}</span>
              <span class="card-header-icon"
                ><el-icon><Folder /></el-icon
              ></span>
            </div>
          </template>
          <el-table
            v-if="info.disk_usage?.length"
            :data="info.disk_usage"
            size="small"
            border
            max-height="280"
          >
            <el-table-column
              prop="mount_point"
              :label="t('statusInfo.mountPoint')"
              min-width="90"
              show-overflow-tooltip
            />
            <el-table-column
              prop="file_system"
              :label="t('statusInfo.fileSystem')"
              min-width="80"
              show-overflow-tooltip
            />
            <el-table-column :label="t('statusInfo.capacity')" min-width="80">
              <template #default="{ row }">{{ formatBytes(row.total, 1) }}</template>
            </el-table-column>
            <el-table-column :label="t('statusInfo.usedAvailable')" min-width="120">
              <template #default="{ row }">
                <span class="disk-usage-text"
                  >{{ formatBytes(row.used, 1) }} / {{ formatBytes(row.available, 1) }}</span
                >
              </template>
            </el-table-column>
            <el-table-column :label="t('statusInfo.usageCol')" min-width="130">
              <template #default="{ row }">
                <el-progress
                  :percentage="usagePct(row.usage_pct)"
                  :color="usageColor(row.usage_pct)"
                  :stroke-width="8"
                />
              </template>
            </el-table-column>
          </el-table>
          <el-empty
            v-else
            :description="t('statusInfo.noDiskPartition')"
            :image-size="60"
            class="mt-2"
          />
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Monitor, Cpu, DataLine, Box, Folder } from '@/icons'
import { getSystemOverview } from '@/api/dashboard.ts'
import { formatBytes } from '@/utils/fmt.ts'

const { t } = useI18n()

const info = ref<Record<string, any>>({})

let timer: ReturnType<typeof setInterval> | undefined
let destroyed = false

const fmtLoad = (v: number) => (v == null ? '-' : v.toFixed(2))
const fmtFreq = (mhz: number) => (mhz ? `${(mhz / 1000).toFixed(2)} GHz` : '-')
const usagePct = (v: number) => (v == null || Number.isNaN(v) ? 0 : Math.round(v * 10) / 10)
const usageColor = (pct: number) => (pct >= 90 ? '#f56c6c' : pct >= 70 ? '#e6a23c' : '#67c23a')

const fetchOverview = async () => {
  const resp = await getSystemOverview()
  if (destroyed || resp.code !== 0) return
  info.value = resp.data
}

onMounted(async () => {
  await fetchOverview()
  if (destroyed) return
  timer = setInterval(fetchOverview, 5000)
})

onUnmounted(() => {
  destroyed = true
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.card-header-icon {
  font-size: 18px;
  color: var(--el-text-color-secondary);
}
.kv-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px 20px;
}
.kv-item {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.kv-label {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 2px;
}
.kv-value {
  font-size: 14px;
  color: var(--el-text-color-primary);
  word-break: break-all;
}
.cpu-model {
  font-weight: 600;
}
.load-row {
  display: flex;
  margin-top: 16px;
  border-top: 1px solid var(--el-border-color-lighter);
  padding-top: 14px;
}
.load-cell {
  flex: 1;
  text-align: center;
}
.load-value {
  font-size: 22px;
  font-weight: bold;
  color: var(--el-color-primary);
}
.load-label {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.usage-block {
  padding: 4px 0;
}
.usage-bar-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.usage-label {
  font-size: 14px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
}
.stat-row {
  display: flex;
  text-align: center;
}
.stat-cell {
  flex: 1;
}
.stat-cell-value {
  font-size: 17px;
  font-weight: 600;
}
.stat-cell-label {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.swap-block {
  border-top: 1px solid var(--el-border-color-lighter);
  padding-top: 12px;
}
.swap-title {
  font-size: 13px;
  color: var(--el-text-color-secondary);
  margin-bottom: 10px;
}
.disk-usage-text {
  font-size: 12px;
  color: var(--el-text-color-regular);
}
.mt-2 {
  margin-top: 12px;
}
.mt-4 {
  margin-top: 16px;
}
</style>
