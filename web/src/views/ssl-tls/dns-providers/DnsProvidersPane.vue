<!--
  DNS 服务商凭据面板。

  既作为独立页面（`/ssl-tls/dns-providers`，侧栏入口已收起但 URL 仍可达），
  也被 SSL 证书页放进抽屉里复用（`embedded`）：此时不再重复渲染标题，
  由抽屉自身的标题栏承担。
-->
<template>
  <div :class="embedded ? 'dns-pane' : 'dns-container'">
    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <div class="header-left">
            <span v-if="!embedded" class="title">{{ t('dnsProvider.cardTitle') }}</span>
            <el-tag type="info" size="small" :style="embedded ? '' : 'margin-left: 8px'"
              >DNS-01</el-tag
            >
          </div>
          <el-button type="primary" :icon="Plus" @click="openAdd">{{
            t('dnsProvider.add')
          }}</el-button>
        </div>
      </template>

      <el-alert type="info" :closable="false" class="tip">
        <p style="margin: 0 0 4px">{{ t('dnsProvider.tipUsage') }}</p>
        <p style="margin: 0">
          {{ t('dnsProvider.tipSecurity') }}
        </p>
      </el-alert>

      <el-table :data="tableData" v-loading="loading" stripe style="margin-top: 14px">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column v-if="canManageAll" :label="t('dnsProvider.colOwner')" width="160">
          <template #default="{ row }">{{
            row.user_id ? ownerText(row.user_id) : t('dnsProvider.ownerSelf')
          }}</template>
        </el-table-column>
        <el-table-column
          prop="name"
          :label="t('dnsProvider.colName')"
          min-width="150"
          show-overflow-tooltip
        />
        <el-table-column :label="t('dnsProvider.colProvider')" width="170">
          <template #default="{ row }">{{ providerLabel(row.provider) }}</template>
        </el-table-column>
        <el-table-column :label="t('dnsProvider.colCredentials')" min-width="260">
          <template #default="{ row }">
            <div v-for="(v, k) in row.credentials" :key="k" class="cred-line mono">
              <span class="cred-key">{{ fieldLabel(row.provider, String(k)) }}</span>
              <span class="cred-val">{{ v }}</span>
            </div>
          </template>
        </el-table-column>
        <el-table-column :label="t('dnsProvider.colRemark')" min-width="130" show-overflow-tooltip>
          <template #default="{ row }">{{ row.remark || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="210" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link @click="openEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button type="success" link @click="handleTest(row)">{{
              t('dnsProvider.test')
            }}</el-button>
            <el-button type="danger" link @click="handleDelete(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <span>{{ t('dnsProvider.empty') }}</span>
        </template>
      </el-table>
    </el-card>

    <!-- 新增 / 编辑 -->
    <el-dialog
      v-model="editVisible"
      :title="form.id ? t('dnsProvider.editTitle') : t('dnsProvider.addTitle')"
      width="640px"
      @closed="resetForm"
    >
      <el-form :model="form" label-width="120px" @submit.prevent>
        <el-form-item :label="t('dnsProvider.colName')">
          <el-input v-model="form.name" :placeholder="t('dnsProvider.namePlaceholder')" maxlength="60" />
        </el-form-item>
        <el-form-item :label="t('dnsProvider.colProvider')">
          <el-select
            v-model="form.provider"
            :placeholder="t('dnsProvider.providerPlaceholder')"
            :loading="metasLoading"
            style="width: 100%"
            @change="resetCredentialInputs"
          >
            <el-option v-for="m in metas" :key="m.kind" :label="m.label" :value="m.kind" />
          </el-select>
        </el-form-item>

        <!-- 凭据字段由后端下发，新增服务商无需改前端 -->
        <el-form-item
          v-for="f in currentFields"
          :key="f.key"
          :label="f.label"
          class="cred-item"
        >
          <el-input
            v-model="credInputs[f.key]"
            :type="f.secret ? 'password' : 'text'"
            :placeholder="f.hint"
            :show-password="f.secret"
            autocomplete="new-password"
          />
          <span class="form-hint">{{ f.hint }}</span>
        </el-form-item>

        <el-form-item v-if="canManageAll" :label="t('dnsProvider.colOwner')">
          <el-select
            v-model="form.user_id"
            filterable
            :loading="ownersLoading"
            :placeholder="t('dnsProvider.ownerPlaceholder')"
            style="width: 100%"
          >
            <el-option
              v-for="o in ownerOptions"
              :key="o.id"
              :label="`${o.nickname || o.username} (${o.username})`"
              :value="o.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('dnsProvider.colRemark')">
          <el-input v-model="form.remark" maxlength="200" />
        </el-form-item>
      </el-form>

      <template #footer>
        <el-button @click="editVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button :loading="testing" @click="submitTest">{{ t('dnsProvider.test') }}</el-button>
        <el-button type="primary" :loading="saving" @click="submitSave">{{
          t('common.save')
        }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'
import { useUserStore } from '@/stores/user'
import {
  getAcmeDnsProviders,
  getAcmeDnsList,
  saveAcmeDnsProvider,
  deleteAcmeDnsProvider,
  testAcmeDnsProvider,
  type AcmeDnsProviderItem,
  type AcmeDnsProviderMeta,
  type OwnerOption,
} from '@/api/ssl'

/** embedded：嵌在别的页面的抽屉里时置 true（省掉外层留白与重复标题） */
withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false })
/** 增删改后通知宿主刷新（如证书页的 DNS 服务商下拉） */
const emit = defineEmits<{ changed: [] }>()

const { t } = useI18n()
const userStore = useUserStore()
const canManageAll = computed(
  () => userStore.roles.includes('admin') || userStore.roles.includes('reseller'),
)

const loading = ref(false)
const tableData = ref<AcmeDnsProviderItem[]>([])
const metas = ref<AcmeDnsProviderMeta[]>([])
const metasLoading = ref(false)

const ownerOptions = ref<OwnerOption[]>([])
const ownersLoading = ref(false)
const ownerText = (id: number) => {
  const o = ownerOptions.value.find((x) => x.id === id)
  return o ? `${o.nickname || o.username} (${o.username})` : `#${id}`
}
const providerLabel = (kind: string) => metas.value.find((m) => m.kind === kind)?.label || kind
const fieldLabel = (kind: string, key: string) =>
  metas.value.find((m) => m.kind === kind)?.fields.find((f) => f.key === key)?.label || key

async function loadMetas() {
  metasLoading.value = true
  try {
    const res = await getAcmeDnsProviders()
    metas.value = res.data ?? []
  } catch {
    /* handled */
  } finally {
    metasLoading.value = false
  }
}

async function loadList() {
  loading.value = true
  try {
    const res = await getAcmeDnsList()
    tableData.value = res.data ?? []
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

async function loadOwners() {
  if (!canManageAll.value) return
  ownersLoading.value = true
  try {
    const res = await http.get<{ code: number; data: OwnerOption[] }>('/site/users')
    ownerOptions.value = res.data || []
  } catch {
    /* handled */
  } finally {
    ownersLoading.value = false
  }
}

// ── 表单 ────────────────────────────────────────────────────
const editVisible = ref(false)
const saving = ref(false)
const testing = ref(false)
const form = reactive<{
  id?: number
  name: string
  provider: string
  remark: string
  user_id?: number
}>({ name: '', provider: '', remark: '', user_id: undefined })
const credInputs = reactive<Record<string, string>>({})

const currentFields = computed(
  () => metas.value.find((m) => m.kind === form.provider)?.fields ?? [],
)

function resetCredentialInputs() {
  for (const k of Object.keys(credInputs)) delete credInputs[k]
  for (const f of currentFields.value) credInputs[f.key] = ''
}

function resetForm() {
  form.id = undefined
  form.name = ''
  form.provider = metas.value[0]?.kind ?? ''
  form.remark = ''
  form.user_id = undefined
  resetCredentialInputs()
}

function openAdd() {
  resetForm()
  form.user_id = canManageAll.value ? userStore.userInfo.id : undefined
  editVisible.value = true
}

function openEdit(row: AcmeDnsProviderItem) {
  resetForm()
  form.id = row.id
  form.name = row.name
  form.provider = row.provider
  form.remark = row.remark
  form.user_id = row.user_id
  // 编辑时不回填真实密钥（列表只给脱敏值），留空表示保持不变
  editVisible.value = true
}

/** 收集团表里填写的凭据；空值字段会被丢弃，后端据此保留原值 */
function collectCredentials() {
  const out: Record<string, string> = {}
  for (const f of currentFields.value) {
    const v = String(credInputs[f.key] || '').trim()
    if (v) out[f.key] = v
  }
  return out
}

async function submitSave() {
  if (!form.name.trim()) {
    ElMessage.warning(t('dnsProvider.nameRequired'))
    return
  }
  if (!form.provider) {
    ElMessage.warning(t('dnsProvider.providerRequired'))
    return
  }
  const creds = collectCredentials()
  if (!form.id && currentFields.value.some((f) => !creds[f.key])) {
    ElMessage.warning(t('dnsProvider.credRequired'))
    return
  }
  saving.value = true
  try {
    await saveAcmeDnsProvider({
      ...(form.id ? { id: form.id } : {}),
      name: form.name.trim(),
      provider: form.provider,
      credentials: creds,
      remark: form.remark.trim(),
      ...(canManageAll.value ? { user_id: form.user_id || undefined } : {}),
    })
    ElMessage.success(t('dnsProvider.saved'))
    editVisible.value = false
    loadList()
    emit('changed')
  } catch {
    /* handled */
  } finally {
    saving.value = false
  }
}

/** 表单内的「测试连接」：用当前填写（未保存）的凭据探测 */
async function submitTest() {
  if (!form.provider) {
    ElMessage.warning(t('dnsProvider.providerRequired'))
    return
  }
  const creds = collectCredentials()
  const missing = currentFields.value.filter((f) => !creds[f.key])
  if (missing.length) {
    ElMessage.warning(t('dnsProvider.credRequired'))
    return
  }
  testing.value = true
  try {
    const res = await testAcmeDnsProvider(form.provider, creds)
    ElMessage.success(res.data?.message || t('dnsProvider.testOk'))
  } catch {
    /* handled */
  } finally {
    testing.value = false
  }
}

/** 列表里的「测试连接」：用库里保存的密钥（后端合并），此处不传任何明文 */
async function handleTest(row: AcmeDnsProviderItem) {
  testing.value = true
  try {
    const res = await testAcmeDnsProvider(row.provider, {}, row.id)
    ElMessage.success(res.data?.message || t('dnsProvider.testOk'))
  } catch {
    /* handled */
  } finally {
    testing.value = false
  }
}

async function handleDelete(row: AcmeDnsProviderItem) {
  try {
    await ElMessageBox.confirm(
      t('dnsProvider.deleteConfirm', { name: row.name }),
      t('dnsProvider.deleteTitle'),
      { type: 'warning', confirmButtonText: t('dnsProvider.confirmDelete') },
    )
  } catch {
    return
  }
  try {
    await deleteAcmeDnsProvider(row.id)
    ElMessage.success(t('dnsProvider.deleteOk'))
    loadList()
    emit('changed')
  } catch {
    /* handled */
  }
}

onMounted(async () => {
  await loadMetas()
  resetForm()
  loadList()
  loadOwners()
})
</script>

<style scoped>
.dns-container {
  padding: 20px;
}
/* 抽屉里由 el-drawer 提供留白与标题，这里不再重复 */
.dns-pane {
  padding: 0;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
}
.title {
  font-size: 16px;
  font-weight: 600;
}
.form-hint {
  display: block;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.4;
}
.mono {
  font-family: 'JetBrains Mono', Consolas, monospace;
}
.cred-line {
  display: flex;
  gap: 8px;
  font-size: 12px;
  line-height: 1.7;
}
.cred-key {
  color: var(--el-text-color-secondary);
  min-width: 150px;
}
.cred-val {
  color: var(--el-text-color-regular);
  word-break: break-all;
}
.cred-item {
  margin-bottom: 4px;
}
</style>
