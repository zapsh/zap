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
      <div class="fp-toolbar__spacer" />
      <el-checkbox v-model="onlyYaml" size="small">
        {{ t('docker.compose.onlyYaml') }}
      </el-checkbox>
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

      <!-- 文件：单击选中，双击直接确认 -->
      <div
        v-for="f in files"
        :key="f.path"
        class="fp-item fp-item--file"
        :class="{ 'is-selected': selected === f.path }"
        @click="selected = f.path"
        @dblclick="confirm"
      >
        <el-icon><Document /></el-icon>
        <span class="fp-item__name">{{ f.name }}</span>
        <el-tag v-if="isYaml(f.name)" size="small" effect="plain">yaml</el-tag>
        <span class="fp-item__size">{{ sizeText(f.size) }}</span>
      </div>

      <el-empty
        v-if="!loading && !dirs.length && !files.length"
        :description="t('filePicker.empty')"
        :image-size="60"
      />
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ t('filePicker.cancel') }}</el-button>
      <el-button type="primary" :disabled="!selected" @click="confirm">
        {{ t('filePicker.confirm') }}
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
 * 服务器文件选择窗口（Compose「从服务器选择」用）。
 *
 * 走的是文件管理那套 `/system/files/list`：起点是**当前用户的家目录**，
 * 管理员可以一路向上（后端白名单对 admin 放行），普通用户出不了自己的 home ——
 * 与文件管理、云存储「从服务器选择」是同一套隔离规则，不另开口子。
 */
const props = withDefaults(
  defineProps<{
    modelValue: boolean
    title?: string
    /** 打开时的起始目录，留空即落到当前用户家目录 */
    startPath?: string
  }>(),
  { title: '', startPath: '' },
)

const emit = defineEmits<{
  'update:modelValue': [boolean]
  pick: [path: string]
}>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const dialogTitle = computed(() => props.title || t('filePicker.title'))

const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

const home = ref('')
const path = ref('')
const parentPath = ref('')
const pathInput = ref('')
const entries = ref<FileEntry[]>([])
const selected = ref('')
const onlyYaml = ref(true)
const loading = ref(false)
const error = ref('')

const dirs = computed(() => entries.value.filter((e) => e.is_dir))
/** 只看 yaml 时隐藏其余文件：compose 导入的场景里目录通常堆着一堆别的东西 */
const files = computed(() => {
  const list = entries.value.filter((e) => !e.is_dir)
  return onlyYaml.value ? list.filter((e) => isYaml(e.name)) : list
})

const canGoUp = computed(() => {
  const up = parentPath.value
  if (!path.value || !up || up === path.value) return false
  if (isAdmin.value) return true
  return !!home.value && (up === home.value || up.startsWith(`${home.value}/`))
})

function isYaml(name: string): boolean {
  return /\.(yml|yaml)$/i.test(name)
}

function sizeText(size: number): string {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / 1024 / 1024).toFixed(1)} MB`
}

async function fetch(target: string) {
  loading.value = true
  error.value = ''
  selected.value = ''
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

function confirm() {
  if (!selected.value) return
  emit('pick', selected.value)
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) fetch(props.startPath.trim())
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

.fp-toolbar__spacer {
  flex: 1;
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
