<template>
  <el-card shadow="never" v-loading="loading">
    <template #header>
      <div class="card-header">
        <span>{{ t('serverEnv.pythonTitle') }}</span>
        <div class="card-actions">
          <el-button :icon="Refresh" size="small" :loading="refreshing" @click="reprobe">
            {{ t('common.refresh') }}
          </el-button>
        </div>
      </div>
      <div class="card-sub">{{ t('serverEnv.pythonSub') }}</div>
    </template>

    <!-- uv 状态 -->
    <el-alert
      v-if="!py?.uv"
      type="warning"
      :closable="false"
      show-icon
      :title="t('serverEnv.pythonUvMissing')"
    >
      <div class="uv-cmd">curl -LsSf https://astral.sh/uv/install.sh | sh</div>
      <div class="uv-hint">{{ t('serverEnv.pythonUvMissingHint') }}</div>
    </el-alert>
    <div v-else class="uv-ok">
      <el-tag type="success" size="small">uv</el-tag>
      <span class="uv-ver">{{ py.uv_version || '--' }}</span>
      <span class="uv-path">{{ py.uv_path }}</span>
    </div>

    <!-- 版本列表 -->
    <el-table :data="versions" size="small" class="py-table" empty-text="--">
      <el-table-column :label="t('serverEnv.pythonColVersion')" width="120">
        <template #default="{ row }">
          <span class="py-ver">{{ row.version }}</span>
        </template>
      </el-table-column>
      <el-table-column :label="t('serverEnv.pythonColSource')" width="110">
        <template #default="{ row }">
          <el-tag size="small" :type="row.source === 'uv' ? 'primary' : 'info'">
            {{ row.source === 'uv' ? 'uv' : t('serverEnv.pythonSourceSystem') }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('serverEnv.pythonColPath')" min-width="220">
        <template #default="{ row }">
          <span class="py-path">{{ row.path || '--' }}</span>
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
            {{ t('serverEnv.pythonUninstall') }}
          </el-button>
          <el-button
            link
            type="primary"
            size="small"
            :loading="busyDefault === row.version"
            @click="setDefault(row.version)"
          >
            {{ isDefault(row.version) ? t('serverEnv.pythonIsDefault') : t('serverEnv.pythonSetDefault') }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>

    <!-- 安装新版本 -->
    <div class="install-row">
      <el-input
        v-model="newVersion"
        size="small"
        class="ver-input"
        :placeholder="t('serverEnv.pythonVersionPlaceholder')"
        @keydown.enter.prevent="installVersion"
      />
      <el-button
        type="primary"
        size="small"
        :loading="installing"
        :disabled="!newVersion.trim()"
        @click="installVersion"
      >
        {{ t('serverEnv.pythonInstall') }}
      </el-button>
    </div>
    <div class="hint-text">{{ t('serverEnv.pythonHint') }}</div>
  </el-card>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Refresh } from '@/icons'
import { http } from '@/utils/request'
import { getServerEnv, refreshServerEnv, saveServerEnvDefaults } from '@/api/serverEnv'

const { t } = useI18n()

const loading = ref(false)
const refreshing = ref(false)
const installing = ref(false)
const newVersion = ref('')
const busyVersion = ref('')
const busyDefault = ref('')
const env = ref<any>(null)

const py = computed(() => env.value?.payload?.python ?? null)
const versions = computed<any[]>(() => py.value?.versions ?? [])
const currentDefault = computed(() => env.value?.conf?.python_default ?? '')

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
    ElMessage.success(t('serverEnv.pythonReprobeDone'))
  } catch (e: any) {
    ElMessage.error(e?.message || t('common.system'))
  } finally {
    refreshing.value = false
  }
}

async function installVersion() {
  const v = newVersion.value.trim()
  if (!v) return
  installing.value = true
  try {
    await http.post('/system/env/python/install', { version: v })
    ElMessage.success(t('serverEnv.pythonInstallOk', { version: v }))
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
    await http.post('/system/env/python/remove', { version: v })
    ElMessage.success(t('serverEnv.pythonRemoveOk', { version: v }))
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
    ElMessage.success(t('serverEnv.pythonDefaultOk', { version: v }))
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
export default { name: 'ServerEnvPython' }
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
.card-actions {
  display: flex;
  gap: 8px;
}
.uv-ok {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  font-size: 13px;
}
.uv-ver {
  font-weight: 600;
}
.uv-path,
.py-path {
  font-family: var(--el-font-family-mono, monospace);
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.uv-cmd {
  margin-top: 6px;
  font-family: var(--el-font-family-mono, monospace);
  font-size: 12px;
}
.uv-hint {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.py-table {
  margin-top: 12px;
}
.py-ver {
  font-weight: 600;
}
.install-row {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}
.ver-input {
  max-width: 200px;
}
.hint-text {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
