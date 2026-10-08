<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="broadcast-pane">
    <el-card shadow="never" class="block">
      <template #header>{{ t('notifyCfg.broadcastTitle') }}</template>

      <el-form label-width="88px" label-position="right">
        <el-form-item :label="t('notifyCfg.tplSelect')">
          <div class="tpl-row">
            <el-select v-model="tplKey" :placeholder="t('notifyCfg.tplNone')" clearable style="width: 280px">
              <el-option :value="''" :label="t('notifyCfg.tplNone')" />
              <el-option v-for="tpl in templates" :key="tpl.id" :value="String(tpl.id)" :label="tpl.name" />
            </el-select>
            <el-button link type="primary" @click="openManage">{{ t('notifyCfg.tplManage') }}</el-button>
          </div>
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

    <el-dialog
      v-model="manageVisible"
      :title="t('notifyCfg.tplManage')"
      width="640px"
      top="6vh"
      destroy-on-close
    >
      <template v-if="!editing">
        <div class="tpl-list-head">
          <span>{{ t('notifyCfg.tplManage') }}</span>
          <el-button type="primary" size="small" @click="startCreate">{{ t('notifyCfg.tplNew') }}</el-button>
        </div>
        <el-table :data="templates" height="360" v-loading="tplLoading">
          <el-table-column prop="name" :label="t('notifyCfg.tplName')" min-width="160" />
          <el-table-column :label="t('notifyCfg.tplScope')" width="100">
            <template #default="{ row }">
              <el-tag v-if="row.scope === 'global'" size="small" type="warning">
                {{ t('notifyCfg.tplGlobalTag') }}
              </el-tag>
              <el-tag v-else size="small">{{ t('notifyCfg.tplSelfTag') }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="t('common.operation')" width="150">
            <template #default="{ row }">
              <el-button link type="primary" size="small" @click="startEdit(row)">
                {{ t('notifyCfg.tplEdit') }}
              </el-button>
              <el-button link type="danger" size="small" @click="remove(row)">
                {{ t('notifyCfg.tplDelete') }}
              </el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-empty
          v-if="!tplLoading && templates.length === 0"
          :description="t('notifyCfg.tplNoContent')"
        />
      </template>

      <template v-else>
        <el-form label-width="88px" label-position="right">
          <el-form-item :label="t('notifyCfg.tplName')" required>
            <el-input
              v-model="form.name"
              :placeholder="t('notifyCfg.tplName')"
              maxlength="60"
              show-word-limit
            />
          </el-form-item>
          <el-form-item v-if="isAdmin" :label="t('notifyCfg.tplScope')">
            <el-radio-group v-model="form.scope">
              <el-radio value="self">{{ t('notifyCfg.tplScopeSelf') }}</el-radio>
              <el-radio value="global">{{ t('notifyCfg.tplScopeGlobal') }}</el-radio>
            </el-radio-group>
          </el-form-item>
          <el-form-item :label="t('notifyCfg.subject')" required>
            <el-input
              v-model="form.subject"
              :placeholder="t('notifyCfg.subject')"
              maxlength="200"
              show-word-limit
            />
          </el-form-item>
          <el-form-item :label="t('notifyCfg.format')">
            <el-radio-group v-model="form.isHtml">
              <el-radio :value="false">{{ t('notifyCfg.formatText') }}</el-radio>
              <el-radio :value="true">{{ t('notifyCfg.formatHtml') }}</el-radio>
            </el-radio-group>
          </el-form-item>
          <el-form-item :label="t('notifyCfg.body')">
            <div class="editor-wrap">
              <CodeEditor
                :model-value="form.isHtml ? form.bodyHtml : form.bodyText"
                :lang="form.isHtml ? 'html' : 'text'"
                :placeholder="t('notifyCfg.body')"
                @update:model-value="onFormBody"
              />
            </div>
          </el-form-item>
        </el-form>
      </template>

      <template #footer>
        <template v-if="editing">
          <el-button @click="editing = false">{{ t('notifyCfg.tplBack') }}</el-button>
          <el-button type="primary" :loading="saving" @click="save">
            {{ t('notifyCfg.tplSave') }}
          </el-button>
        </template>
        <template v-else>
          <el-button @click="manageVisible = false">{{ t('common.close') }}</el-button>
        </template>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox, type TableInstance } from 'element-plus'
import CodeEditor from '@/components/CodeEditor.vue'
import {
  broadcastMail,
  getBroadcastTemplates,
  saveBroadcastTemplate,
  deleteBroadcastTemplate,
} from '@/api/systemNotify'
import type { BroadcastTemplate } from '@/api/systemNotify'
import { getUserList } from '@/api/user'
import { useUserStore } from '@/stores/user'
import type { UserListItem } from '@/api/user'

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

// ── 自建群发通知模板 ──
const templates = ref<BroadcastTemplate[]>([])
const tplLoading = ref(false)
const manageVisible = ref(false)
const editing = ref(false)
const saving = ref(false)
const editId = ref<number | null>(null)
const form = reactive({
  name: '',
  scope: 'self' as 'global' | 'self',
  subject: '',
  isHtml: false,
  bodyText: '',
  bodyHtml: '',
})

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

// 选模板预填：从自建模板取主题 + 正文（HTML 标记与纯文本兜底）
watch(tplKey, (key) => {
  const id = Number(key)
  if (!id) {
    subject.value = ''
    bodyText.value = ''
    bodyHtml.value = ''
    isHtml.value = false
    return
  }
  const tpl = templates.value.find((x) => x.id === id)
  if (tpl) {
    subject.value = tpl.subject
    isHtml.value = tpl.is_html
    bodyText.value = tpl.body_text
    bodyHtml.value = tpl.body_html
  }
})

async function loadTemplates() {
  tplLoading.value = true
  try {
    const { data } = await getBroadcastTemplates()
    templates.value = data.list ?? []
  } catch {
    /* 拦截器已提示 */
  } finally {
    tplLoading.value = false
  }
}

function openManage() {
  editing.value = false
  manageVisible.value = true
  loadTemplates()
}

function startCreate() {
  editId.value = null
  form.name = ''
  form.scope = 'self'
  form.subject = ''
  form.isHtml = false
  form.bodyText = ''
  form.bodyHtml = ''
  editing.value = true
}

function startEdit(row: BroadcastTemplate) {
  editId.value = row.id
  form.name = row.name
  form.scope = row.scope
  form.subject = row.subject
  form.isHtml = row.is_html
  form.bodyText = row.body_text
  form.bodyHtml = row.body_html
  editing.value = true
}

function onFormBody(v: string) {
  if (form.isHtml) form.bodyHtml = v
  else form.bodyText = v
}

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning(t('notifyCfg.tplNameRequired'))
    return
  }
  if (!form.subject.trim()) {
    ElMessage.warning(t('notifyCfg.emptySubject'))
    return
  }
  saving.value = true
  try {
    await saveBroadcastTemplate(
      {
        name: form.name.trim(),
        scope: isAdmin.value ? form.scope : 'self',
        subject: form.subject.trim(),
        body_text: form.bodyText,
        body_html: form.bodyHtml || undefined,
        is_html: form.isHtml,
      },
      editId.value ?? undefined,
    )
    ElMessage.success(editId.value ? t('notifyCfg.tplUpdated') : t('notifyCfg.tplCreated'))
    editing.value = false
    await loadTemplates()
  } catch {
    /* 拦截器已提示 */
  } finally {
    saving.value = false
  }
}

async function remove(row: BroadcastTemplate) {
  try {
    await ElMessageBox.confirm(
      t('notifyCfg.tplConfirmDelete', { name: row.name }),
      t('notifyCfg.tplManage'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteBroadcastTemplate(row.id)
    ElMessage.success(t('notifyCfg.tplDeleted'))
    if (String(row.id) === tplKey.value) tplKey.value = ''
    await loadTemplates()
  } catch {
    /* 拦截器已提示 */
  }
}

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
  } catch (err) {
    ElMessage.error((err as Error)?.message || t('error.system'))
  } finally {
    sending.value = false
  }
}

onMounted(() => {
  loadRecipients()
  loadTemplates()
})
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
.tpl-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.tpl-list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
  font-weight: 600;
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
