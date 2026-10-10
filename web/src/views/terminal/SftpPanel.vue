<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="sftp-panel">
    <div class="sftp-bar">
      <el-button size="small" :disabled="loading" @click="goUp">
        {{ t('terminal.sftpUp') }}
      </el-button>
      <el-input
        v-model="pathInput"
        size="small"
        class="sftp-path"
        :placeholder="t('terminal.sftpPathPlaceholder')"
        @keyup.enter="goPath"
      />
      <el-button size="small" :disabled="loading" @click="load">
        {{ t('terminal.sftpRefresh') }}
      </el-button>
      <el-button size="small" :disabled="loading" @click="doMkdir">
        {{ t('terminal.sftpMkdir') }}
      </el-button>
      <el-button size="small" type="primary" :disabled="loading" @click="pickFile">
        {{ t('terminal.sftpUpload') }}
      </el-button>
      <input ref="fileInput" type="file" class="sftp-file" @change="onPicked" />
    </div>

    <div class="sftp-body">
      <el-table :data="items" size="small" height="100%" @row-dblclick="onRowDblClick">
        <el-table-column :label="t('terminal.sftpName')" min-width="200">
          <template #default="{ row }">
            <span :class="row.is_dir ? 'sftp-dir' : 'sftp-file-name'">{{ row.name }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.sftpSize')" width="110">
          <template #default="{ row }">
            {{ row.is_dir ? '-' : formatSize(row.size) }}
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.sftpMtime')" width="170">
          <template #default="{ row }">
            {{ formatTime(row.mtime) }}
          </template>
        </el-table-column>
        <el-table-column :label="t('terminal.sftpAction')" width="200">
          <template #default="{ row }">
            <el-button link size="small" @click.stop="onRowDblClick(row)">
              {{ row.is_dir ? t('terminal.sftpOpen') : t('terminal.sftpDownload') }}
            </el-button>
            <el-button link size="small" @click.stop="doRename(row)">
              {{ t('terminal.sftpRename') }}
            </el-button>
            <el-button link size="small" type="danger" @click.stop="doRemove(row)">
              {{ t('terminal.sftpDelete') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <span>{{ loading ? t('terminal.sftpLoading') : t('terminal.sftpEmpty') }}</span>
        </template>
      </el-table>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import {
  sftpDownload,
  sftpList,
  sftpMkdir,
  sftpRemove,
  sftpRename,
  sftpUpload,
} from '@/api/terminal'
import type { SftpEntry } from '@/api/terminal'

const props = defineProps<{ connId: number }>()
const { t } = useI18n()

const items = ref<SftpEntry[]>([])
const path = ref('')
const pathInput = ref('')
const loading = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)

/** 空路径 = 远端登录用户的家目录，由后端 canonicalize 出绝对路径 */
async function load() {
  loading.value = true
  try {
    const resp = await sftpList(props.connId, path.value)
    items.value = resp.data?.items ?? []
    const cwd = resp.data?.cwd ?? ''
    // 首次加载（家目录）后端会回真实路径，之后就一直用绝对路径导航
    if (cwd) {
      path.value = cwd
      pathInput.value = cwd
    }
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('terminal.sftpLoadFailed'))
  } finally {
    loading.value = false
  }
}

function goPath() {
  path.value = pathInput.value.trim()
  load()
}

function goUp() {
  const trimmed = path.value.replace(/\/+$/, '')
  if (!trimmed || trimmed === '/') return
  const idx = trimmed.lastIndexOf('/')
  path.value = idx <= 0 ? '/' : trimmed.slice(0, idx)
  load()
}

/** 目录 → 进入；文件 → 下载 */
function onRowDblClick(row: SftpEntry) {
  if (row.is_dir) {
    path.value = row.path || joinPath(path.value, row.name)
    load()
  } else {
    doDownload(row)
  }
}

async function doDownload(row: SftpEntry) {
  try {
    const resp = await sftpDownload(props.connId, row.path)
    const content = resp.data?.content ?? ''
    const name = resp.data?.name || row.name
    const bin = atob(content)
    const bytes = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i)
    const url = URL.createObjectURL(new Blob([bytes]))
    const a = document.createElement('a')
    a.href = url
    a.download = name
    a.click()
    URL.revokeObjectURL(url)
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('terminal.sftpDownloadFailed'))
  }
}

function pickFile() {
  fileInput.value?.click()
}

async function onPicked(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  // 先清空，保证连续选同一个文件也会触发 change
  input.value = ''
  if (!file) return
  try {
    const buf = await file.arrayBuffer()
    const bytes = new Uint8Array(buf)
    // 分块转字符串，避免大文件下 String.fromCharCode 参数过多
    let bin = ''
    const chunk = 0x8000
    for (let i = 0; i < bytes.length; i += chunk) {
      bin += String.fromCharCode(...bytes.subarray(i, i + chunk))
    }
    await sftpUpload(props.connId, path.value, file.name, btoa(bin))
    ElMessage.success(t('terminal.sftpUploadDone'))
    load()
  } catch (err: unknown) {
    ElMessage.error((err as Error)?.message || t('terminal.sftpUploadFailed'))
  }
}

async function doMkdir() {
  try {
    const { value } = await ElMessageBox.prompt(
      t('terminal.sftpMkdirPrompt'),
      t('terminal.sftpMkdir'),
      {
        inputValidator: (v: string) => (v.trim() ? true : t('terminal.sftpNameRequired')),
      },
    )
    await sftpMkdir(props.connId, joinPath(path.value, String(value).trim()))
    ElMessage.success(t('terminal.sftpMkdirDone'))
    load()
  } catch {
    // 取消或失败：prompt 取消会 reject，静默即可
  }
}

async function doRename(row: SftpEntry) {
  try {
    const { value } = await ElMessageBox.prompt(
      t('terminal.sftpRenamePrompt'),
      t('terminal.sftpRename'),
      {
        inputValue: row.name,
        inputValidator: (v: string) => (v.trim() ? true : t('terminal.sftpNameRequired')),
      },
    )
    const newName = String(value).trim()
    if (newName === row.name) return
    const idx = row.path.lastIndexOf('/')
    const dir = idx <= 0 ? '/' : row.path.slice(0, idx)
    await sftpRename(props.connId, row.path, joinPath(dir, newName))
    ElMessage.success(t('terminal.sftpRenameDone'))
    load()
  } catch {
    // 同上
  }
}

async function doRemove(row: SftpEntry) {
  try {
    await ElMessageBox.confirm(
      t('terminal.sftpRemoveConfirm', { name: row.name }),
      t('common.tip'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await sftpRemove(props.connId, row.path, row.is_dir)
    ElMessage.success(t('terminal.sftpRemoveDone'))
    load()
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('terminal.sftpRemoveFailed'))
  }
}

function joinPath(dir: string, name: string): string {
  if (!dir || dir === '.') return name
  return `${dir.replace(/\/+$/, '')}/${name}`
}

function formatSize(n: number): string {
  if (!n) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i += 1
  }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`
}

function formatTime(ts: number): string {
  if (!ts) return '-'
  return new Date(ts * 1000).toLocaleString()
}

// 切换连接时回到该连接的家目录
watch(
  () => props.connId,
  () => {
    path.value = ''
    pathInput.value = ''
    load()
  },
)

onMounted(load)
</script>

<style scoped>
.sftp-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.sftp-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

.sftp-path {
  flex: 1;
}

/* 文件选择框不占位，由「上传」按钮触发 */
.sftp-file {
  display: none;
}

.sftp-body {
  flex: 1;
  min-height: 0;
}

.sftp-dir {
  font-weight: 600;
}
</style>
