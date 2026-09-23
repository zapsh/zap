<template>
  <el-radio-group :model-value="modelValue" size="small" @update:model-value="onChange">
    <el-radio-button v-for="r in MONITOR_RANGES" :key="r.value" :value="r.value">
      {{ t(`monitorRange.${r.value}`) }}
    </el-radio-button>
  </el-radio-group>
</template>

<script setup lang="ts">
/**
 * 监控图表的时间范围选择器（实时 / 1h / 6h / 24h / 7d / 30d）。
 * 服务器状态各页共用，配合 useMonitorRange 使用。
 */
import { useI18n } from 'vue-i18n'
import { MONITOR_RANGES, type MonitorRangeValue } from '@/composables/useMonitorRange'

defineProps<{ modelValue: string }>()
const emit = defineEmits<{ (e: 'update:modelValue', v: MonitorRangeValue): void }>()
const { t } = useI18n()

function onChange(v: MonitorRangeValue | string) {
  emit('update:modelValue', v as MonitorRangeValue)
}
</script>
