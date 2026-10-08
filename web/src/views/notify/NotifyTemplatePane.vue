<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { reactive, ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { getMailTemplates, saveMailTemplates } from '@/api/systemNotify'
import type { MailTemplate } from '@/api/systemNotify'
import CodeEditor from '@/components/CodeEditor.vue'

const { t } = useI18n()
const loading = ref(false)
const saving = ref(false)
const scope = ref<'global' | 'self'>('global')
const activeEvent = ref('')

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
  Object.fromEntries(
    eventDefs.map((e) => [
      e.key,
      { subject: '', body_text: '', body_html: '', is_html: false, template_id: '' },
    ]),
  ),
)

// 正文双向绑定：根据当前格式在 body_text / body_html 之间切换
function bodyValue(key: string): string {
  const tpl = templates[key]
  return tpl.is_html ? tpl.body_html : tpl.body_text
}

function onBodyInput(key: string, v: string) {
  const tpl = templates[key]
  if (tpl.is_html) tpl.body_html = v
  else tpl.body_text = v
}

async function load() {
  loading.value = true
  try {
    const res = await getMailTemplates()
    scope.value = res.data.scope
    activeEvent.value = eventDefs[0].key
    for (const ev of eventDefs) {
      const tpl = res.data.events[ev.key] ?? {
        subject: '',
        body_text: '',
        body_html: '',
        is_html: false,
      }
      templates[ev.key] = {
        subject: tpl.subject ?? '',
        body_text: tpl.body_text ?? '',
        body_html: tpl.body_html ?? '',
        is_html: tpl.is_html ?? false,
      }
    }
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    loading.value = false
  }
}

// 切换到 HTML 且 HTML 正文为空时，用纯文本正文预填，减少重复输入
function onFormatChange(key: string, val: boolean | string | number) {
  if (val && !templates[key].body_html.trim()) {
    templates[key].body_html = templates[key].body_text
  }
}

async function save() {
  saving.value = true
  try {
    const payload: Record<string, MailTemplate> = {}
    for (const ev of eventDefs) {
      const tpl = templates[ev.key] ?? {
        subject: '',
        body_text: '',
        body_html: '',
        is_html: false,
        template_id: '',
      }
      payload[ev.key] = {
        subject: (tpl.subject ?? '').trim(),
        body_text: (tpl.body_text ?? '').trim(),
        body_html: (tpl.body_html ?? '').trim(),
        is_html: !!tpl.is_html,
        template_id: (tpl.template_id ?? '').trim(),
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
      <el-tabs v-model="activeEvent" class="tpl-tabs" type="border-card">
        <el-tab-pane
          v-for="ev in eventDefs"
          :key="ev.key"
          :name="ev.key"
          :label="ev.label"
          lazy
        >
          <div class="tpl-vars">{{ t('notifyCfg.varHint', { vars: ev.vars }) }}</div>

          <label class="tpl-label">{{ t('notifyCfg.templateSubjectLabel') }}</label>
          <el-input
            v-model="templates[ev.key].subject"
            :placeholder="t('notifyCfg.templateSubject')"
            style="margin-bottom: 12px"
          />

          <label class="tpl-label">{{ t('notifyCfg.templateIdLabel') }}</label>
          <el-input
            v-model="templates[ev.key].template_id"
            :placeholder="t('notifyCfg.templateIdPlaceholder')"
            style="margin-bottom: 4px"
          />
          <div class="tpl-vars">{{ t('notifyCfg.templateIdHint') }}</div>

          <div class="tpl-format">
            <span class="tpl-format-label">{{ t('notifyCfg.templateFormat') }}</span>
            <el-switch
              v-model="templates[ev.key].is_html"
              :active-text="t('notifyCfg.templateHtml')"
              :inactive-text="t('notifyCfg.templateText')"
              @change="(v: boolean | string | number) => onFormatChange(ev.key, v)"
            />
          </div>

          <label class="tpl-label">{{ t('notifyCfg.templateBodyLabel') }}</label>
          <div class="tpl-editor-wrap">
            <CodeEditor
              :model-value="bodyValue(ev.key)"
              @update:model-value="(v: string) => onBodyInput(ev.key, v)"
              :lang="templates[ev.key].is_html ? 'html' : 'text'"
              :placeholder="
                templates[ev.key].is_html
                  ? t('notifyCfg.templateBodyHtml')
                  : t('notifyCfg.templateBody')
              "
              :active="activeEvent === ev.key"
            />
          </div>
        </el-tab-pane>
      </el-tabs>

      <el-button type="primary" :loading="saving" style="margin-top: 12px" @click="save">
        {{ t('notifyCfg.saveTemplates') }}
      </el-button>
    </div>
  </div>
</template>

<style scoped>
.tpl-tabs {
  margin-top: 12px;
}
.tpl-vars {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  margin-bottom: 12px;
}
.tpl-label {
  display: block;
  color: var(--el-text-color-regular);
  font-size: 13px;
  margin-bottom: 6px;
}
.tpl-format {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.tpl-format-label {
  color: var(--el-text-color-regular);
  font-size: 13px;
}
.tpl-editor-wrap {
  height: 260px;
  border: 1px solid var(--el-border-color);
  border-radius: 6px;
  overflow: hidden;
}
</style>
