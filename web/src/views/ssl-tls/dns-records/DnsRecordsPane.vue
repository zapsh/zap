<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="dns-records">
    <el-alert
      type="info"
      :closable="false"
      show-icon
      class="top-tip"
      :description="t('sslCerts.dnsRecordsTip')"
    />

    <!-- 服务商选择：仅展示支持解析记录管理的服务商 -->
    <div class="provider-bar">
      <span class="bar-label">{{ t('sslCerts.leDnsProvider') }}</span>
      <el-select
        v-model="providerId"
        :loading="providersLoading"
        :placeholder="t('sslCerts.leDnsProviderPlaceholder')"
        style="width: 320px"
        @change="onProviderChange"
      >
        <el-option
          v-for="p in recordProviders"
          :key="p.id"
          :label="`${p.name}（${providerLabel(p.provider)}）`"
          :value="p.id"
        />
      </el-select>
      <el-button
        v-if="!recordProviders.length"
        link
        type="primary"
        @click="goManage"
        >{{ t('sslCerts.dnsRecordsNoProvider') }}</el-button
      >
    </div>

    <el-row v-loading="domainsLoading" :gutter="14" class="body">
      <!-- 左：域名列表 -->
      <el-col :span="8" class="col">
        <div class="col-title">{{ t('sslCerts.dnsRecordsDomains') }}</div>
        <el-empty
          v-if="!providerId"
          :description="t('sslCerts.dnsRecordsPickProvider')"
          :image-size="80"
        />
        <el-empty
          v-else-if="!domainsLoading && !domains.length"
          :description="t('sslCerts.dnsRecordsNoDomain')"
          :image-size="80"
        />
        <ul v-else class="domain-list">
          <li
            v-for="d in domains"
            :key="d.id"
            :class="{ active: d.name === activeDomain }"
            @click="selectDomain(d.name)"
          >
            <span class="d-name">{{ d.name }}</span>
            <el-tag v-if="d.status !== 'active'" size="small" type="info">{{
              d.status
            }}</el-tag>
          </li>
        </ul>
      </el-col>

      <!-- 右：解析记录表 -->
      <el-col :span="16" class="col">
        <div class="col-title">
          {{ t('sslCerts.dnsRecordsList') }}
          <el-tag v-if="activeDomain" size="small" effect="plain">{{ activeDomain }}</el-tag>
          <el-button
            v-if="activeDomain"
            size="small"
            type="primary"
            :icon="Plus"
            style="float: right"
            @click="openCreate"
            >{{ t('sslCerts.dnsRecordsAdd') }}</el-button
          >
        </div>
        <el-table
          v-loading="recordsLoading"
          :data="records"
          size="small"
          height="100%"
          class="rec-table"
        >
          <el-table-column :label="t('sslCerts.dnsRecordsHost')" min-width="150" show-overflow-tooltip>
            <template #default="{ row }">{{ row.name }}</template>
          </el-table-column>
          <el-table-column prop="type" label="类型" width="80" />
          <el-table-column
            :label="t('sslCerts.dnsRecordsValue')"
            min-width="180"
            show-overflow-tooltip
          >
            <template #default="{ row }">
              <span class="mono">{{ row.content }}</span>
            </template>
          </el-table-column>
          <el-table-column :label="t('sslCerts.dnsRecordsTtl')" width="70">
            <template #default="{ row }">{{ row.ttl || '-' }}</template>
          </el-table-column>
          <el-table-column :label="t('sslCerts.dnsRecordsPriority')" width="80">
            <template #default="{ row }">{{ row.priority ?? '-' }}</template>
          </el-table-column>
          <el-table-column :label="t('sslCerts.dnsRecordsLine')" width="90">
            <template #default="{ row }">{{ row.line || '-' }}</template>
          </el-table-column>
          <el-table-column :label="t('common.operation')" width="130" fixed="right">
            <template #default="{ row }">
              <el-button link type="primary" size="small" @click="openEdit(row)">{{
                t('common.edit')
              }}</el-button>
              <el-button link type="danger" size="small" @click="remove(row)">{{
                t('common.delete')
              }}</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-pagination
          v-if="pages > 1"
          class="rec-pager"
          layout="prev, pager, next"
          :total="total"
          :page-size="50"
          :current-page="page"
          @current-change="(p: number) => loadRecords(p)"
        />
      </el-col>
    </el-row>

    <!-- 新建 / 编辑 -->
    <el-dialog
      v-model="dialog.visible"
      :title="dialog.id ? t('sslCerts.dnsRecordsEdit') : t('sslCerts.dnsRecordsAdd')"
      width="520px"
      @closed="resetDialog"
    >
      <el-form label-width="84px" @submit.prevent>
        <el-form-item :label="t('sslCerts.dnsRecordsType')">
          <el-select v-model="dialog.type" style="width: 100%" @change="onTypeChange">
            <el-option v-for="tp in recordTypes" :key="tp" :label="tp" :value="tp" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('sslCerts.dnsRecordsHost')">
          <el-input
            v-model="dialog.name"
            :placeholder="t('sslCerts.dnsRecordsHostPh')"
          />
          <span class="form-hint">{{ t('sslCerts.dnsRecordsHostHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('sslCerts.dnsRecordsValue')">
          <el-input
            v-model="dialog.content"
            :placeholder="t('sslCerts.dnsRecordsValuePh')"
            type="textarea"
            :rows="2"
          />
        </el-form-item>
        <el-form-item :label="t('sslCerts.dnsRecordsTtl')">
          <el-input-number v-model="dialog.ttl" :min="1" :max="86400" />
        </el-form-item>
        <el-form-item v-if="showPriority" :label="t('sslCerts.dnsRecordsPriority')">
          <el-input-number v-model="dialog.priority" :min="0" :max="65535" />
        </el-form-item>
        <el-form-item :label="t('sslCerts.dnsRecordsLine')">
          <el-input
            v-model="dialog.line"
            :placeholder="t('sslCerts.dnsRecordsLinePh')"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialog.visible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="submit">{{
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
import {
  getAcmeDnsProviders,
  getAcmeDnsList,
  getDnsDomains,
  getDnsRecords,
  createDnsRecord,
  updateDnsRecord,
  deleteDnsRecord,
  type AcmeDnsProviderItem,
  type AcmeDnsProviderMeta,
  type DnsDomainItem,
  type DnsRecordItem,
  type DnsRecordInput,
} from '@/api/ssl'

const { t } = useI18n()

const recordTypes = ['A', 'AAAA', 'CNAME', 'MX', 'TXT', 'NS', 'SRV', 'CAA']

const providers = ref<AcmeDnsProviderItem[]>([])
const providerMetas = ref<AcmeDnsProviderMeta[]>([])
const providersLoading = ref(false)
const providerId = ref<number>()
const providerLabel = (kind: string) =>
  providerMetas.value.find((m) => m.kind === kind)?.label || kind

/** 仅展示支持解析记录管理的服务商（meta.records === true） */
const recordProviders = computed(() =>
  providers.value.filter((p) => providerMetas.value.find((m) => m.kind === p.provider)?.records),
)

const domains = ref<DnsDomainItem[]>([])
const domainsLoading = ref(false)
const activeDomain = ref('')

const records = ref<DnsRecordItem[]>([])
const recordsLoading = ref(false)
const page = ref(1)
const total = ref(0)
const pages = ref(1)

const dialog = reactive<{
  visible: boolean
  id?: string
  name: string
  type: string
  content: string
  ttl: number
  priority: number
  line: string
}>({ visible: false, name: '', type: 'A', content: '', ttl: 600, priority: 10, line: '' })
const saving = ref(false)
const showPriority = computed(() => dialog.type === 'MX' || dialog.type === 'SRV')

async function loadProviders() {
  providersLoading.value = true
  try {
    const [metas, list] = await Promise.all([getAcmeDnsProviders(), getAcmeDnsList()])
    providerMetas.value = metas.data ?? []
    providers.value = list.data ?? []
    if (!providerId.value && recordProviders.value.length) {
      providerId.value = recordProviders.value[0].id
      await loadDomains()
    }
  } catch (e) {
    ElMessage.error((e as any)?.message || '服务商列表加载失败')
  } finally {
    providersLoading.value = false
  }
}

async function loadDomains() {
  if (!providerId.value) return
  domainsLoading.value = true
  try {
    const res = await getDnsDomains(providerId.value)
    domains.value = res.data ?? []
    // 自动选中第一个域名；若当前选中的还在这批里就保留
    if (!activeDomain.value || !domains.value.some((d) => d.name === activeDomain.value)) {
      activeDomain.value = domains.value[0]?.name || ''
    }
    if (activeDomain.value) await loadRecords(1)
    else records.value = []
  } catch (e) {
    ElMessage.error((e as any)?.message || '域名列表加载失败')
  } finally {
    domainsLoading.value = false
  }
}

async function loadRecords(p: number) {
  if (!providerId.value || !activeDomain.value) return
  page.value = p
  recordsLoading.value = true
  try {
    const res = await getDnsRecords({
      provider_id: providerId.value,
      domain: activeDomain.value,
      page: p,
    })
    records.value = res.data?.records ?? []
    total.value = res.data?.total ?? 0
    pages.value = res.data?.pages ?? 1
  } catch (e) {
    ElMessage.error((e as any)?.message || '解析记录加载失败')
  } finally {
    recordsLoading.value = false
  }
}

function onProviderChange() {
  activeDomain.value = ''
  records.value = []
  loadDomains()
}

function selectDomain(name: string) {
  if (name === activeDomain.value) return
  activeDomain.value = name
  loadRecords(1)
}

function onTypeChange() {
  if (!showPriority.value) dialog.priority = 10
}

function openCreate() {
  resetDialog()
  dialog.visible = true
}

function openEdit(row: DnsRecordItem) {
  dialog.visible = true
  dialog.id = row.id
  dialog.name = row.name
  dialog.type = row.type
  dialog.content = row.content
  dialog.ttl = row.ttl || 600
  dialog.priority = row.priority ?? 10
  dialog.line = row.line || ''
}

function resetDialog() {
  dialog.visible = false
  dialog.id = undefined
  dialog.name = ''
  dialog.type = 'A'
  dialog.content = ''
  dialog.ttl = 600
  dialog.priority = 10
  dialog.line = ''
}

function validate(): DnsRecordInput | null {
  if (!dialog.name.trim()) {
    ElMessage.warning(t('sslCerts.dnsRecordsHostRequired'))
    return null
  }
  if (!dialog.content.trim()) {
    ElMessage.warning(t('sslCerts.dnsRecordsValueRequired'))
    return null
  }
  return {
    name: dialog.name.trim(),
    type: dialog.type,
    content: dialog.content.trim(),
    ttl: dialog.ttl,
    ...(showPriority.value ? { priority: dialog.priority } : {}),
    ...(dialog.line.trim() ? { line: dialog.line.trim() } : {}),
  }
}

async function submit() {
  const rec = validate()
  if (!rec || !providerId.value || !activeDomain.value) return
  saving.value = true
  try {
    if (dialog.id) {
      await updateDnsRecord({
        provider_id: providerId.value,
        domain: activeDomain.value,
        id: dialog.id,
        record: rec,
      })
      ElMessage.success(t('sslCerts.dnsRecordsUpdated'))
    } else {
      await createDnsRecord({
        provider_id: providerId.value,
        domain: activeDomain.value,
        record: rec,
      })
      ElMessage.success(t('sslCerts.dnsRecordsCreated'))
    }
    dialog.visible = false
    loadRecords(page.value)
  } catch (e) {
    ElMessage.error((e as any)?.message || '解析记录保存失败')
  } finally {
    saving.value = false
  }
}

async function remove(row: DnsRecordItem) {
  if (!providerId.value || !activeDomain.value) return
  try {
    await ElMessageBox.confirm(
      t('sslCerts.dnsRecordsDeleteConfirm', { name: row.name, type: row.type }),
      t('sslCerts.dnsRecordsDelete'),
      { type: 'warning', confirmButtonText: t('common.delete') },
    )
  } catch {
    return
  }
  try {
    await deleteDnsRecord({
      provider_id: providerId.value,
      domain: activeDomain.value,
      id: row.id,
    })
    ElMessage.success(t('sslCerts.dnsRecordsDeleted'))
    loadRecords(page.value)
  } catch {
    /* handled */
  }
}

function goManage() {
  // 跳到 DNS 服务商管理：当前页抽屉入口在父组件，这里直接提示去侧栏配置
  ElMessage.info(t('sslCerts.dnsRecordsManageHint'))
}

onMounted(loadProviders)
</script>

<style scoped>
.dns-records {
  padding: 4px 0;
}
.top-tip {
  margin-bottom: 12px;
}
.provider-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}
.bar-label {
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.col {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 10px;
  background: var(--el-fill-color-blank);
}
.col-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 10px;
  color: var(--el-text-color-primary);
}
.domain-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 480px;
  overflow: auto;
}
.domain-list li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 13px;
}
.domain-list li:hover {
  background: var(--el-fill-color-light);
}
.domain-list li.active {
  background: var(--el-color-primary-light-9);
  color: var(--el-color-primary);
}
.d-name {
  font-family: 'JetBrains Mono', Consolas, monospace;
  word-break: break-all;
}
.rec-table {
  margin-top: 4px;
}
.rec-pager {
  margin-top: 10px;
  justify-content: flex-end;
}
.mono {
  font-family: 'JetBrains Mono', Consolas, monospace;
}
.form-hint {
  margin-left: 10px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
</style>
