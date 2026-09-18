<template>
  <el-drawer v-model="visible" :title="t('docker.image.detailTitle')" size="60%">
    <div v-loading="loading" class="detail">
      <template v-if="summary">
        <el-descriptions :column="2" border size="small" class="detail-desc">
          <el-descriptions-item :label="t('docker.image.name')">
            <span class="mono">{{ summary.tags[0] || t('docker.common.empty') }}</span>
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.tag')">
            <el-space wrap size="small">
              <el-tag v-for="tg in summary.tags" :key="tg" size="small" effect="plain" type="success">
                {{ tagOf(tg) }}
              </el-tag>
              <span v-if="!summary.tags.length" class="cell-muted">—</span>
            </el-space>
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.id')">
            <span class="mono">{{ shortId(summary.id) }}</span>
            <el-tooltip :content="t('docker.image.copyId')" placement="top">
              <el-button link size="small" class="copy-btn" :icon="Copy" @click="copy(summary.id)" />
            </el-tooltip>
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.common.size')">
            {{ summary.sizeText }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.age')">
            {{ ageText || '—' }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.layers')">
            {{ summary.layers }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.arch')">
            {{ summary.os }}/{{ summary.architecture }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.dockerVersion')">
            {{ summary.dockerVersion || '—' }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.author')" :span="2">
            {{ summary.author || '—' }}
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.cmd')" :span="2">
            <span class="mono">{{ summary.cmd || '—' }}</span>
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.entrypoint')" :span="2">
            <span class="mono">{{ summary.entrypoint || '—' }}</span>
          </el-descriptions-item>
          <el-descriptions-item :label="t('docker.image.workdir')" :span="2">
            <span class="mono">{{ summary.workdir || '—' }}</span>
          </el-descriptions-item>
          <el-descriptions-item v-if="summary.comment" :label="t('docker.image.comment')" :span="2">
            {{ summary.comment }}
          </el-descriptions-item>
        </el-descriptions>

        <div v-if="summary.env.length" class="detail-block">
          <div class="block-title">{{ t('docker.image.env') }}</div>
          <div class="env-list">
            <div v-for="e in summary.env" :key="e" class="mono env-item">{{ e }}</div>
          </div>
        </div>

        <div v-if="portList.length" class="detail-block">
          <div class="block-title">{{ t('docker.image.exposedPorts') }}</div>
          <el-space wrap size="small">
            <el-tag v-for="p in portList" :key="p" size="small" effect="plain">{{ p }}</el-tag>
          </el-space>
        </div>

        <div v-if="labelList.length" class="detail-block">
          <div class="block-title">{{ t('docker.image.labels') }}</div>
          <div class="env-list">
            <div v-for="l in labelList" :key="l" class="mono env-item">{{ l }}</div>
          </div>
        </div>
      </template>

      <el-tabs v-if="!loading && inspectText" v-model="tab" class="detail-tabs">
        <el-tab-pane :label="t('docker.image.history')" name="history">
          <!-- 逆序展示：docker history 的输出是「最外层在最上面」，留意 before 的数据本身已是该顺序 -->
          <el-table :data="history" size="small" max-height="340px" row-key="Id">
            <el-table-column :label="t('docker.image.historyId')" width="110">
              <template #default="{ row }">
                <span class="mono">{{ shortId(row.Id) }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('docker.common.created')" width="170">
              <template #default="{ row }">{{ row.CreatedAt || '—' }}</template>
            </el-table-column>
            <el-table-column :label="t('docker.image.createdBy')" min-width="260">
              <template #default="{ row }">
                <span class="mono wrap-any">{{ row.CreatedBy || '—' }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('docker.common.size')" width="100">
              <template #default="{ row }">{{ row.SizeText }}</template>
            </el-table-column>
          </el-table>
        </el-tab-pane>
        <el-tab-pane :label="t('docker.inspect.title')" name="inspect">
          <div class="inspect-toolbar">
            <el-button size="small" :icon="Copy" @click="copy(inspectText)">
              {{ t('docker.inspect.copy') }}
            </el-button>
          </div>
          <pre class="inspect-json">{{ inspectText }}</pre>
        </el-tab-pane>
      </el-tabs>

      <el-empty v-if="!loading && !summary" :description="t('docker.common.empty')" :image-size="60" />
    </div>
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Copy } from '@/icons'
import { imageInspect, type DockerImageHistoryItem, type DockerImageSummary } from '@/api/docker'

/**
 * 镜像详情抽屉：镜像名 / ID / tag / size / age / layers + 构建历史 + inspect 原文。
 *
 * 后端一次返回三段内容（`DockerImageInspect`），这里只做展示，
 * 不再为每个字段单独发一次请求 —— inspect 本身也不小，重复请求不划算。
 */
const props = defineProps<{ modelValue: boolean; imageId: string }>()
const emit = defineEmits<{ 'update:modelValue': [boolean] }>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const loading = ref(false)
const tab = ref('history')
const summary = ref<DockerImageSummary | null>(null)
const history = ref<DockerImageHistoryItem[]>([])
const inspectText = ref('')

/** `repo:tag` → tag（没有冒号则按 latest 展示，与 docker 的默认行为一致） */
function tagOf(ref_: string): string {
  const last = ref_.split('/').pop() || ref_
  return last.includes(':') ? last.slice(last.lastIndexOf(':') + 1) : 'latest'
}

function shortId(id: string): string {
  return id?.replace('sha256:', '').slice(0, 12) || ''
}

/** 相对时间（拉取 / 构建距今多久） */
const ageText = computed(() => {
  const raw = summary.value?.created
  if (!raw) return ''
  const ts = Date.parse(raw)
  if (Number.isNaN(ts)) return raw
  const sec = Math.max(0, Math.floor((Date.now() - ts) / 1000))
  const units: [number, string][] = [
    [86400, t('docker.age.days')],
    [3600, t('docker.age.hours')],
    [60, t('docker.age.minutes')],
  ]
  for (const [size, unit] of units) {
    if (sec >= size) return `${Math.floor(sec / size)}${unit}`
  }
  return `${sec}${t('docker.age.seconds')}`
})

/** ExposedPorts 的 key 形如 `80/tcp`，直接展示即可 */
const portList = computed(() => Object.keys(summary.value?.exposedPorts || {}))

const labelList = computed(() => {
  const labels = summary.value?.labels || {}
  return Object.entries(labels).map(([k, v]) => `${k}=${v}`)
})

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success(t('docker.inspect.copied'))
  } catch {
    ElMessage.warning(t('docker.image.copyFailed'))
  }
}

async function load() {
  if (!props.imageId) return
  loading.value = true
  summary.value = null
  history.value = []
  inspectText.value = ''
  try {
    const resp = await imageInspect(props.imageId)
    summary.value = resp.data?.summary ?? null
    history.value = resp.data?.history ?? []
    inspectText.value = JSON.stringify(resp.data?.inspect ?? {}, null, 2)
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.loadFailed'))
  } finally {
    loading.value = false
  }
}

watch(visible, (v) => {
  if (v) {
    tab.value = 'history'
    load()
  }
})
</script>

<style scoped>
.detail-desc {
  margin-bottom: 14px;
}

.detail-block {
  margin: 12px 0;
}

.block-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 6px;
  color: var(--el-text-color-regular);
}

.env-list {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 8px 10px;
  max-height: 160px;
  overflow: auto;
  background: var(--el-fill-color-light);
}

.env-item {
  font-size: 12px;
  line-height: 1.7;
  word-break: break-all;
}

.detail-tabs {
  margin-top: 6px;
}

.inspect-toolbar {
  margin-bottom: 8px;
}

.inspect-json {
  margin: 0;
  padding: 12px;
  max-height: calc(100vh - 300px);
  overflow: auto;
  background: var(--el-fill-color-light);
  border-radius: 6px;
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.6;
}

.copy-btn {
  height: auto;
  padding: 0 2px;
}

.cell-muted {
  color: var(--el-text-color-placeholder);
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}

.wrap-any {
  word-break: break-all;
  white-space: normal;
}
</style>
