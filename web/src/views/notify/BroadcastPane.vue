<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="broadcast-pane">
    <el-card shadow="never" class="block">
      <template #header>{{ t('notifyCfg.broadcastTitle') }}</template>

      <el-form label-width="88px" label-position="right">
        <el-form-item :label="t('notifyCfg.tplSelect')">
          <el-select v-model="tplKey" :placeholder="t('notifyCfg.tplNone')" clearable style="width: 280px">
            <el-option :value="''" :label="t('notifyCfg.tplNone')" />
            <el-option v-for="o in tplOptions" :key="o.value" :value="o.value" :label="o.label" />
          </el-select>
        </el-form-item>

        <el-form-item :label="t('notifyCfg.channel')">
          <el-radio-group v-model="channel">
            <el-radio value="inbox">{{ t('notifyCfg.channelInbox') }}</el-radio>
            <el-radio value="email">{{ t('notifyCfg.channelEmail') }}</el-radio>
            <el-radio value="both">{{ t('notifyCfg.channelBoth') }}</el-radio>
          </el-radio-group>
        </el-form-item>

        <el-form-item :label="t('notifyCfg.subject')" required>
          <el-input v-model="subject" :placeholder="t('notifyCfg.subject')" maxlength="200" show-word-limit />
        </el-form-item>

        <el-form-item v-if="channel !== 'inbox'" :label="t('notifyCfg.format')">
          <el-radio-group v-model="isHtml">
            <el-radio :value="false">{{ t('notifyCfg.formatText') }}</el-radio>
            <el-radio :value="true">{{ t('notifyCfg.formatHtml') }}</el-radio>
          </el-radio-group>
        </el-form-item>

        <el-form-item :label="t('notifyCfg.body')">
          <div class="editor-wrap">
            <CodeEditor
              :model-value="isHtml ? bodyHtml : bodyText"
              :lang="isHtml ? 'html' : 'text'"
              :placeholder="t('notifyCfg.body')"
              @update:model-value="onBodyInput"
            />
          </div>
        </el-form-item>
      </el-form>
    </el-card>

    <el-card shadow="never" class="block">
      <template #header>
        <div class="recipients-head">
          <span>{{ t('notifyCfg.recipients') }}</span>
          <span class="count">{{ t('notifyCfg.selectedCount', { count: selectedIds.length }) }}</span>
        </div>
      </template>

      <div v-if="recipientsLoading" class="loading">{{ t('notifyCfg.sending') }}</div>
      <el-table
        v-else
        ref="tableRef"
        :data="recipients"
        height="320"
        @selection-change="onSelectionChange"
      >
        <el-table-column type="selection" width="46" />
        <el-table-column prop="username" :label="t('notifyCfg.recipients')" min-width="140" />
        <el-table-column label="Email" min-width="220">
          <template #default="{ row }">{{ row.email || '—' }}</template>
        </el-table-column>
        <el-table-column prop="nickname" label="" min-width="120" />
      </el-table>
      <div class="hint">{{ t('notifyCfg.recipientHint') }}</div>

      <div class="actions">
        <el-button type="primary" :loading="sending" @click="send">
          {{ sending ? t('notifyCfg.sending') : t('notifyCfg.send') }}
        </el-button>
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox, type TableInstance } from 'element-plus'
import CodeEditor from '@/components/CodeEditor.vue'
import { getMailTemplates, broadcastMail } from '@/api/systemNotify'
import { getUserList } from '@/api/user'
import type { UserListItem } from '@/api/user'

const { t } = useI18n()

const tplOptions = [
  { value: 'login_success', label: t('notifyCfg.evLogin') },
  { value: 'password_change', label: t('notifyCfg.evPassword') },
  { value: 'site_created', label: t('notifyCfg.evSiteCreated') },
  { value: 'disk_low', label: t('notifyCfg.evDiskLow') },
]

const tplKey = ref('')
const subject = ref('')
const channel = ref<'inbox' | 'email' | 'both'>('both')
const isHtml = ref(false)
const bodyText = ref('')
const bodyHtml = ref('')

// 仅站内信渠道时正文恒为纯文本，强制关掉 HTML 选项
watch(channel, (v) => {
  if (v === 'inbox') isHtml.value = false
})

function onBodyInput(v: string) {
  if (isHtml.value) bodyHtml.value = v
  else bodyText.value = v
}

// 选模板预填：取到该事件的主题 + 正文（含 HTML 标记与纯文本兜底）
watch(tplKey, async (key) => {
  if (!key) {
    subject.value = ''
    bodyText.value = ''
    bodyHtml.value = ''
    isHtml.value = false
    return
  }
  try {
    const { data } = await getMailTemplates()
    const ev = data.events[key]
    if (ev) {
      subject.value = ev.subject
      isHtml.value = ev.is_html
      bodyText.value = ev.body_text
      bodyHtml.value = ev.body_html
    }
  } catch {
    /* 取模板失败不影响自由撰写 */
  }
})

const recipients = ref<UserListItem[]>([])
const recipientsLoading = ref(false)
const selectedIds = ref<number[]>([])
const tableRef = ref<TableInstance>()

function onSelectionChange(rows: UserListItem[]) {
  selectedIds.value = rows.map((r) => r.id)
}

async function loadRecipients() {
  recipientsLoading.value = true
  try {
    const { data } = await getUserList()
    recipients.value = data ?? []
  } finally {
    recipientsLoading.value = false
  }
}

const sending = ref(false)
async function send() {
  if (!subject.value.trim()) {
    ElMessage.warning(t('notifyCfg.emptySubject'))
    return
  }
  if (selectedIds.value.length === 0) {
    ElMessage.warning(t('notifyCfg.noRecipient'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('notifyCfg.confirmSend', { count: selectedIds.value.length }),
      t('notifyCfg.broadcastTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  sending.value = true
  try {
    const { data } = await broadcastMail({
      subject: subject.value.trim(),
      body_text: bodyText.value,
      body_html: bodyHtml.value || undefined,
      is_html: isHtml.value,
      channel: channel.value,
      user_ids: selectedIds.value,
    })
    const parts: string[] = []
    if (channel.value !== 'inbox') {
      parts.push(t('notifyCfg.resultMail', { email: data.sent_email }))
    }
    if (channel.value !== 'email') {
      parts.push(t('notifyCfg.resultInbox', { inbox: data.sent_inbox }))
    }
    const msg = parts.join('；')
    const failed = data.failed.length
    if (failed > 0) {
      ElMessage.warning(`${msg}；${t('notifyCfg.resultFailed', { n: failed })}`)
    } else {
      ElMessage.success(msg)
    }
  } catch {
    /* 错误已由拦截器提示 */
  } finally {
    sending.value = false
  }
}

onMounted(loadRecipients)
</script>

<style scoped>
.broadcast-pane {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.block :deep(.el-card__header) {
  font-weight: 600;
}
.recipients-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.recipients-head .count {
  font-weight: 400;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.editor-wrap {
  width: 100%;
  height: 240px;
  border: 1px solid var(--el-border-color);
  border-radius: 4px;
  overflow: hidden;
}
.hint {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.actions {
  margin-top: 16px;
  text-align: right;
}
.loading {
  padding: 40px 0;
  text-align: center;
  color: var(--el-text-color-secondary);
}
</style>
