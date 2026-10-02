<template>
  <el-dialog
    v-model="visible"
    :title="t('filesLocal.extractDialog.title')"
    width="560px"
    append-to-body
    :close-on-click-modal="false"
  >
    <el-alert
      v-if="!supported"
      :title="t('filesLocal.extractDialog.unsupported')"
      type="warning"
      :closable="false"
      show-icon
      class="ex-alert"
    />

    <div class="ex-row">
      <span class="ex-label">{{ t('filesLocal.extractDialog.source') }}</span>
      <code class="ex-path">{{ basename }}</code>
    </div>
    <p class="ex-hint">{{ t('filesLocal.extractDialog.sourceHint') }}</p>

    <div class="ex-field">
      <label>{{ t('filesLocal.extractDialog.destMode') }}</label>
      <el-radio-group v-model="mode">
        <el-radio value="here">{{ t('filesLocal.extractDialog.here') }}</el-radio>
        <el-radio value="newFolder">{{ t('filesLocal.extractDialog.newFolder') }}</el-radio>
        <el-radio value="custom">{{ t('filesLocal.extractDialog.custom') }}</el-radio>
      </el-radio-group>
    </div>

    <p v-if="mode === 'newFolder'" class="ex-hint ex-indent">
      {{ t('filesLocal.extractDialog.newFolderHint') }}
    </p>

    <div v-if="mode === 'custom'" class="ex-field">
      <label>&nbsp;</label>
      <el-input v-model="customDir" size="small" :placeholder="parentDir">
        <template #append>
          <el-button @click="pickerVisible = true">
            {{ t('filesLocal.extractDialog.pickDir') }}
          </el-button>
        </template>
      </el-input>
    </div>

    <div class="ex-field">
      <label>{{ t('filesLocal.extractDialog.target') }}</label>
      <code class="ex-path ex-target">{{ targetDir || '—' }}</code>
    </div>

    <div class="ex-field">
      <label>&nbsp;</label>
      <el-checkbox v-model="overwrite">{{ t('filesLocal.extractDialog.overwrite') }}</el-checkbox>
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ t('common.cancel') }}</el-button>
      <el-button
        type="primary"
        :disabled="!supported || !targetDir"
        :loading="loading"
        @click="confirm"
      >
        {{ t('filesLocal.extractDialog.start') }}
      </el-button>
    </template>

    <DirPicker
      v-model="pickerVisible"
      :start-path="startPath"
      :confirm-text="t('filesLocal.extractDialog.pickDir')"
      @confirm="onPick"
    />
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import DirPicker from '@/components/DirPicker.vue'
import { extractArchive } from '@/api/file'

const props = withDefaults(
  defineProps<{
    modelValue: boolean
    /** 压缩包完整路径 */
    archivePath: string
    /** 打开目录选择器的起始目录 */
    startPath?: string
  }>(),
  { startPath: '' },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: boolean): void
  (e: 'success'): void
}>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v),
})

const mode = ref<'here' | 'newFolder' | 'custom'>('newFolder')
const customDir = ref('')
const overwrite = ref(false)
const loading = ref(false)
const pickerVisible = ref(false)

/** 仅前端用于决定菜单/确认是否可用；后端会再做一次格式校验 */
const SUPPORTED = ['zip', 'tar', 'tar.gz', 'tgz', 'tar.bz2', 'tar.xz', '7z', 'gz']

function basenameOf(p: string): string {
  const i = p.lastIndexOf('/')
  return i >= 0 ? p.slice(i + 1) : p
}
function parentOf(p: string): string {
  const i = p.lastIndexOf('/')
  return i > 0 ? p.slice(0, i) : '/'
}
function stripArchiveExt(name: string): string {
  const lower = name.toLowerCase()
  const suffixes = ['.tar.gz', '.tar.bz2', '.tar.xz', '.tgz', '.zip', '.tar']
  for (const s of suffixes) {
    if (lower.endsWith(s)) return name.slice(0, name.length - s.length)
  }
  return name
}

const basename = computed(() => basenameOf(props.archivePath))
const parentDir = computed(() => parentOf(props.archivePath))

const supported = computed(() => {
  const n = basename.value.toLowerCase()
  return SUPPORTED.some((s) => n.endsWith(`.${s}`))
})

const targetDir = computed(() => {
  if (mode.value === 'here') return parentDir.value
  if (mode.value === 'newFolder') {
    return `${parentDir.value}/${stripArchiveExt(basename.value)}`
  }
  return customDir.value.trim()
})

function onPick(path: string) {
  customDir.value = path
}

async function confirm() {
  if (!targetDir.value) return
  if (!supported.value) {
    ElMessage.warning(t('filesLocal.extractDialog.unsupported'))
    return
  }
  loading.value = true
  try {
    await extractArchive(props.archivePath, targetDir.value, overwrite.value)
    ElMessage.success(t('common.success'))
    emit('success')
    visible.value = false
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.extractDialog.unsupported'))
  } finally {
    loading.value = false
  }
}

watch(visible, (open) => {
  if (open) {
    mode.value = 'newFolder'
    customDir.value = ''
    overwrite.value = false
    loading.value = false
  }
})
</script>

<style scoped lang="scss">
.ex-alert {
  margin-bottom: 14px;
}

.ex-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 2px;
}

.ex-label {
  flex: none;
  width: 72px;
  color: var(--el-text-color-secondary);
  font-size: 13px;
}

.ex-path {
  font-size: 13px;
  color: var(--el-text-color-regular);
  word-break: break-all;
}

.ex-hint {
  margin: 0 0 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.ex-indent {
  padding-left: 72px;
}

.ex-field {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin-bottom: 12px;

  > label {
    flex: none;
    width: 72px;
    line-height: 32px;
    color: var(--el-text-color-secondary);
    font-size: 13px;
  }

  > .el-radio-group,
  > .el-input,
  > .ex-target {
    flex: 1;
    min-width: 0;
  }
}

.ex-target {
  line-height: 32px;
}
</style>
