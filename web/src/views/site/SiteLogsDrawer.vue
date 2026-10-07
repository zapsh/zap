<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-drawer
    v-model="visible"
    :title="`${t('site.logs')} — ${siteName}`"
    size="72%"
    :close-on-click-modal="false"
    destroy-on-close
    class="site-drawer"
  >
    <div class="log-toolbar">
      <el-radio-group v-model="kind" @change="onKindChange">
        <el-radio-button value="access">{{ t('site.logAccess') }}</el-radio-button>
        <el-radio-button value="error">{{ t('site.logError') }}</el-radio-button>
        <el-radio-button value="waf">{{ t('site.logWaf') }}</el-radio-button>
      </el-radio-group>

      <el-select v-model="archive" style="width: 220px" @change="loadLogs">
        <el-option :label="t('site.logCurrent')" value="" />
        <el-option v-for="a in archivesOfKind" :key="a.name" :label="a.name" :value="a.name" />
      </el-select>

      <el-input
        v-model="keyword"
        :placeholder="t('site.logKeyword')"
        clearable
        style="width: 180px"
        @keyup.enter="loadLogs"
        @clear="loadLogs"
      />
      <el-input
        v-if="kind === 'access'"
        v-model="status"
        :placeholder="t('site.logStatus')"
        clearable
        style="width: 110px"
        @keyup.enter="loadLogs"
        @clear="loadLogs"
      />
      <el-select v-model="lines" style="width: 120px" @change="loadLogs">
        <el-option
          v-for="n in [200, 500, 1000, 2000]"
          :key="n"
          :label="`${n} ${t('site.logLines')}`"
          :value="n"
        />
      </el-select>

      <el-button :icon="Refresh" @click="loadLogs">{{ t('common.refresh') }}</el-button>
      <el-button :icon="Download" :disabled="!content" @click="download">
        {{ t('site.logDownload') }}
      </el-button>
      <el-button plain @click="rotate">{{ t('site.logRotate') }}</el-button>
      <el-button type="danger" plain :icon="Delete" @click="clearLog">
        {{ t('site.logClear') }}
      </el-button>
    </div>

    <div class="log-meta">
      <span class="muted">{{ path || '-' }}</span>
      <span v-if="size" class="muted">{{ t('site.logSize', { size: formatBytes(size) }) }}</span>
      <span class="muted">{{ t('site.logCount', { n: lines_.length }) }}</span>
    </div>

    <!-- WAF 审计反查：error / waf 视图下可粘贴 error_log 里的 unique_id 查完整拦截原因 -->
    <div v-if="kind === 'error' || kind === 'waf'" class="log-audit-bar">
      <span class="muted tip">{{ t('site.logAuditTip') }}</span>
      <div class="log-audit-row">
        <el-input
          v-model="auditUid"
          :placeholder="t('site.logAuditPlaceholder')"
          clearable
          style="max-width: 360px"
          @keyup.enter="runAuditQuery"
        />
        <el-button type="primary" :loading="auditLoading" @click="runAuditQuery">
          {{ t('site.logAuditQuery') }}
        </el-button>
        <el-button v-if="auditDetail" @click="clearAudit">{{ t('site.logAuditBack') }}</el-button>
      </div>
    </div>

    <div v-if="auditDetail">
      <el-alert
        v-if="!auditDetail.found"
        type="info"
        :closable="false"
        :title="t('site.logAuditNotFound')"
      />
      <template v-else>
        <el-descriptions :column="2" border size="small">
          <el-descriptions-item :label="t('waf.auditReqLine')">
            {{ auditDetail.request?.line }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('waf.auditClient')">
            {{ auditDetail.header?.client_ip }}:{{ auditDetail.header?.client_port }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('waf.auditTime')">
            {{ auditDetail.header?.timestamp }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('waf.auditServer')">
            {{ auditDetail.header?.server_ip }}:{{ auditDetail.header?.server_port }}
          </el-descriptions-item>
        </el-descriptions>

        <div class="waf-audit-sub">{{ t('waf.auditRules') }}</div>
        <el-table :data="auditDetail.messages || []" size="small" border>
          <el-table-column prop="id" label="ID" width="110" />
          <el-table-column prop="action" :label="t('waf.auditAction')" width="200" />
          <el-table-column prop="severity" :label="t('waf.auditSeverity')" width="120" />
          <el-table-column prop="msg" :label="t('waf.auditMsg')" min-width="200" />
          <el-table-column :label="t('waf.auditRule')" min-width="220">
            <template #default="{ row }">{{ row.file }}:{{ row.line }}</template>
          </el-table-column>
          <el-table-column prop="data" :label="t('waf.auditData')" min-width="220" />
        </el-table>

        <el-collapse class="waf-audit-raw">
          <el-collapse-item :title="t('waf.auditRaw')">
            <pre class="mono">{{ auditDetail.raw }}</pre>
          </el-collapse-item>
        </el-collapse>
      </template>
    </div>
    <pre v-else class="log-view">{{ content || t('site.logEmpty') }}</pre>
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Delete, Download, Refresh } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { formatBytes } from '@/utils/fmt'
import type { SiteLogFile } from '@/api/site'
import {
  clearSiteLogs,
  getSiteLogArchives,
  getSiteLogAudit,
  getSiteLogs,
  rotateSiteLogs,
} from '@/api/site'
import type { WafAuditData } from '@/api/waf'

const props = defineProps<{
  modelValue: boolean
  siteId: number
  siteName: string
}>()
const emit = defineEmits<{ (e: 'update:modelValue', v: boolean): void }>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const kind = ref('access')
const archive = ref('')
const keyword = ref('')
const status = ref('')
const lines = ref(200)
const files = ref<SiteLogFile[]>([])
const lines_ = ref<string[]>([])
const path = ref('')
const size = ref(0)
const loading = ref(false)

const auditUid = ref('')
const auditLoading = ref(false)
const auditDetail = ref<WafAuditData | null>(null)

const archivesOfKind = computed(() => files.value.filter((f) => f.kind === kind.value))
const content = computed(() => lines_.value.join('\n'))

watch(
  () => props.modelValue,
  async (v) => {
    if (!v || !props.siteId) return
    await loadArchives()
    await loadLogs()
  },
)

async function loadArchives() {
  try {
    const res = await getSiteLogArchives(props.siteId)
    const d = res.data || ({} as any)
    files.value = [...(d.archives || [])]
  } catch {
    files.value = []
  }
}

async function loadLogs() {
  if (!props.siteId) return
  auditDetail.value = null
  loading.value = true
  try {
    const res = await getSiteLogs({
      id: props.siteId,
      kind: kind.value,
      archive: archive.value,
      lines: lines.value,
      keyword: keyword.value.trim(),
      status: status.value.trim(),
    })
    const d = (res.data || {}) as any
    lines_.value = d.lines || []
    path.value = d.path || ''
    size.value = Number(d.size || 0)
  } catch (e: any) {
    ElMessage.error(e.message || t('error.system'))
  } finally {
    loading.value = false
  }
}

function onKindChange() {
  // 归档按类型隔离，切换类型时回到当前日志
  archive.value = ''
  auditDetail.value = null
  auditUid.value = ''
  loadLogs()
}

function download() {
  const blob = new Blob([content.value], { type: 'text/plain;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${props.siteName}-${kind.value}.log`
  a.click()
  URL.revokeObjectURL(url)
}

async function clearLog() {
  try {
    await ElMessageBox.confirm(t('site.logClearConfirm'), t('site.logClear'), {
      type: 'warning',
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  try {
    // 归档日志只可查看，清空只作用于当前日志
    if (archive.value) {
      ElMessage.warning(t('site.logArchiveReadonly'))
      return
    }
    await clearSiteLogs(props.siteId, kind.value)
    ElMessage.success(t('site.logCleared'))
    await loadArchives()
    await loadLogs()
  } catch (e: any) {
    ElMessage.error(e.message || t('error.system'))
  }
}

async function rotate() {
  try {
    const res = await rotateSiteLogs(props.siteId)
    ElMessage.success(res.message || t('site.logRotated'))
    await loadArchives()
    await loadLogs()
  } catch (e: any) {
    ElMessage.error(e.message || t('error.system'))
  }
}

/** 按 error_log 里的 unique_id 反查单条 WAF 审计明细 */
async function runAuditQuery() {
  const uid = auditUid.value.trim()
  if (!uid) {
    ElMessage.warning(t('site.logAuditPlaceholder'))
    return
  }
  auditLoading.value = true
  auditDetail.value = null
  try {
    const res = await getSiteLogAudit(props.siteId, uid)
    auditDetail.value = (res.data || {}) as WafAuditData
  } catch (e: any) {
    ElMessage.error(e.message || t('error.system'))
  } finally {
    auditLoading.value = false
  }
}

function clearAudit() {
  auditDetail.value = null
  auditUid.value = ''
}
</script>

<style scoped>
.log-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
  margin-bottom: 10px;
}
.log-meta {
  display: flex;
  gap: 16px;
  margin-bottom: 6px;
  font-size: 12px;
}
.muted {
  color: var(--el-text-color-secondary);
}
.log-view {
  height: calc(100vh - 220px);
  margin: 0;
  padding: 10px 12px;
  overflow: auto;
  background: var(--el-fill-color-lighter);
  border-radius: 6px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre;
  color: var(--el-text-color-primary);
}
.log-audit-bar {
  margin-bottom: 10px;
}
.log-audit-bar .tip {
  display: block;
  margin-bottom: 6px;
  line-height: 1.5;
}
.log-audit-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
}
.waf-audit-sub {
  margin: 14px 0 6px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.waf-audit-raw {
  margin-top: 14px;
}
.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  white-space: pre-wrap;
  word-break: break-all;
  font-size: 12px;
  line-height: 1.6;
  margin: 0;
}
</style>
