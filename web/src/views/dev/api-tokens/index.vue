<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="api-tokens-container">
    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <div class="header-left">
            <span class="title">API Tokens</span>
            <el-tag type="info" size="small" style="margin-left: 8px">{{
              t('devApiTokens.devTag')
            }}</el-tag>
          </div>
          <el-button type="primary" :icon="Plus" @click="openCreate">{{
            t('devApiTokens.createBtn')
          }}</el-button>
        </div>
      </template>

      <el-alert type="info" :closable="false" class="tip">
        <p style="margin: 0 0 4px">
          {{ t('devApiTokens.tip1') }}<code>Authorization: Bearer &lt;token&gt;</code
          >{{ t('devApiTokens.tip2') }}
        </p>
        <p style="margin: 0">
          {{ t('devApiTokens.tipSecurity') }}
        </p>
      </el-alert>

      <el-table :data="tableData" v-loading="loading" stripe style="margin-top: 14px">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column
          prop="name"
          :label="t('devApiTokens.colName')"
          min-width="130"
          show-overflow-tooltip
        >
          <template #default="{ row }">{{ row.name || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('devApiTokens.colTokenPrefix')" min-width="220">
          <template #default="{ row }">
            <code class="token-prefix">{{ row.prefix }}…</code>
          </template>
        </el-table-column>
        <el-table-column :label="t('devApiTokens.colStatus')" width="90">
          <template #default="{ row }">
            <el-switch
              :model-value="row.status === 1"
              inline-prompt
              :active-text="t('devApiTokens.enable')"
              :inactive-text="t('devApiTokens.disable')"
              @change="(v: boolean) => toggleStatus(row, v)"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('devApiTokens.colExpires')" width="160">
          <template #default="{ row }">
            <span v-if="row.expires_at === 0">{{ t('devApiTokens.neverExpire') }}</span>
            <el-tag v-else-if="row.expires_at < nowTs" type="danger" size="small">{{
              t('devApiTokens.expired')
            }}</el-tag>
            <span v-else>{{ fmtTime(row.expires_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('devApiTokens.colLastUsed')" width="160">
          <template #default="{ row }">
            <span v-if="row.last_used_at">{{ fmtTime(row.last_used_at) }}</span>
            <span v-else class="never">{{ t('devApiTokens.neverUsed') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.createdAt')" width="160">
          <template #default="{ row }">{{ fmtTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="140" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link @click="openRename(row)">{{
              t('devApiTokens.rename')
            }}</el-button>
            <el-button type="danger" link @click="handleDelete(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新建 -->
    <el-dialog
      v-model="createVisible"
      :title="t('devApiTokens.createTitle')"
      width="460px"
      @closed="resetCreate"
    >
      <el-form :model="createForm" label-width="90px" @submit.prevent>
        <el-form-item :label="t('devApiTokens.nameLabel')">
          <el-input
            v-model="createForm.name"
            :placeholder="t('devApiTokens.namePlaceholder')"
            maxlength="60"
          />
        </el-form-item>
        <el-form-item :label="t('devApiTokens.expireLabel')">
          <el-input-number v-model="createForm.expire_days" :min="0" :max="3650" />
          <span class="form-hint">{{ t('devApiTokens.expireHint') }}</span>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="creating" @click="submitCreate">{{
          t('devApiTokens.generate')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 创建成功：仅此一次显示完整 Token -->
    <el-dialog
      v-model="createdVisible"
      :title="t('devApiTokens.createdTitle')"
      width="640px"
      :close-on-click-modal="false"
    >
      <el-alert
        type="warning"
        :closable="false"
        :title="t('devApiTokens.copyNow')"
        show-icon
        :description="t('devApiTokens.copyNowDesc')"
      />
      <div style="margin-top: 14px">
        <el-input :model-value="createdToken" readonly>
          <template #append>
            <el-button @click="copyToken">{{ t('devApiTokens.copy') }}</el-button>
          </template>
        </el-input>
        <div class="created-meta">
          <span v-if="createdExpire === 0">{{ t('devApiTokens.validForever') }}</span>
          <span v-else>{{ t('devApiTokens.validUntil', { time: fmtTime(createdExpire) }) }}</span>
        </div>
      </div>
      <template #footer>
        <el-button type="primary" @click="createdVisible = false">{{
          t('devApiTokens.saved')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 重命名 -->
    <el-dialog v-model="renameVisible" :title="t('devApiTokens.renameTitle')" width="420px">
      <el-input v-model="renameName" :placeholder="t('devApiTokens.nameLabel')" maxlength="60" />
      <template #footer>
        <el-button @click="renameVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="renaming" @click="submitRename">{{
          t('common.save')
        }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  getApiTokenList,
  createApiToken,
  updateApiToken,
  deleteApiToken,
  type ApiTokenItem,
} from '@/api/dev'

const { t } = useI18n()

const nowTs = ref(Math.floor(Date.now() / 1000))

const loading = ref(false)
const tableData = ref<ApiTokenItem[]>([])

async function loadList() {
  loading.value = true
  try {
    const res = await getApiTokenList()
    tableData.value = res.data ?? []
  } catch {
    /* handled by interceptor */
  } finally {
    loading.value = false
  }
}

// ── 新建 ────────────────────────────────────────────────────
const createVisible = ref(false)
const creating = ref(false)
const createForm = reactive({ name: '', expire_days: 0 })

function openCreate() {
  createForm.name = ''
  createForm.expire_days = 0
  createVisible.value = true
}

async function submitCreate() {
  creating.value = true
  try {
    const res = await createApiToken({
      name: createForm.name || '',
      expire_days: createForm.expire_days || 0,
    })
    createdToken.value = res.data?.token ?? ''
    createdExpire.value = res.data?.expires_at ?? 0
    createVisible.value = false
    createdVisible.value = true
    loadList()
  } catch {
    /* handled */
  } finally {
    creating.value = false
  }
}

function resetCreate() {
  createForm.name = ''
  createForm.expire_days = 0
}

// ── 展示完整 Token ─────────────────────────────────────────
const createdVisible = ref(false)
const createdToken = ref('')
const createdExpire = ref(0)

async function copyToken() {
  try {
    await navigator.clipboard.writeText(createdToken.value)
    ElMessage.success(t('devApiTokens.copyOk'))
  } catch {
    // 手工复制兜底：选中输入框内容
    ElMessage.info(t('devApiTokens.copyManual'))
  }
}

// ── 启停 / 重命名 / 删除 ───────────────────────────────────
async function toggleStatus(row: ApiTokenItem, enabled: boolean) {
  try {
    await updateApiToken({ id: row.id, status: enabled ? 1 : 0 })
    row.status = enabled ? 1 : 0
    ElMessage.success(enabled ? t('devApiTokens.enabled') : t('devApiTokens.disabled'))
  } catch {
    /* handled */
  }
}

const renameVisible = ref(false)
const renaming = ref(false)
const renameId = ref(0)
const renameName = ref('')

function openRename(row: ApiTokenItem) {
  renameId.value = row.id
  renameName.value = row.name
  renameVisible.value = true
}

async function submitRename() {
  renaming.value = true
  try {
    await updateApiToken({ id: renameId.value, name: renameName.value.trim() })
    ElMessage.success(t('devApiTokens.updated'))
    renameVisible.value = false
    loadList()
  } catch {
    /* handled */
  } finally {
    renaming.value = false
  }
}

async function handleDelete(row: ApiTokenItem) {
  try {
    await ElMessageBox.confirm(
      t('devApiTokens.deleteConfirm', { name: row.name || row.prefix }),
      t('devApiTokens.deleteTitle'),
      { type: 'warning', confirmButtonText: t('devApiTokens.confirmDelete') },
    )
  } catch {
    return
  }
  try {
    await deleteApiToken(row.id)
    ElMessage.success(t('devApiTokens.deleteOk'))
    loadList()
  } catch {
    /* handled */
  }
}

function fmtTime(ts: number) {
  return ts ? new Date(ts * 1000).toLocaleString() : '-'
}

onMounted(() => {
  loadList()
  // 页面停留期间刷新“已过期”展示
  setInterval(() => {
    nowTs.value = Math.floor(Date.now() / 1000)
  }, 30000)
})
</script>

<style scoped>
.api-tokens-container {
  padding: 20px;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.title {
  font-size: 16px;
  font-weight: 600;
}
.tip code,
.token-prefix {
  background: var(--el-fill-color);
  border-radius: 3px;
  padding: 1px 5px;
  font-family: 'JetBrains Mono', Consolas, monospace;
}
.form-hint {
  margin-left: 10px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.created-meta {
  margin-top: 8px;
  color: var(--el-text-color-regular);
  font-size: 13px;
}
.never {
  color: var(--el-text-color-placeholder);
}
</style>
