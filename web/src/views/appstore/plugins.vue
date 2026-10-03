<template>
  <div class="plugin-page">
    <!-- 页头 -->
    <el-card shadow="never" class="head-card">
      <div class="head-row">
        <div class="head-left">
          <el-icon :size="22" color="#409eff"><Box /></el-icon>
          <div>
            <div class="head-title">{{ t('devPlugins.headTitle') }}</div>
            <div class="head-sub">{{ t('devPlugins.headSub') }}</div>
          </div>
        </div>
        <div class="head-right">
          <el-button :icon="Upload" @click="openUpload">{{ t('devPlugins.installUpload') }}</el-button>
          <el-button :icon="Refresh" circle :loading="loading" @click="load" />
        </div>
      </div>
    </el-card>

    <!-- 插件统一装在系统目录，所有用户共享；不再区分「我的」 -->

    <!-- 列表 -->
    <el-card shadow="never" class="table-card">
      <el-table :data="visibleRows" v-loading="loading" stripe>
        <el-table-column :label="t('devPlugins.colPlugin')" min-width="220">
          <template #default="{ row }">
            <div class="cell-name">{{ row.title || row.name }}</div>
            <div class="cell-sub">
              <code>{{ row.name }}</code>
              <span v-if="row.version" class="ver">v{{ row.version }}</span>
            </div>
            <div v-if="row.description" class="cell-desc">{{ row.description }}</div>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colLevel')" width="120">
          <template #default="{ row }">
            <el-tooltip :content="t('devPlugins.levelSystemTip')" placement="top">
              <el-tag type="warning" effect="plain" size="small">
                {{ t('devPlugins.levelSystem') }}
              </el-tag>
            </el-tooltip>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colScope')" width="110">
          <template #default="{ row }">
            <el-tag :type="row.scope === 'system' ? 'danger' : 'success'" effect="plain" size="small">
              {{ row.scope === 'system' ? t('devPlugins.scopeSystem') : t('devPlugins.scopeSite') }}
            </el-tag>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colPlacement')" width="130">
          <template #default="{ row }">
            <span class="mono">{{ row.placement || '-' }}</span>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colSource')" min-width="180">
          <template #default="{ row }">
            <div class="cell-sub">
              <span>{{ sourceLabel(row) }}</span>
              <span v-if="row.installed_at" class="dim">{{ fmtTime(row.installed_at) }}</span>
            </div>
            <div v-if="row.src && row.source === 'git'" class="cell-desc mono ellipsis">
              {{ row.src }}
            </div>
          </template>
        </el-table-column>

        <el-table-column :label="t('devPlugins.colActions')" width="90" align="right">
          <template #default="{ row }">
            <el-button
              link
              type="danger"
              size="small"
              :disabled="row.level === 'system' && !isAdmin"
              @click="onUninstall(row)"
            >
              {{ t('devPlugins.uninstall') }}
            </el-button>
          </template>
        </el-table-column>

        <template #empty>
          <el-empty :description="t('devPlugins.empty')" :image-size="60" />
        </template>
      </el-table>
    </el-card>

    <!-- 上传安装 -->
    <el-dialog v-model="uploadVisible" :title="t('devPlugins.uploadTitle')" width="520px">
      <el-form label-width="110px">
        <el-form-item :label="t('devPlugins.uploadLevel')">
          <span class="form-tip">{{ t('devPlugins.levelSystemTip') }}</span>
          <div v-if="!isAdmin" class="form-tip">{{ t('devPlugins.adminOnly') }}</div>
        </el-form-item>
        <el-form-item :label="t('devPlugins.uploadName')">
          <el-input
            v-model="uploadForm.name"
            :placeholder="t('devPlugins.uploadNamePlaceholder')"
          />
        </el-form-item>
        <el-form-item :label="t('devPlugins.uploadFile')">
          <div class="file-row">
            <input ref="fileInputRef" type="file" class="hidden-input" @change="onFilePick" />
            <el-input :model-value="uploadForm.file?.name || ''" readonly style="flex: 1">
              <template #append>
                <el-button @click="fileInputRef?.click()">{{ t('devPlugins.uploadPick') }}</el-button>
              </template>
            </el-input>
          </div>
          <div class="form-tip">{{ t('devPlugins.uploadTip') }}</div>
        </el-form-item>
        <el-form-item>
          <el-checkbox v-model="uploadForm.force">
            {{ t('devPlugins.uploadForce') }}
          </el-checkbox>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="uploadVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="submitting" @click="submitUpload">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Box, Refresh, Upload } from '@/icons'
import {
  pluginInstallUpload,
  pluginList,
  pluginUninstall,
  type PluginInfo,
} from '@/api/plugin'
import { useUserStore } from '@/stores/user'

const { t } = useI18n()
const userStore = useUserStore()

const rows = ref<PluginInfo[]>([])
const loading = ref(false)
const submitting = ref(false)

// 插件统一装在系统目录，所有用户共享；直接展示全部
const visibleRows = computed(() => rows.value)

// 系统级插件会以 root 身份运行，只有 admin 能装 / 卸；前端先拦一道，后端再兜一次
const isAdmin = computed(() => (userStore.roles || []).includes('admin'))

async function load() {
  loading.value = true
  try {
    // 不传 slot：管理页要看全部插件，不按入口位置过滤
    const r: any = await pluginList({})
    const data = r?.data?.data ?? r?.data ?? []
    rows.value = Array.isArray(data) ? data : []
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  } finally {
    loading.value = false
  }
}

function sourceLabel(row: PluginInfo) {
  if (row.source === 'git') return t('devPlugins.sourceGit')
  if (row.source === 'appstore') return t('devPlugins.sourceAppstore')
  if (row.source === 'archive') return t('devPlugins.sourceArchive')
  return t('devPlugins.sourceManual')
}

function fmtTime(ts: number) {
  if (!ts) return ''
  const d = new Date(ts * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

// ── 上传安装 ────────────────────────────────────────────────
const uploadVisible = ref(false)
const fileInputRef = ref<HTMLInputElement | null>(null)
const uploadForm = ref<{
  level: 'system'
  force: boolean
  name: string
  file: File | null
}>({ level: 'system', force: false, name: '', file: null })

function openUpload() {
  uploadForm.value = { level: 'system', force: false, name: '', file: null }
  uploadVisible.value = true
}

function onFilePick(e: Event) {
  const input = e.target as HTMLInputElement
  uploadForm.value.file = input.files?.[0] || null
}

async function submitUpload() {
  if (!uploadForm.value.file) {
    ElMessage.warning(t('devPlugins.uploadNoFile'))
    return
  }
  if (!isAdmin.value) {
    ElMessage.warning(t('devPlugins.adminOnly'))
    return
  }
  submitting.value = true
  try {
    await pluginInstallUpload({
      file: uploadForm.value.file,
      level: uploadForm.value.level,
      force: uploadForm.value.force,
      name: uploadForm.value.name,
    })
    ElMessage.success(t('devPlugins.installOk'))
    uploadVisible.value = false
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  } finally {
    submitting.value = false
  }
}

// ── 卸载 ────────────────────────────────────────────────────
async function onUninstall(row: PluginInfo) {
  if (row.level === 'system' && !isAdmin.value) {
    ElMessage.warning(t('devPlugins.adminOnly'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('devPlugins.uninstallConfirm', { name: row.name }),
      t('devPlugins.uninstallTitle'),
      { type: 'warning', confirmButtonText: t('devPlugins.uninstall'), cancelButtonText: t('common.cancel') },
    )
  } catch {
    return
  }
  try {
    await pluginUninstall({ name: row.name, level: row.level || 'system' })
    ElMessage.success(t('devPlugins.uninstallOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || String(e))
  }
}

onMounted(load)
</script>

<style scoped>
.plugin-page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.plugin-tabs {
  align-self: flex-start;
}
.head-card :deep(.el-card__body) {
  padding: 14px 16px;
}
.head-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
.head-left {
  display: flex;
  align-items: center;
  gap: 12px;
}
.head-title {
  font-size: 17px;
  font-weight: 600;
}
.head-sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.head-right {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cell-name {
  font-weight: 600;
}
.cell-sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  display: flex;
  gap: 8px;
  align-items: center;
}
.cell-desc {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.ver {
  color: var(--el-color-primary);
}
.dim {
  color: var(--el-text-color-placeholder);
}
.mono {
  font-family: var(--el-font-family-mono, monospace);
  font-size: 12px;
}
.ellipsis {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-row {
  display: flex;
  align-items: center;
  width: 100%;
}
.hidden-input {
  display: none;
}
.form-tip {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
}
</style>
