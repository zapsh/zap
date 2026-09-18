<template>
  <el-drawer v-model="visible" :title="`${t('docker.inspect.title')} · ${containerName}`" size="55%">
    <div class="inspect-toolbar">
      <el-button size="small" :icon="Copy" :disabled="!json" @click="copyJson">
        {{ t('docker.inspect.copy') }}
      </el-button>
    </div>
    <pre v-loading="loading" class="inspect-json">{{ json || (loading ? '' : t('docker.common.empty')) }}</pre>
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Copy } from '@/icons'
import { containerInspect } from '@/api/docker'

const props = defineProps<{ modelValue: boolean; containerId: string; containerName: string }>()
const emit = defineEmits<{ 'update:modelValue': [boolean] }>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const loading = ref(false)
const json = ref('')

async function load() {
  if (!props.containerId) return
  loading.value = true
  try {
    const resp = await containerInspect(props.containerId)
    json.value = JSON.stringify(resp.data ?? {}, null, 2)
  } catch (e: any) {
    json.value = ''
    ElMessage.error(e.message || t('docker.common.loadFailed'))
  } finally {
    loading.value = false
  }
}

async function copyJson() {
  try {
    await navigator.clipboard.writeText(json.value)
    ElMessage.success(t('docker.inspect.copied'))
  } catch {
    ElMessage.warning(t('docker.common.actionFailed'))
  }
}

watch(visible, (v) => {
  if (v) {
    json.value = ''
    load()
  }
})
</script>

<style scoped>
.inspect-toolbar {
  margin-bottom: 12px;
}

.inspect-json {
  margin: 0;
  padding: 12px;
  max-height: calc(100vh - 200px);
  overflow: auto;
  background: var(--el-fill-color-light);
  border-radius: 6px;
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.6;
}
</style>
