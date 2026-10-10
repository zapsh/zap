<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-dialog
    v-model="visible"
    :title="t('prefs.title')"
    width="520px"
    append-to-body
    @open="syncFromStore"
  >
    <div class="pref-section">{{ t('prefs.keepAliveTitle') }}</div>
    <div class="pref-hint">{{ t('prefs.keepAliveHint') }}</div>

    <el-checkbox-group v-model="selected" class="pref-pages">
      <el-checkbox v-for="c in ALIVE_CANDIDATES" :key="c.path" :value="c.path">
        {{ t(c.labelKey) }}
        <!-- 默认两项给个小标记，方便用户认出出厂配置 -->
        <span v-if="DEFAULT_ALIVE_PATHS.includes(c.path)" class="pref-default">{{
          t('prefs.defaultTag')
        }}</span>
      </el-checkbox>
    </el-checkbox-group>

    <template #footer>
      <el-button @click="visible = false">{{ t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="saving" @click="save">
        {{ t('common.save') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { useTagsStore, ALIVE_CANDIDATES, DEFAULT_ALIVE_PATHS } from '@/stores/tags'

const { t } = useI18n()
const tagsStore = useTagsStore()

const visible = defineModel<boolean>({ default: false })

/** 弹层内的临时勾选：打开时从 store 拷贝，保存时才写回 */
const selected = ref<string[]>([])
const saving = ref(false)

function syncFromStore() {
  selected.value = [...tagsStore.alivePaths]
}

function save() {
  saving.value = true
  try {
    tagsStore.setAlivePages(selected.value)
    visible.value = false
    ElMessage.success(t('prefs.saved'))
  } finally {
    saving.value = false
  }
}
</script>

<style lang="scss" scoped>
.pref-section {
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  margin-bottom: 6px;
}

.pref-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.6;
  margin-bottom: 12px;
}

.pref-pages {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  max-height: 320px;
  overflow-y: auto;
  padding: 4px 2px;
}

.pref-default {
  margin-left: 6px;
  font-size: 11px;
  color: var(--el-color-primary);
  border: 1px solid var(--el-color-primary-light-5);
  border-radius: 3px;
  padding: 0 4px;
  line-height: 16px;
}
</style>
