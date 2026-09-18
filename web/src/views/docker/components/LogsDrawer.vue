<template>
  <el-drawer v-model="visible" :title="`${t('docker.logs.title')} · ${containerName}`" size="60%">
    <template #header>
      <div class="logs-title">
        <span>{{ t('docker.logs.title') }}</span>
        <el-tag size="small" effect="plain">{{ containerName }}</el-tag>
      </div>
    </template>

    <div class="logs-toolbar">
      <el-select v-model="tail" size="small" :style="{ width: '120px' }">
        <el-option label="100" :value="100" />
        <el-option label="500" :value="500" />
        <el-option label="2000" :value="2000" />
        <el-option :label="t('docker.logs.sinceAll')" :value="0" />
      </el-select>
      <el-select v-model="since" size="small" :style="{ width: '140px' }">
        <el-option :label="t('docker.logs.sinceAll')" value="" />
        <el-option :label="t('docker.logs.since10m')" value="10m" />
        <el-option :label="t('docker.logs.since1h')" value="1h" />
        <el-option :label="t('docker.logs.since24h')" value="24h" />
      </el-select>
      <el-checkbox v-model="timestamps" size="small">{{ t('docker.logs.timestamps') }}</el-checkbox>
      <el-checkbox v-model="follow" size="small">{{ t('docker.logs.follow') }}</el-checkbox>
      <div class="logs-toolbar__spacer" />
      <el-button size="small" :icon="Refresh" @click="load">{{ t('docker.refresh') }}</el-button>
      <el-button size="small" :icon="Copy" :disabled="!log" @click="copyLog">
        {{ t('docker.logs.copy') }}
      </el-button>
    </div>

    <pre v-loading="loading" class="log-box">{{ log || (loading ? '' : t('docker.logs.empty')) }}</pre>
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Copy, Refresh } from '@/icons'
import { containerLogs } from '@/api/docker'

const props = defineProps<{ modelValue: boolean; containerId: string; containerName: string }>()
const emit = defineEmits<{ 'update:modelValue': [boolean] }>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const loading = ref(false)
const log = ref('')
const tail = ref(500)
const since = ref('')
const timestamps = ref(true)
const follow = ref(false)

let timer: ReturnType<typeof setInterval> | undefined

async function load() {
  if (!props.containerId) return
  loading.value = true
  try {
    const resp = await containerLogs({
      id: props.containerId,
      tail: tail.value || undefined,
      since: since.value || undefined,
      timestamps: timestamps.value,
    })
    log.value = resp.data.log || ''
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.logs.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function copyLog() {
  try {
    await navigator.clipboard.writeText(log.value)
    ElMessage.success(t('docker.inspect.copied'))
  } catch {
    ElMessage.warning(t('docker.common.actionFailed'))
  }
}

function startPoll() {
  stopPoll()
  timer = setInterval(load, 3000)
}

function stopPoll() {
  if (timer) {
    clearInterval(timer)
    timer = undefined
  }
}

watch(visible, (v) => {
  if (v) {
    log.value = ''
    load()
    if (follow.value) startPoll()
  } else {
    stopPoll()
  }
})

watch([tail, since, timestamps], () => {
  if (visible.value) load()
})

watch(follow, (v) => {
  if (!visible.value) return
  if (v) startPoll()
  else stopPoll()
})

onBeforeUnmount(stopPoll)
</script>

<style scoped>
.logs-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
}

.logs-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.logs-toolbar__spacer {
  flex: 1;
}

.log-box {
  margin: 0;
  padding: 12px;
  min-height: 320px;
  max-height: calc(100vh - 220px);
  overflow: auto;
  background: #1e1e1e;
  color: #d4d4d4;
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  border-radius: 6px;
}
</style>
