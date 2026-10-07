<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-dialog
    v-model="visible"
    :title="dialogTitle"
    width="600px"
    append-to-body
    :close-on-click-modal="false"
  >
    <div class="dp-head">
      <el-tag size="small" type="info" effect="plain">HOME</el-tag>
      <code class="dp-home">{{ home || '—' }}</code>
    </div>

    <div class="dp-toolbar">
      <el-button size="small" :disabled="!home || path === home" @click="fetch(home)">
        {{ t('dirPicker.home') }}
      </el-button>
      <el-button size="small" :disabled="!canGoUp" @click="fetch(parentPath)">
        {{ t('dirPicker.up') }}
      </el-button>
      <el-button size="small" :icon="Refresh" :disabled="!path" @click="fetch(path)">
        {{ t('dirPicker.refresh') }}
      </el-button>
    </div>

    <el-input
      v-model="pathInput"
      size="small"
      class="dp-path"
      :placeholder="t('dirPicker.pathPlaceholder')"
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
      class="dp-alert"
    />

    <div v-loading="loading" class="dp-body">
      <template v-if="dirs.length">
        <div v-for="d in dirs" :key="d.path" class="dp-item" @click="fetch(d.path)">
          <el-icon><FolderOpened /></el-icon>
          <span class="dp-item-name" :title="d.path">{{ d.name }}</span>
        </div>
      </template>
      <el-empty v-else-if="!loading" :description="t('dirPicker.empty')" :image-size="60" />
    </div>

    <!-- 调用方可以补一行额外输入（例如打包时的压缩包名称） -->
    <slot name="extra" :path="path" />

    <template #footer>
      <el-button @click="visible = false">{{ t('dirPicker.cancel') }}</el-button>
      <el-button type="primary" :disabled="!path" :loading="confirmLoading" @click="confirm">
        {{ resolvedConfirmText }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { FolderOpened, Refresh } from '@/icons'
import { listFiles, type FileEntry } from '@/api/file'
import { useUserStore } from '@/stores/user'

/**
 * 目录选择窗口（新建站点「选择已有目录」那套交互）：
 * 家目录速览 + 回到首页 / 返回上级 / 刷新 + 可编辑路径 + 子目录列表。
 *
 * 只用于选目录，不列文件；起始目录由 `startPath` 指定，留空即当前用户家目录。
 */
const props = withDefaults(
  defineProps<{
    modelValue: boolean
    title?: string
    /** 打开时的起始目录，留空则落到当前用户家目录 */
    startPath?: string
    confirmText?: string
    /** 确认按钮的 loading，由调用方的实际操作接管 */
    confirmLoading?: boolean
  }>(),
  {
    title: '',
    startPath: '',
    confirmText: '',
    confirmLoading: false,
  },
)

/** 标题与确认按钮文案：调用方未指定时回落到内置文案（跟随语言切换） */
const dialogTitle = computed(() => props.title || t('dirPicker.title'))
const resolvedConfirmText = computed(() => props.confirmText || t('dirPicker.confirm'))

const emit = defineEmits<{
  (e: 'update:modelValue', value: boolean): void
  (e: 'confirm', path: string): void
}>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v),
})

const userStore = useUserStore()
/** 普通用户出不了家目录（后端白名单兜底），管理员可以一路向上 */
const isAdmin = computed(() => userStore.roles.includes('admin'))

const home = ref('')
const path = ref('')
const parentPath = ref('')
const pathInput = ref('')
const dirs = ref<FileEntry[]>([])
const loading = ref(false)
const error = ref('')

const canGoUp = computed(() => {
  const up = parentPath.value
  if (!path.value || !up || up === path.value) return false
  if (isAdmin.value) return true
  return !!home.value && (up === home.value || up.startsWith(`${home.value}/`))
})

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
    dirs.value = (data?.entries || []).filter((e) => e.is_dir)
  } catch (e: any) {
    error.value = e?.message || t('dirPicker.loadFailed')
    dirs.value = []
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
  if (!path.value) return
  emit('confirm', path.value)
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) fetch(props.startPath.trim())
  },
)
</script>

<style scoped lang="scss">
.dp-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

.dp-home {
  font-size: 13px;
  color: var(--el-text-color-regular);
  word-break: break-all;
}

.dp-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.dp-path {
  margin-bottom: 8px;
}

.dp-alert {
  margin-bottom: 10px;
}

.dp-body {
  min-height: 120px;
  max-height: 46vh;
  overflow-y: auto;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 6px;
}

.dp-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 14px;
  color: var(--el-text-color-regular);

  &:hover {
    background: var(--el-fill-color-light);
    color: var(--el-color-primary);
  }

  .el-icon {
    color: var(--el-color-warning);
  }
}

.dp-item-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
