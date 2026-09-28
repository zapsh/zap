<template>
  <el-card shadow="never" v-loading="loading">
    <template #header>
      <div class="card-header">
        <span>{{ t('serviceConf.pythonTitle') }}</span>
        <el-button :icon="Refresh" size="small" :loading="refreshing" @click="reprobe">
          {{ t('common.refresh') }}
        </el-button>
      </div>
      <div class="card-sub">{{ t('serviceConf.pythonSub') }}</div>
    </template>

    <!-- uv -->
    <div class="block">
      <div class="block-title">{{ t('serviceConf.pythonUv') }}</div>
      <el-alert
        v-if="!py?.uv"
        type="warning"
        :closable="false"
        show-icon
        :title="t('serviceConf.pythonUvMissing')"
      >
        <div class="hint">{{ t('serviceConf.pythonUvMissingHint') }}</div>
      </el-alert>
      <div v-else class="row">
        <el-tag type="success" size="small">uv</el-tag>
        <span class="mono">{{ py.uv_version || '--' }}</span>
        <span class="mono dim">{{ py.uv_path }}</span>
      </div>
      <div class="row">
        <span class="dim">{{ t('serviceConf.mirror') }}</span>
        <el-select v-model="mirrorKey" size="small" class="index-select" @change="applyMirror">
          <el-option :label="t('serviceConf.mirrorOfficial')" value="official" />
          <el-option :label="t('serviceConf.mirrorChina')" value="china" />
        </el-select>
      </div>
      <div class="hint">{{ t('serviceConf.mirrorHint') }}</div>
      <div class="row">
        <el-button
          v-if="!py?.uv"
          type="primary"
          size="small"
          :loading="installingUv"
          @click="installUv"
        >
          {{ t('serviceConf.pythonInstallUv') }}
        </el-button>
        <span v-else class="dim">{{ t('serviceConf.pythonUvShared') }}</span>
      </div>
    </div>

    <!-- PyPI 源 -->
    <div class="block">
      <div class="block-title">{{ t('serviceConf.pythonIndex') }}</div>
      <div class="row">
        <el-select v-model="indexKey" size="small" class="index-select">
          <el-option
            v-for="o in indexOptions"
            :key="o.key"
            :label="o.label"
            :value="o.key"
          />
        </el-select>
        <el-input
          v-if="indexKey === 'custom'"
          v-model="customUrl"
          size="small"
          class="index-input"
          placeholder="https://..."
        />
        <el-button
          type="primary"
          size="small"
          :loading="savingIndex"
          @click="applyIndex"
        >
          {{ t('serviceConf.pythonIndexApply') }}
        </el-button>
      </div>
      <div class="hint">
        {{ t('serviceConf.pythonIndexCurrent', { url: py?.index_url || '--' }) }}
      </div>
    </div>

    <!-- 版本 -->
    <div class="block">
      <div class="block-title">{{ t('serviceConf.pythonVersions') }}</div>
      <el-table :data="versions" size="small" empty-text="--">
        <el-table-column :label="t('serviceConf.rtColVersion')" width="120">
          <template #default="{ row }">
            <span class="ver">{{ row.version }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('serviceConf.rtColSource')" width="110">
          <template #default="{ row }">
            <el-tag size="small" :type="row.source === 'uv' ? 'primary' : 'info'">
              {{ row.source === 'uv' ? 'uv' : t('serviceConf.rtSourceSystem') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('serviceConf.rtColPath')" min-width="220">
          <template #default="{ row }">
            <span class="mono dim">{{ row.path || '--' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.action')" width="200" align="right">
          <template #default="{ row }">
            <el-button
              v-if="row.source === 'uv'"
              link
              type="danger"
              size="small"
              :loading="busyVersion === row.version"
              @click="removeVersion(row.version)"
            >
              {{ t('serviceConf.rtUninstall') }}
            </el-button>
            <el-button
              link
              type="primary"
              size="small"
              :loading="busyDefault === row.version"
              @click="setDefault(row.version)"
            >
              {{
                isDefault(row.version)
                  ? t('serviceConf.rtIsDefault')
                  : t('serviceConf.rtSetDefault')
              }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>

      <div class="row">
        <el-input
          v-model="newVersion"
          size="small"
          class="ver-input"
          :placeholder="t('serviceConf.pythonVersionPlaceholder')"
          @keydown.enter.prevent="installVersion"
        />
        <el-button
          type="primary"
          size="small"
          :loading="installing"
          :disabled="!newVersion.trim()"
          @click="installVersion"
        >
          {{ t('serviceConf.rtInstall') }}
        </el-button>
      </div>
      <div class="hint">{{ t('serviceConf.pythonHint') }}</div>
    </div>
  </el-card>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Refresh } from '@/icons'
import {
  getServerEnv,
  installEnvUv,
  installPythonVersion,
  refreshServerEnv,
  removePythonVersion,
  saveServerEnvDefaults,
  setPythonIndex,
} from '@/api/serverEnv'

const { t } = useI18n()

const loading = ref(false)
const refreshing = ref(false)
const installingUv = ref(false)
const installing = ref(false)
const savingIndex = ref(false)
const newVersion = ref('')
const busyVersion = ref('')
const busyDefault = ref('')
const env = ref<any>(null)

/** 源预设：默认官方，国内源可选 */
const indexOptions = computed(() => [
  { key: 'official', label: t('serviceConf.pyIndexOfficial'), url: 'https://pypi.org/simple' },
  { key: 'tuna', label: t('serviceConf.pyIndexTuna'), url: 'https://pypi.tuna.tsinghua.edu.cn/simple' },
  { key: 'aliyun', label: t('serviceConf.pyIndexAliyun'), url: 'https://mirrors.aliyun.com/pypi/simple' },
  { key: 'tencent', label: t('serviceConf.pyIndexTencent'), url: 'https://mirrors.cloud.tencent.com/pypi/simple' },
  { key: 'custom', label: t('serviceConf.pyIndexCustom'), url: '' },
])
const indexKey = ref('official')
const customUrl = ref('')
/** 下载镜像：official（默认直连）/ china（GitHub 与官方源慢时走国内镜像） */
const mirrorKey = ref('official')

watch(
  () => env.value?.conf?.download_mirror,
  (v) => {
    mirrorKey.value = v === 'china' ? 'china' : 'official'
  },
  { immediate: true },
)

const py = computed(() => env.value?.payload?.python ?? null)
const versions = computed<any[]>(() => py.value?.versions ?? [])
const currentDefault = computed(() => env.value?.conf?.python_default ?? '')

// 探测出的源如果不在预设里，就落到「自定义」
watch(
  () => py.value?.index_url,
  (url) => {
    if (!url) return
    const hit = indexOptions.value.find((o) => o.url && o.url === url)
    if (hit) {
      indexKey.value = hit.key
    } else {
      indexKey.value = 'custom'
      customUrl.value = url
    }
  },
  { immediate: true },
)

function isDefault(v: string) {
  const c = currentDefault.value
  if (!c) return false
  return v === c || v.startsWith(`${c}.`)
}

async function load() {
  loading.value = true
  try {
    const res = await getServerEnv()
    env.value = res.data
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    loading.value = false
  }
}

async function reprobe() {
  refreshing.value = true
  try {
    const res = await refreshServerEnv()
    env.value = res.data
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    refreshing.value = false
  }
}

/** 装 / 删运行时后必须用强制重测：GET 读的是最多 60s 陈旧的快照 */
async function hardRefresh() {
  try {
    const res = await refreshServerEnv()
    env.value = res.data
  } catch (e: any) {
    await load()
  }
}

async function installUv() {
  installingUv.value = true
  try {
    await installEnvUv()
    ElMessage.success(t('serviceConf.pythonUvOk'))
    await hardRefresh()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    installingUv.value = false
  }
}

async function applyMirror() {
  try {
    await saveServerEnvDefaults({ download_mirror: mirrorKey.value })
    ElMessage.success(t('serviceConf.mirrorOk'))
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  }
}

async function applyIndex() {
  const opt = indexOptions.value.find((o) => o.key === indexKey.value)
  const url = indexKey.value === 'custom' ? customUrl.value.trim() : (opt?.url ?? '')
  if (!url) {
    ElMessage.warning(t('serviceConf.pyIndexNeedUrl'))
    return
  }
  savingIndex.value = true
  try {
    await setPythonIndex(url)
    ElMessage.success(t('serviceConf.pythonIndexOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    savingIndex.value = false
  }
}

async function installVersion() {
  const v = newVersion.value.trim()
  if (!v) return
  installing.value = true
  try {
    await installPythonVersion(v)
    ElMessage.success(t('serviceConf.rtInstallOk', { version: v }))
    newVersion.value = ''
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    installing.value = false
  }
}

async function removeVersion(v: string) {
  busyVersion.value = v
  try {
    await removePythonVersion(v)
    ElMessage.success(t('serviceConf.rtRemoveOk', { version: v }))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    busyVersion.value = ''
  }
}

async function setDefault(v: string) {
  busyDefault.value = v
  try {
    await saveServerEnvDefaults({ python_default: v })
    ElMessage.success(t('serviceConf.rtDefaultOk', { version: v }))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    busyDefault.value = ''
  }
}

onMounted(load)
</script>

<script lang="ts">
export default { name: 'ServiceConfPython' }
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.card-sub {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.block {
  padding: 12px 0;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.block:last-child {
  border-bottom: none;
}
.block-title {
  margin-bottom: 8px;
  font-size: 13px;
  font-weight: 600;
}
.row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}
.mono {
  font-family: var(--el-font-family-mono, monospace);
  font-size: 12px;
}
.dim {
  color: var(--el-text-color-secondary);
}
.hint {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.ver {
  font-weight: 600;
}
.index-select {
  width: 200px;
}
.index-input {
  max-width: 320px;
}
.ver-input {
  max-width: 200px;
}
</style>
