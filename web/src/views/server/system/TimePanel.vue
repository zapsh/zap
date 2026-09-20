<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'

interface TimeInfo {
  datetime: string
  timestamp: number
  timezone: string
  timezone_offset: string
}

const { t } = useI18n()

const timeInfo = ref<TimeInfo | null>(null)
const timezones = ref<string[]>([])
const selectedTz = ref('')
const tzFilter = ref('')
const tzDialogVisible = ref(false)
const syncing = ref(false)

const filteredZones = computed(() => {
  if (!tzFilter.value) return timezones.value.slice(0, 200)
  const f = tzFilter.value.toLowerCase()
  return timezones.value.filter((z) => z.toLowerCase().includes(f)).slice(0, 200)
})

async function loadTime() {
  try {
    const res = await http.get<{ code: number; data: TimeInfo }>('/system/config/time')
    timeInfo.value = res.data
  } catch {
    /* handled */
  }
}

async function syncTime() {
  try {
    await ElMessageBox.confirm(t('serverTime.syncConfirm'), t('common.tip'), { type: 'info' })
  } catch {
    return
  }
  syncing.value = true
  try {
    const res = await http.post<{ code: number; message: string }>('/system/config/time/sync')
    ElMessage.success(res.message ?? t('serverTime.syncSuccess'))
    loadTime()
  } catch {
    /* handled */
  } finally {
    syncing.value = false
  }
}

async function loadTimezones() {
  try {
    const res = await http.get<{ code: number; data: string[] }>('/system/config/time/timezones')
    timezones.value = res.data ?? []
    tzDialogVisible.value = true
  } catch {
    /* handled */
  }
}

async function setTimezone() {
  if (!selectedTz.value) return
  try {
    await http.post('/system/config/time/timezone', { timezone: selectedTz.value })
    ElMessage.success(t('serverTime.tzSuccess'))
    tzDialogVisible.value = false
    loadTime()
  } catch {
    /* handled */
  }
}

onMounted(loadTime)
</script>

<template>
  <div class="time-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>{{ t('serverTime.title') }}</span>
          <el-tag size="small" type="info">{{ timeInfo?.timezone ?? '--' }}</el-tag>
        </div>
      </template>
      <el-descriptions v-if="timeInfo" :column="2" border>
        <el-descriptions-item :label="t('serverTime.currentTime')">
          {{ timeInfo.datetime }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('serverTime.timezone')">
          {{ timeInfo.timezone }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('serverTime.utcOffset')">
          {{ timeInfo.timezone_offset }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('serverTime.timestamp')">
          {{ timeInfo.timestamp }}
        </el-descriptions-item>
      </el-descriptions>
      <el-empty v-else :description="t('common.noData')" :image-size="60" />
      <div style="margin-top: 16px; display: flex; gap: 12px">
        <el-button type="primary" :loading="syncing" @click="syncTime">
          {{ t('serverTime.syncNtp') }}
        </el-button>
        <el-button @click="loadTimezones">{{ t('serverTime.changeTz') }}</el-button>
      </div>
    </el-card>

    <!-- 时区选择对话框 -->
    <el-dialog v-model="tzDialogVisible" :title="t('serverTime.selectTz')" width="500px">
      <el-input
        v-model="tzFilter"
        :placeholder="t('serverTime.searchTz')"
        clearable
        style="margin-bottom: 12px"
      />
      <el-scrollbar height="400px">
        <el-radio-group v-model="selectedTz" style="display: flex; flex-direction: column">
          <el-radio v-for="tz in filteredZones" :key="tz" :value="tz" style="margin-bottom: 4px">
            {{ tz }}
          </el-radio>
        </el-radio-group>
      </el-scrollbar>
      <template #footer>
        <el-button @click="tzDialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :disabled="!selectedTz" @click="setTimezone">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.time-container {
  padding: 0;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
</style>
