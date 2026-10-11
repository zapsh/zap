<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- 单条 nginx 指令行：label 就是指令名，空值 = 不生成该指令 -->
<template>
  <el-form-item :label="field.label">
    <template #label>
      <span class="dir-label mono">{{ field.label }}</span>
      <el-tooltip v-if="field.tip" :content="field.tip" placement="top">
        <el-icon class="dir-tip"><InfoFilled /></el-icon>
      </el-tooltip>
    </template>

    <el-input-number
      v-if="field.type === 'num'"
      :model-value="numVal"
      :min="0"
      controls-position="right"
      class="w280"
      @update:model-value="onNum"
    />
    <TriBool
      v-else-if="field.type === 'bool'"
      :model-value="boolVal"
      @update:model-value="(v: boolean | null) => set(v)"
    />
    <el-input
      v-else-if="field.type === 'area'"
      :model-value="strVal"
      type="textarea"
      :rows="2"
      class="mono"
      @update:model-value="(v: string | number) => set(v)"
    />
    <el-input
      v-else
      :model-value="strVal"
      class="mono"
      :placeholder="t('statusNginx.omitPlaceholder')"
      @update:model-value="(v: string | number) => set(v)"
    />
  </el-form-item>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { InfoFilled } from '@/icons'
import TriBool from './TriBool.vue'

const { t } = useI18n()

const props = defineProps<{
  field: { key: string; label: string; type: 'text' | 'num' | 'bool' | 'area'; tip?: string }
  model: Record<string, unknown>
}>()

const emit = defineEmits<{ (e: 'change'): void }>()

const raw = computed(() => props.model?.[props.field.key])

const strVal = computed(() => {
  const v = raw.value
  return v === null || v === undefined ? '' : String(v)
})

const boolVal = computed<boolean | null>(() => {
  const v = raw.value
  return typeof v === 'boolean' ? v : null
})

const numVal = computed<number | undefined>(() => {
  const v = raw.value
  return typeof v === 'number' ? v : undefined
})

/** 空串一律转成 null —— 渲染层遇到 null 就不输出这条指令 */
function set(v: unknown) {
  const empty = v === null || v === undefined || (typeof v === 'string' && v.trim() === '')
  props.model[props.field.key] = empty ? null : v
  emit('change')
}

function onNum(v: number | null | undefined) {
  set(typeof v === 'number' && Number.isFinite(v) ? v : null)
}
</script>

<style scoped>
.dir-label {
  font-size: 12px;
}
.dir-tip {
  margin-left: 3px;
  color: var(--el-text-color-secondary);
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
  font-size: 12px;
}
.w280 {
  width: 200px;
}
</style>
