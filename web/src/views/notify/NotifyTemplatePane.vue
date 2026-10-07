<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { reactive, ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { getMailTemplates, saveMailTemplates } from '@/api/systemNotify'
import type { MailTemplate } from '@/api/systemNotify'

const { t } = useI18n()
const loading = ref(false)
const saving = ref(false)
const scope = ref<'global' | 'self'>('global')

const eventDefs = [
  {
    key: 'login_success',
    label: t('notifyCfg.evLogin'),
    vars: '{{username}} {{time}} {{ip}}',
  },
  {
    key: 'password_change',
    label: t('notifyCfg.evPassword'),
    vars: '{{time}} {{operator}}',
  },
  {
    key: 'site_created',
    label: t('notifyCfg.evSiteCreated'),
    vars: '{{site}} {{time}}',
  },
  {
    key: 'disk_low',
    label: t('notifyCfg.evDiskLow'),
    vars: '{{pct}}',
  },
]
// 预先用各事件键初始化，避免首次渲染时 templates[ev.key] 为 undefined
const templates = reactive<Record<string, MailTemplate>>(
  Object.fromEntries(eventDefs.map((e) => [e.key, { subject: '', body: '' }])),
)

async function load() {
  loading.value = true
  try {
    const res = await getMailTemplates()
    scope.value = res.data.scope
    for (const ev of eventDefs) {
      const tpl = res.data.events[ev.key] ?? { subject: '', body: '' }
      templates[ev.key] = { subject: tpl.subject ?? '', body: tpl.body ?? '' }
    }
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    loading.value = false
  }
}

async function save() {
  saving.value = true
  try {
    const payload: Record<string, MailTemplate> = {}
    for (const ev of eventDefs) {
      payload[ev.key] = {
        subject: (templates[ev.key]?.subject ?? '').trim(),
        body: (templates[ev.key]?.body ?? '').trim(),
      }
    }
    await saveMailTemplates(payload)
    ElMessage.success(t('notifyCfg.templatesSaved'))
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<template>
  <div>
    <el-alert
      type="info"
      :closable="false"
      show-icon
      :title="t('notifyCfg.templatesHint')"
      style="margin-bottom: 12px"
    />
    <el-tag v-if="scope === 'global'" type="primary" effect="plain">
      {{ t('notifyCfg.templateScopeGlobal') }}
    </el-tag>
    <el-tag v-else type="warning" effect="plain">
      {{ t('notifyCfg.templateScopeSelf') }}
    </el-tag>

    <div v-loading="loading" style="margin-top: 12px">
      <div v-for="ev in eventDefs" :key="ev.key" class="tpl-block">
        <div class="tpl-head">
          <strong>{{ ev.label }}</strong>
          <span class="tpl-vars">{{ t('notifyCfg.varHint', { vars: ev.vars }) }}</span>
        </div>
        <el-input
          v-model="templates[ev.key].subject"
          :placeholder="t('notifyCfg.templateSubject')"
          style="margin-bottom: 8px"
        />
        <el-input
          v-model="templates[ev.key].body"
          type="textarea"
          :rows="3"
          :placeholder="t('notifyCfg.templateBody')"
        />
      </div>
      <el-button
        type="primary"
        :loading="saving"
        style="margin-top: 12px"
        @click="save"
      >
        {{ t('notifyCfg.saveTemplates') }}
      </el-button>
    </div>
  </div>
</template>

<style scoped>
.tpl-block {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  padding: 12px 14px;
  margin-bottom: 12px;
}
.tpl-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin-bottom: 8px;
  flex-wrap: wrap;
}
.tpl-vars {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
</style>
