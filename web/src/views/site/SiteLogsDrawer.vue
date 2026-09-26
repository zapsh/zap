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

    <pre class="log-view">{{ content || t('site.logEmpty') }}</pre>
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Delete, Download, Refresh } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { formatBytes } from '@/utils/fmt'
import type { SiteLogFile } from '@/api/site'
import { clearSiteLogs, getSiteLogArchives, getSiteLogs, rotateSiteLogs } from '@/api/site'

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
</style>
