<template>
  <el-dialog
    v-model="visible"
    :title="dialogTitle"
    width="640px"
    append-to-body
    :close-on-click-modal="false"
  >
    <div class="fp-head">
      <el-tag size="small" type="info" effect="plain">HOME</el-tag>
      <code class="fp-home">{{ home || '—' }}</code>
    </div>

    <div class="fp-toolbar">
      <el-button size="small" :disabled="!home || path === home" @click="fetch(home)">
        {{ t('filePicker.home') }}
      </el-button>
      <el-button size="small" :disabled="!canGoUp" @click="fetch(parentPath)">
        {{ t('filePicker.up') }}
      </el-button>
      <el-button size="small" :icon="Refresh" :disabled="!path" @click="fetch(path)">
        {{ t('filePicker.refresh') }}
      </el-button>
    </div>

    <el-input
      v-model="pathInput"
      size="small"
      class="fp-path"
      :placeholder="t('filePicker.pathPlaceholder')"
      @keydown.enter.prevent="jump"
    >
      <template #prefix>
        <el-icon><FolderOpened /></el-icon>
      </template>
    </el-input>

    <el-alert
      v-if="error"
      :title="error"
      type="error"
      :closable="false"
      show-icon
      class="fp-alert"
    />

    <div v-loading="loading" class="fp-body">
      <!-- 目录：点一下进下一层 -->
      <div v-for="d in dirs" :key="d.path" class="fp-item" @click="fetch(d.path)">
        <el-icon><FolderOpened /></el-icon>
        <span class="fp-item__name" :title="d.path">{{ d.name }}</span>
      </div>

      <!-- 文件 -->
      <label
        v-for="f in files"
        :key="f.path"
        class="fp-item fp-item--file"
        :class="{
          'is-selected': props.multiple
            ? selectedFiles.includes(f.path)
            : singleSelected === f.path,
        }"
      >
        <el-checkbox
          v-if="props.multiple"
          :model-value="selectedFiles.includes(f.path)"
          @click.prevent="toggle(f.path)"
        />
        <el-icon><Document /></el-icon>
        <span class="fp-item__name" :title="f.path" @click="onFileClick(f.path)">{{ f.name }}</span>
        <span class="fp-item__size">{{ sizeText(f.size) }}</span>
      </label>

      <el-empty
        v-if="!loading && !dirs.length && !files.length"
        :description="t('filePicker.empty')"
        :image-size="60"
      />
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ t('filePicker.cancel') }}</el-button>
      <el-button
        type="primary"
        :disabled="!hasSelection"
        :loading="confirmLoading"
        @click="confirm"
      >
        {{ resolvedConfirmText }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Document, FolderOpened, Refresh } from '@/icons'
import { listFiles, type FileEntry } from '@/api/file'
import { useUserStore } from '@/stores/user'

/**
 * 服务器文件选择窗口（插件 `file` / `files` 选项复用）。
 *
 * 单选（`multiple=false`）：点文件选中、双击直接确认；多选（`multiple=true`）：每个文件带勾选框。
 * 始终以**绝对路径数组**回传（`confirm: string[]`），单选也是长度 1 的数组，调用方按需取。
 *
 * 走文件管理那套 `/system/files/list`：起点是**当前用户的家目录**，管理员可一路向上（后端白名单兜底），
 * 普通用户出不了自己的 home —— 与文件管理、云存储「从服务器选择」同一套隔离规则。
 */
const props = withDefaults(
  defineProps<{
    modelValue: boolean
    title?: string
    /** 打开时的起始目录，留空即落到当前用户家目录 */
    startPath?: string
    multiple?: boolean
    confirmText?: string
    /** 确认按钮的 loading，由调用方的实际操作接管 */
    confirmLoading?: boolean
  }>(),
  { title: '', startPath: '', multiple: false, confirmText: '', confirmLoading: false },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: boolean): void
  (e: 'confirm', paths: string[]): void
}>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v),
})
const dialogTitle = computed(() => props.title || t('filePicker.title'))
const resolvedConfirmText = computed(() => {
  if (props.confirmText) return props.confirmText
  if (props.multiple) return `选择 ${selectedFiles.value.length} 个文件`
  return t('filePicker.confirm')
})

const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

const home = ref('')
const path = ref('')
const parentPath = ref('')
const pathInput = ref('')
const entries = ref<FileEntry[]>([])
const singleSelected = ref('')
const selectedFiles = ref<string[]>([])
const loading = ref(false)
const error = ref('')

const dirs = computed(() => entries.value.filter((e) => e.is_dir))
const files = computed(() => entries.value.filter((e) => !e.is_dir))
const hasSelection = computed(() =>
  props.multiple ? selectedFiles.value.length > 0 : !!singleSelected.value,
)

const canGoUp = computed(() => {
  const up = parentPath.value
  if (!path.value || !up || up === path.value) return false
  if (isAdmin.value) return true
  return !!home.value && (up === home.value || up.startsWith(`${home.value}/`))
})

function sizeText(size: number): string {
  if (!size) return ''
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / 1024 / 1024).toFixed(1)} MB`
}

async function fetch(target: string) {
  loading.value = true
  error.value = ''
  try {
    const res = await listFiles(target)
    const data = res.data
    if (data?.home) home.value = data.home
    path.value = data?.current_path || target
    pathInput.value = path.value
    parentPath.value = data?.parent_path || ''
    entries.value = data?.entries || []
  } catch (e: any) {
    error.value = e?.message || t('filePicker.loadFailed')
    entries.value = []
  } finally {
    loading.value = false
  }
}

/** 地址栏回车：路径没变就只是回填，避免白跑一次请求 */
function jump() {
  const target = pathInput.value.trim()
  if (!target || target === path.value) {
    pathInput.value = path.value
    return
  }
  fetch(target)
}

function onFileClick(p: string) {
  if (props.multiple) toggle(p)
  else singleSelected.value = p
}
function toggle(p: string) {
  const i = selectedFiles.value.indexOf(p)
  if (i >= 0) selectedFiles.value.splice(i, 1)
  else selectedFiles.value.push(p)
}

function confirm() {
  if (!hasSelection.value) return
  emit('confirm', props.multiple ? [...selectedFiles.value] : [singleSelected.value])
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) {
      singleSelected.value = ''
      selectedFiles.value = []
      fetch(props.startPath.trim())
    }
  },
)
</script>

<style scoped lang="scss">
.fp-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

.fp-home {
  font-size: 13px;
  color: var(--el-text-color-regular);
  word-break: break-all;
}

.fp-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.fp-path {
  margin-bottom: 8px;
}

.fp-alert {
  margin-bottom: 8px;
}

.fp-body {
  height: 320px;
  overflow-y: auto;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
}

.fp-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  cursor: pointer;
  font-size: 13px;
}

.fp-item:hover {
  background: var(--el-fill-color-light);
}

.fp-item--file.is-selected {
  background: var(--el-color-primary-light-9);
}

.fp-item__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fp-item__size {
  margin-left: auto;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
