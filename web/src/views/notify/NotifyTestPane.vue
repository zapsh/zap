<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { sendTestMail } from '@/api/systemNotify'

const { t } = useI18n()
const to = ref('')
const sending = ref(false)

async function send() {
  if (!to.value.trim() || !to.value.includes('@')) {
    ElMessage.warning(t('notifyCfg.testMailPlaceholder'))
    return
  }
  sending.value = true
  try {
    await sendTestMail(to.value.trim())
    ElMessage.success(t('notifyCfg.testSent'))
  } catch (err) {
    ElMessage.error((err as Error)?.message || t('error.system'))
  } finally {
    sending.value = false
  }
}
</script>

<template>
  <div>
    <el-alert
      type="warning"
      :closable="false"
      :title="t('notifyCfg.testMailHint')"
      style="margin-bottom: 12px"
    />
    <el-form label-width="160px" style="max-width: 720px" @submit.prevent>
      <el-form-item :label="t('notifyCfg.testMailTo')">
        <el-input
          v-model="to"
          :placeholder="t('notifyCfg.testMailPlaceholder')"
          style="max-width: 360px"
        />
      </el-form-item>
      <el-form-item>
        <el-button type="primary" :loading="sending" @click="send">
          {{ t('notifyCfg.sendTest') }}
        </el-button>
      </el-form-item>
    </el-form>
  </div>
</template>
