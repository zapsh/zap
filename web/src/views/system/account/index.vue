<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="account-mng">
    <div class="page-head">
      <div>
        <h3 class="page-title">{{ t('accountMng.title') }}</h3>
        <p class="page-sub">{{ t('accountMng.subtitle') }}</p>
      </div>
      <el-button :icon="Refresh" :loading="loading" @click="loadList">
        {{ t('accountMng.refresh') }}
      </el-button>
    </div>

    <!-- 工具栏：作用于多选 -->
    <div class="toolbar">
      <el-button
        type="primary"
        :icon="Refresh"
        :disabled="!hasSelection"
        @click="onResync"
      >
        {{ t('accountMng.resyncSel') }}
      </el-button>
      <el-button :icon="HardDrive" :disabled="!hasSelection" @click="openSetDisk">
        {{ t('accountMng.setDisk') }}
      </el-button>
      <el-button :icon="DataLine" :disabled="!hasSelection" @click="openSetBw">
        {{ t('accountMng.setBw') }}
      </el-button>
      <el-button :disabled="!hasSelection" @click="onResetDisk">
        {{ t('accountMng.resetDiskSel') }}
      </el-button>
      <el-button :disabled="!hasSelection" @click="onResetBw">
        {{ t('accountMng.resetBwSel') }}
      </el-button>
      <span v-if="hasSelection" class="sel-hint">
        {{ t('accountMng.bulkHint', { n: selectedIds.length }) }}
      </span>
    </div>

    <el-table
      v-loading="loading"
      :data="list"
      row-key="id"
      border
      stripe
      @selection-change="onSelect"
    >
      <el-table-column type="selection" width="46" />
      <el-table-column :label="t('accountMng.colUser')" min-width="160">
        <template #default="{ row }">
          <div class="user-cell">
            <span class="uname">{{ row.username }}</span>
            <small v-if="row.nickname" class="nick">{{ row.nickname }}</small>
          </div>
        </template>
      </el-table-column>
      <el-table-column :label="t('accountMng.colPackage')" min-width="120">
        <template #default="{ row }">
          {{ row.package_name || t('accountMng.followPackage') }}
        </template>
      </el-table-column>
      <el-table-column :label="t('accountMng.colDiskQuota')" min-width="120" align="right">
        <template #default="{ row }">
          <span :class="{ muted: !row.disk_quota_mb }">
            {{ row.disk_quota_mb || t('accountMng.followPackage') }}
          </span>
        </template>
      </el-table-column>
      <el-table-column :label="t('accountMng.colDiskUsed')" min-width="120" align="right">
        <template #default="{ row }">{{ fmtBytes(row.disk_used_bytes) }}</template>
      </el-table-column>
      <el-table-column :label="t('accountMng.colBwQuota')" min-width="120" align="right">
        <template #default="{ row }">
          <span :class="{ muted: !row.bandwidth_quota_mb }">
            {{ row.bandwidth_quota_mb || t('accountMng.followPackage') }}
          </span>
        </template>
      </el-table-column>
      <el-table-column :label="t('accountMng.colBwUsed')" min-width="120" align="right">
        <template #default="{ row }">{{ fmtBytes(row.bandwidth_used_bytes) }}</template>
      </el-table-column>
      <el-table-column :label="t('accountMng.colPhp')" min-width="140">
        <template #default="{ row }">
          <el-tag v-for="v in phpVersions(row)" :key="v" size="small" class="php-tag">{{ v }}</el-tag>
          <span v-if="!phpVersions(row).length" class="muted">—</span>
        </template>
      </el-table-column>
      <el-table-column :label="t('accountMng.resync')" min-width="220" fixed="right">
        <template #default="{ row }">
          <el-button link type="primary" :icon="Refresh" @click="onResyncOne(row)">
            {{ t('accountMng.resync') }}
          </el-button>
          <el-button link type="primary" @click="onResetDiskOne(row)">
            {{ t('accountMng.resetDisk') }}
          </el-button>
          <el-button link type="primary" @click="onResetBwOne(row)">
            {{ t('accountMng.resetBw') }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>

    <!-- 设置配额弹窗 -->
    <el-dialog v-model="quotaDialog.visible" :title="t('accountMng.setQuotaTitle')" width="420px">
      <el-form label-width="auto">
        <el-form-item :label="t('accountMng.mbLabel')">
          <el-input-number v-model="quotaDialog.mb" :min="0" :step="100" style="width: 100%" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="quotaDialog.visible = false">{{ t('accountMng.cancel') }}</el-button>
        <el-button type="primary" :loading="quotaDialog.loading" @click="confirmSetQuota">
          {{ t('accountMng.confirm') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 操作结果弹窗 -->
    <el-dialog v-model="result.visible" :title="t('accountMng.opResult')" width="560px">
      <el-table :data="result.rows" size="small" border max-height="360">
        <el-table-column prop="id" label="ID" width="80" />
        <el-table-column label="用户名" min-width="120">
          <template #default="{ row }">{{ row.username || ('#' + row.id) }}</template>
        </el-table-column>
        <el-table-column label="结果" min-width="280">
          <template #default="{ row }">
            <el-tag :type="row.ok ? 'success' : 'danger'" size="small">
              {{ row.ok ? 'OK' : 'FAIL' }}
            </el-tag>
            <span class="result-msg">{{ row.message }}</span>
          </template>
        </el-table-column>
      </el-table>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { DataLine, HardDrive, Refresh } from '@/icons'
import { ElMessage } from 'element-plus'
import {
  getUserList,
  resyncUserPools,
  setUserBandwidthQuota,
  setUserDiskQuota,
  type PoolResyncResult,
  type QuotaOpResult,
  type UserListItem,
} from '@/api/user'

const { t } = useI18n()

const loading = ref(false)
const list = ref<UserListItem[]>([])
const selectedRows = ref<UserListItem[]>([])
const selectedIds = computed(() => selectedRows.value.map((r) => r.id))
const hasSelection = computed(() => selectedIds.value.length > 0)

const quotaDialog = reactive({ visible: false, loading: false, mb: 0, kind: 'disk' as 'disk' | 'bw' })
const result = reactive({ visible: false, rows: [] as (QuotaOpResult | PoolResyncResult)[] })

function fmtBytes(n: number): string {
  if (!n || n < 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return (i === 0 ? v : v.toFixed(2)) + ' ' + units[i]
}

function phpVersions(row: UserListItem): string[] {
  const set = new Set<string>()
  for (const s of row.sites || []) {
    if (s.php_instance) set.add(s.php_instance)
  }
  return [...set]
}

async function loadList() {
  loading.value = true
  try {
    const res = await getUserList()
    list.value = res.data || []
  } finally {
    loading.value = false
  }
}

function onSelect(rows: UserListItem[]) {
  selectedRows.value = rows
}

function showResult(data: (QuotaOpResult | PoolResyncResult)[]) {
  result.rows = data
  result.visible = true
}

async function onResync() {
  if (!hasSelection.value) return ElMessage.warning(t('accountMng.noSelection'))
  const res = await resyncUserPools(selectedIds.value)
  ElMessage.success(t('accountMng.resyncDone', { n: selectedIds.value.length }))
  showResult(res.data)
  await loadList()
}

async function onResyncOne(row: UserListItem) {
  const res = await resyncUserPools([row.id])
  showResult(res.data)
  await loadList()
}

function openSetDisk() {
  openSetQuota('disk')
}
function openSetBw() {
  openSetQuota('bw')
}
function openSetQuota(kind: 'disk' | 'bw') {
  if (!hasSelection.value) return ElMessage.warning(t('accountMng.noSelection'))
  quotaDialog.kind = kind
  quotaDialog.mb = 0
  quotaDialog.visible = true
}

async function confirmSetQuota() {
  quotaDialog.loading = true
  try {
    const ids = selectedIds.value
    const res =
      quotaDialog.kind === 'disk'
        ? await setUserDiskQuota(ids, quotaDialog.mb)
        : await setUserBandwidthQuota(ids, quotaDialog.mb)
    ElMessage.success(t('accountMng.quotaDone', { n: ids.length }))
    showResult(res.data)
    quotaDialog.visible = false
    await loadList()
  } finally {
    quotaDialog.loading = false
  }
}

async function onResetDisk() {
  if (!hasSelection.value) return ElMessage.warning(t('accountMng.noSelection'))
  const res = await setUserDiskQuota(selectedIds.value, 0)
  ElMessage.success(t('accountMng.quotaDone', { n: selectedIds.value.length }))
  showResult(res.data)
  await loadList()
}

async function onResetBw() {
  if (!hasSelection.value) return ElMessage.warning(t('accountMng.noSelection'))
  const res = await setUserBandwidthQuota(selectedIds.value, 0)
  ElMessage.success(t('accountMng.quotaDone', { n: selectedIds.value.length }))
  showResult(res.data)
  await loadList()
}

async function onResetDiskOne(row: UserListItem) {
  const res = await setUserDiskQuota([row.id], 0)
  showResult(res.data)
  await loadList()
}

async function onResetBwOne(row: UserListItem) {
  const res = await setUserBandwidthQuota([row.id], 0)
  showResult(res.data)
  await loadList()
}

loadList()
</script>

<style scoped>
.account-mng {
  padding: 4px;
}
.page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  margin-bottom: 12px;
}
.page-title {
  margin: 0;
  font-size: 18px;
}
.page-sub {
  margin: 4px 0 0;
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.sel-hint {
  color: var(--el-color-primary);
  font-size: 13px;
}
.user-cell {
  display: flex;
  flex-direction: column;
  line-height: 1.3;
}
.uname {
  font-weight: 600;
}
.nick {
  color: var(--el-text-color-secondary);
}
.php-tag {
  margin-right: 4px;
}
.muted {
  color: var(--el-text-color-secondary);
}
.result-msg {
  margin-left: 8px;
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
</style>
