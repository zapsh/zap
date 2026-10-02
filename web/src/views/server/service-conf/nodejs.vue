<template>
  <el-card shadow="never" v-loading="loading">
    <template #header>
      <div class="card-header">
        <span>{{ t('serviceConf.nodejsTitle') }}</span>
        <el-button :icon="Refresh" size="small" :loading="refreshing" @click="reprobe">
          {{ t('common.refresh') }}
        </el-button>
      </div>
      <div class="card-sub">{{ t('serviceConf.nodejsSub') }}</div>
    </template>

    <!-- fnm -->
    <div class="block">
      <div class="block-title">{{ t('serviceConf.nodejsFnm') }}</div>
      <el-alert
        v-if="!nj?.fnm"
        type="warning"
        :closable="false"
        show-icon
        :title="t('serviceConf.nodejsFnmMissing')"
      >
        <div class="hint">{{ t('serviceConf.nodejsFnmMissingHint') }}</div>
      </el-alert>
      <div v-else class="row">
        <el-tag type="success" size="small">fnm</el-tag>
        <span class="mono">{{ nj.fnm_version || '--' }}</span>
        <span class="mono dim">{{ nj.fnm_path }}</span>
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
          v-if="!nj?.fnm"
          type="primary"
          size="small"
          :loading="installingFnm"
          @click="installFnm"
        >
          {{ t('serviceConf.nodejsInstallFnm') }}
        </el-button>
        <span v-else class="dim">
          {{ t('serviceConf.nodejsGlobalDir', { dir: nj.fnm_dir || '/usr/local/fnm' }) }}
        </span>
      </div>
      <div v-if="nj?.fnm" class="hint">
        <el-tag size="small" :type="nj.global_link ? 'success' : 'warning'">
          {{ nj.global_link ? t('serviceConf.nodejsLinked') : t('serviceConf.nodejsNotLinked') }}
        </el-tag>
        <span class="hint-inline">{{ t('serviceConf.nodejsLinkHint') }}</span>
      </div>
    </div>

    <!-- npm 源 -->
    <div class="block">
      <div class="block-title">{{ t('serviceConf.npmRegistry') }}</div>
      <div class="row">
        <el-select v-model="regKey" size="small" class="index-select">
          <el-option :label="t('serviceConf.npmOfficial')" value="official" />
          <el-option :label="t('serviceConf.npmCn')" value="cn" />
          <el-option :label="t('serviceConf.npmCustom')" value="custom" />
        </el-select>
        <el-input
          v-if="regKey === 'custom'"
          v-model="customReg"
          size="small"
          class="index-input"
          placeholder="https://..."
        />
        <el-button
          type="primary"
          size="small"
          :loading="savingReg"
          @click="applyRegistry"
        >
          {{ t('serviceConf.npmRegistryApply') }}
        </el-button>
      </div>
      <div class="hint">
        {{ t('serviceConf.npmRegistryCurrent', { url: nj?.npm_registry || '--' }) }}
      </div>
    </div>

    <!-- 版本 -->
    <div class="block">
      <div class="block-title">{{ t('serviceConf.nodejsVersions') }}</div>
      <el-table :data="versions" size="small" empty-text="--">
        <el-table-column :label="t('serviceConf.rtColVersion')" width="140">
          <template #default="{ row }">
            <span class="ver">v{{ row.version }}</span>
            <el-tag v-if="isDefault(row.version)" size="small" type="success" class="def-tag">
              {{ t('serviceConf.rtIsDefault') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('serviceConf.rtColSource')" width="110">
          <template #default="{ row }">
            <el-tag size="small" :type="row.source === 'fnm' ? 'primary' : 'info'">
              {{ row.source === 'fnm' ? 'fnm' : t('serviceConf.rtSourceSystem') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('serviceConf.rtColPath')" min-width="240">
          <template #default="{ row }">
            <span class="mono dim">{{ row.path || '--' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.action')" width="220" align="right">
          <template #default="{ row }">
            <el-button
              v-if="row.source === 'fnm'"
              link
              type="danger"
              size="small"
              :loading="busyVersion === row.version"
              @click="act('uninstall', row.version)"
            >
              {{ t('serviceConf.rtUninstall') }}
            </el-button>
            <el-button
              v-if="row.source === 'fnm'"
              link
              type="primary"
              size="small"
              :loading="busyDefault === row.version"
              @click="act('default', row.version)"
            >
              {{ t('serviceConf.rtSetDefault') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>

      <div class="row">
        <el-input
          v-model="newVersion"
          size="small"
          class="ver-input"
          :placeholder="t('serviceConf.nodejsVersionPlaceholder')"
          @keydown.enter.prevent="act('install', newVersion)"
        />
        <el-button
          type="primary"
          size="small"
          :loading="busyVersion === '__new__'"
          :disabled="!newVersion.trim()"
          @click="act('install', newVersion)"
        >
          {{ t('serviceConf.rtInstall') }}
        </el-button>
      </div>
      <div class="hint">{{ t('serviceConf.nodejsHint') }}</div>
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
  installEnvFnm,
  nodejsAction,
  refreshServerEnv,
  saveServerEnvDefaults,
  setNodeRegistry,
} from '@/api/serverEnv'

defineOptions({ name: 'ServiceConfNodejs' })

const { t } = useI18n()

const loading = ref(false)
const refreshing = ref(false)
const installingFnm = ref(false)
const newVersion = ref('')
const busyVersion = ref('')
const busyDefault = ref('')
const env = ref<any>(null)

const nj = computed(() => env.value?.payload?.nodejs ?? null)
/** 下载镜像：official（默认）/ china（走国内镜像，GitHub 与 nodejs.org 慢时用它） */
const mirror = computed(() => env.value?.conf?.download_mirror || 'official')
const mirrorKey = ref('official')
const regKey = ref('official')
const customReg = ref('')
const savingReg = ref(false)

const NPM_OFFICIAL = 'https://registry.npmjs.org/'
const NPM_CN = 'https://registry.npmmirror.com/'

watch(
  () => env.value?.conf?.download_mirror,
  (v) => {
    mirrorKey.value = v === 'china' ? 'china' : 'official'
  },
  { immediate: true },
)

// 探测出的 registry 不在预设里就落到「自定义」
watch(
  () => nj.value?.npm_registry,
  (url) => {
    if (!url) return
    const norm = (u: string) => u.replace(/\/+$/, '')
    if (norm(url) === norm(NPM_OFFICIAL)) regKey.value = 'official'
    else if (norm(url) === norm(NPM_CN)) regKey.value = 'cn'
    else {
      regKey.value = 'custom'
      customReg.value = url
    }
  },
  { immediate: true },
)
const versions = computed<any[]>(() => nj.value?.versions ?? [])
const currentDefault = computed(
  () => env.value?.conf?.node_default || nj.value?.default || '',
)

function isDefault(v: string) {
  const c = currentDefault.value
  if (!c) return false
  return v === c || v.startsWith(`${c}.`) || c.startsWith(`${v}.`)
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

async function installFnm() {
  installingFnm.value = true
  try {
    await installEnvFnm(mirror.value)
    ElMessage.success(t('serviceConf.nodejsFnmOk'))
    await hardRefresh()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    installingFnm.value = false
  }
}

async function applyMirror() {
  try {
    await saveServerEnvDefaults({ download_mirror: mirrorKey.value })
    ElMessage.success(t('serviceConf.mirrorOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  }
}

async function applyRegistry() {
  const url =
    regKey.value === 'official'
      ? NPM_OFFICIAL
      : regKey.value === 'cn'
        ? NPM_CN
        : customReg.value.trim()
  if (!url) {
    ElMessage.warning(t('serviceConf.npmNeedUrl'))
    return
  }
  savingReg.value = true
  try {
    await setNodeRegistry(url)
    ElMessage.success(t('serviceConf.npmRegistryOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    savingReg.value = false
  }
}

async function act(action: string, rawVersion: string) {
  const v = String(rawVersion || '').trim()
  if (!v) return
  const isNew = action === 'install' && v === newVersion.value.trim()
  if (action === 'uninstall' || isNew) busyVersion.value = isNew ? '__new__' : v
  else busyDefault.value = v
  try {
    await nodejsAction(action, v, mirror.value)
    if (action === 'install') {
      ElMessage.success(t('serviceConf.rtInstallOk', { version: v }))
      newVersion.value = ''
    } else if (action === 'default') {
      // 同时记一份到全局默认，部署时未指定版本就用它
      await saveServerEnvDefaults({ node_default: v })
      ElMessage.success(t('serviceConf.rtDefaultOk', { version: v }))
    } else {
      ElMessage.success(t('serviceConf.rtRemoveOk', { version: v }))
    }
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    busyVersion.value = ''
    busyDefault.value = ''
  }
}

onMounted(load)
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
.hint-inline {
  margin-left: 6px;
}
.ver {
  font-weight: 600;
}
.def-tag {
  margin-left: 6px;
}
.ver-input {
  max-width: 200px;
}
</style>
