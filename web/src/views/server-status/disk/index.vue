<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div>
    <el-card shadow="hover">
      <template #header>
        <div class="card-header">
          <span>{{ t('statusDisk.title') }}</span>
          <el-button size="small" :icon="Refresh" circle @click="loadDisks" />
        </div>
      </template>
      <el-table :data="disks" v-loading="loading" border stripe>
        <el-table-column prop="name" :label="t('statusDisk.device')" width="180" />
        <el-table-column prop="file_system" :label="t('statusDisk.fileSystem')" width="120" />
        <el-table-column prop="mount_point" :label="t('statusDisk.mountPoint')" width="180" />
        <el-table-column :label="t('statusDisk.capacity')" width="120">
          <template #default="{ row }">{{ formatBytes(row.total_space) }}</template>
        </el-table-column>
        <el-table-column :label="t('statusDisk.used')" width="120">
          <template #default="{ row }">{{
            formatBytes(row.total_space - row.available_space)
          }}</template>
        </el-table-column>
        <el-table-column :label="t('statusDisk.available')" width="120">
          <template #default="{ row }">{{ formatBytes(row.available_space) }}</template>
        </el-table-column>
        <el-table-column :label="t('statusDisk.usage')" min-width="220">
          <template #default="{ row }">
            <el-progress
              :percentage="usagePct(row)"
              :color="pctColor(usagePct(row))"
              :stroke-width="12"
              text-inside
            />
          </template>
        </el-table-column>
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import { getSystemInfo } from '@/api/dashboard.ts'
import { formatBytes } from '@/utils/fmt'
import { isArray } from '@/utils/validate'

const { t } = useI18n()

const loading = ref(false)
const disks = ref<any[]>([])
let timer: ReturnType<typeof setInterval> | undefined

onMounted(async () => {
  await loadDisks()
  timer = setInterval(loadDisks, 5000)
})

onUnmounted(() => {
  if (timer) clearInterval(timer)
})

const usagePct = (row: Record<string, any>) => {
  if (!row.total_space) return 0
  return parseFloat((((row.total_space - row.available_space) / row.total_space) * 100).toFixed(1))
}

const pctColor = (p: number) => (p > 90 ? '#f56c6c' : p > 70 ? '#e6a23c' : '#67c23a')

const loadDisks = async () => {
  loading.value = true
  try {
    const resp = await getSystemInfo()
    if (resp.code === 0 && isArray(resp.data.disk_info)) {
      disks.value = resp.data.disk_info
    }
  } finally {
    loading.value = false
  }
}
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
