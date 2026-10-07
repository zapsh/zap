<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="audit-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>{{ t('audit.title') }}</span>
          <el-button :icon="Refresh" circle :title="t('common.refresh')" @click="handleSearch" />
        </div>
      </template>

      <!-- 筛选 -->
      <el-form :inline="true" :model="searchForm" @submit.prevent>
        <el-form-item :label="t('audit.operation')">
          <el-input
            v-model="searchForm.action"
            :placeholder="t('audit.actionPlaceholder')"
            clearable
            style="width: 220px"
            @keyup.enter="handleSearch"
          />
        </el-form-item>
        <el-form-item :label="t('audit.username')">
          <el-input
            v-model="searchForm.username"
            :placeholder="t('audit.usernamePlaceholder')"
            clearable
            style="width: 160px"
            @keyup.enter="handleSearch"
          />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" @click="handleSearch">{{ t('common.search') }}</el-button>
          <el-button @click="resetSearch">{{ t('common.reset') }}</el-button>
        </el-form-item>
      </el-form>

      <el-table :data="tableData" v-loading="loading" stripe>
        <el-table-column type="expand" width="42">
          <template #default="{ row }">
            <el-descriptions :column="2" border size="small" class="audit-detail">
              <el-descriptions-item :label="t('audit.operation')">
                {{ row.action }}
              </el-descriptions-item>
              <el-descriptions-item :label="t('audit.sourceIp')">
                {{ row.ip || '-' }}
              </el-descriptions-item>
              <el-descriptions-item :label="t('audit.target')" :span="2">
                <span class="mono">{{ row.target || '-' }}</span>
              </el-descriptions-item>
              <el-descriptions-item :label="t('audit.detail')" :span="2">
                <pre class="mono detail-pre">{{ row.detail || '-' }}</pre>
              </el-descriptions-item>
            </el-descriptions>
          </template>
        </el-table-column>
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column :label="t('audit.time')" width="170">
          <template #default="{ row }">{{ fmtTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column
          prop="username"
          :label="t('audit.username')"
          width="120"
          show-overflow-tooltip
        />
        <el-table-column :label="t('audit.operation')" min-width="150">
          <template #default="{ row }">
            <el-tag :type="actionTagType(row.action)" size="small" disable-transitions>
              {{ row.action }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('audit.target')" min-width="180" show-overflow-tooltip>
          <template #default="{ row }">{{ row.target || '-' }}</template>
        </el-table-column>
        <el-table-column prop="ip" :label="t('audit.sourceIp')" width="140">
          <template #default="{ row }">{{ row.ip || '-' }}</template>
        </el-table-column>
      </el-table>

      <el-pagination
        class="audit-pagination"
        v-model:current-page="page"
        v-model:page-size="pageSize"
        :total="total"
        :page-sizes="[20, 50, 100]"
        layout="total, sizes, prev, pager, next, jumper"
        @current-change="load"
        @size-change="load"
      />
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { reactive, ref, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import { getAuditLogList, type AuditLogItem } from '@/api/audit'
import { getLocale } from '@/i18n'

const { t } = useI18n()

const loading = ref(false)
const tableData = ref<AuditLogItem[]>([])
const total = ref(0)
const page = ref(1)
const pageSize = ref(20)

const searchForm = reactive({ action: '', username: '' })

async function load() {
  loading.value = true
  try {
    const res = await getAuditLogList({
      page: page.value,
      page_size: pageSize.value,
      action: searchForm.action || undefined,
      username: searchForm.username || undefined,
    })
    tableData.value = res.data || []
    total.value = res.total || 0
  } finally {
    loading.value = false
  }
}

function handleSearch() {
  page.value = 1
  load()
}

function resetSearch() {
  searchForm.action = ''
  searchForm.username = ''
  page.value = 1
  load()
}

/** 操作标签配色：失败/删除红、新增绿、修改黄、登录/登出默认主色、其余信息色 */
function actionTagType(action: string): '' | 'success' | 'info' | 'warning' | 'danger' {
  const a = (action || '').toLowerCase()
  if (/fail|error|denied|forbidden/.test(a)) return 'danger'
  if (/delete|remove|drop|disable/.test(a)) return 'danger'
  if (/create|add|new|install|enable|renew/.test(a)) return 'success'
  if (/update|edit|change|config|reset|upload|save/.test(a)) return 'warning'
  if (/login|logout|auth/.test(a)) return ''
  return 'info'
}

function fmtTime(ts: number): string {
  // 24 小时制与语言无关，locale 只影响日期顺序与月份等文案
  return ts ? new Date(ts * 1000).toLocaleString(getLocale(), { hour12: false }) : '-'
}

let timer: ReturnType<typeof setInterval> | undefined

onMounted(() => {
  load()
  // 每 60s 自动刷新一次，便于实时观察管理员/用户操作
  timer = setInterval(load, 60_000)
})

onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.audit-detail {
  margin: 8px 12px 8px 60px;
}
.mono {
  font-family: 'SFMono-Regular', Consolas, Menlo, monospace;
  font-size: 12px;
  word-break: break-all;
}
.detail-pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: 'SFMono-Regular', Consolas, Menlo, monospace;
  font-size: 12px;
  line-height: 1.6;
}
.audit-pagination {
  margin-top: 16px;
  justify-content: flex-end;
}
</style>
