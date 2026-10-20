<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="secret-env">
    <el-alert
      type="warning"
      :closable="false"
      show-icon
      :title="t('automationSecrets.warn')"
    />
    <el-card class="block" shadow="never">
      <template #header>
        <div class="block-h">
          <span>{{ t('automationSecrets.keyList') }}</span>
          <el-button size="small" :icon="Refresh" @click="loadKeys">{{ t('common.refresh') }}</el-button>
        </div>
      </template>
      <el-table :data="keys" size="small" stripe empty-text="—">
        <el-table-column prop="key" :label="t('automationSecrets.key')" />
        <el-table-column :label="t('common.operation')" width="120" align="right">
          <template #default="{ row }">
            <el-button size="small" type="danger" link @click="del(row.key)">
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <el-card class="block" shadow="never">
      <template #header>
        <div class="block-h">
          <span>{{ t('automationSecrets.editor') }}</span>
          <el-button size="small" type="primary" :loading="saving" @click="save">
            {{ t('common.save') }}
          </el-button>
        </div>
      </template>
      <el-input
        v-model="content"
        type="textarea"
        :rows="12"
        :placeholder="t('automationSecrets.placeholder')"
      />
      <div class="hint">{{ t('automationSecrets.saveHint') }}</div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { getSecrets, saveSecrets, deleteSecret } from '@/api/appstore'

const { t } = useI18n()
const keys = ref<{ key: string }[]>([])
const content = ref('')
const saving = ref(false)

async function loadKeys() {
  try {
    const r = await getSecrets()
    keys.value = (r.data?.keys || []).map((k: any) => ({ key: k.key }))
  } catch {
    /* 拦截器已弹错 */
  }
}

async function save() {
  saving.value = true
  try {
    await saveSecrets({ content: content.value })
    ElMessage.success(t('common.saved'))
    await loadKeys()
  } catch {
    /* 拦截器已弹错 */
  } finally {
    saving.value = false
  }
}

async function del(key: string) {
  try {
    await ElMessageBox.confirm(
      t('automationSecrets.confirmDel', { key }),
      t('common.tip'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteSecret({ key })
    ElMessage.success(t('common.deleteSuccess'))
    await loadKeys()
  } catch {
    /* 拦截器已弹错 */
  }
}

onMounted(loadKeys)
</script>

<style scoped>
.secret-env {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.block-h {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}
.hint {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  white-space: pre-line;
}
</style>
