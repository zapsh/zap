<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- 三态布尔：on / off / 不生成（null）—— 不生成时不在 nginx.conf 里输出该指令，
     用户可以在 conf.d 里自己维护这条，避免 duplicate directive。 -->
<template>
  <el-select v-model="inner" size="small" class="tri">
    <el-option :label="t('statusNginx.triOmit')" value="none" />
    <el-option label="on" :value="'true'" />
    <el-option label="off" :value="'false'" />
  </el-select>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()

const props = defineProps<{ modelValue?: boolean | null }>()
const emit = defineEmits<{ (e: 'update:modelValue', v: boolean | null): void }>()

const inner = computed({
  get: () => (props.modelValue === true ? 'true' : props.modelValue === false ? 'false' : 'none'),
  set: (v: string) => {
    emit('update:modelValue', v === 'true' ? true : v === 'false' ? false : null)
  },
})
</script>

<style scoped>
.tri {
  width: 130px;
}
</style>
