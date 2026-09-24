<template>
  <el-dialog
    v-model="visible"
    :title="isEdit ? t('docker.compose.editTitle') : t('docker.compose.createTitle')"
    width="760px"
    top="6vh"
    :close-on-click-modal="false"
    destroy-on-close
    @opened="onOpened"
  >
    <el-form label-width="96px" label-position="top">
      <el-form-item :label="t('docker.compose.project')">
        <el-input
          v-model="name"
          :disabled="isEdit"
          :placeholder="t('docker.compose.namePlaceholder')"
        />
        <div v-if="!isEdit" class="field-hint">{{ t('docker.compose.nameHint') }}</div>
      </el-form-item>

      <!--
        存放位置：决定项目目录落在哪儿 —— 也就决定了 compose 里 `./src` 这类
        相对路径指向哪儿。编辑时位置跟着文件走，只显示当前路径。
      -->
      <el-form-item :label="t('docker.compose.locationLabel')">
        <el-radio-group v-if="!isEdit" v-model="location">
          <el-radio value="global">{{ t('docker.compose.locationGlobal') }}</el-radio>
          <el-radio value="user">{{ t('docker.compose.locationUser') }}</el-radio>
        </el-radio-group>
        <div class="loc-preview mono">{{ isEdit ? (path || '—') : locationPath }}</div>
        <div class="field-hint">
          {{ isEdit ? t('docker.compose.locationLocked') : t('docker.compose.locationHint') }}
        </div>
      </el-form-item>

      <el-form-item :label="t('docker.compose.content')">
        <div class="editor">
          <div class="editor__toolbar">
            <el-button size="small" :icon="FolderOpened" @click="pickerVisible = true">
              {{ t('docker.compose.pickServer') }}
            </el-button>
            <el-button size="small" :icon="Upload" @click="pickFile">
              {{ t('docker.compose.importFile') }}
            </el-button>
            <el-button size="small" :icon="DocumentAdd" @click="useTemplate">
              {{ t('docker.compose.template') }}
            </el-button>
            <span class="editor__hint">{{ t('docker.compose.editorHint') }}</span>
            <!-- 隐藏的原生文件选择器：从本机上传 .yml / .yaml -->
            <input
              ref="fileRef"
              type="file"
              accept=".yml,.yaml,text/yaml,text/plain"
              class="editor__file"
              @change="onFilePicked"
            />
          </div>
          <div v-if="sourcePath" class="editor__source">
            {{ t('docker.compose.source') }}：<code>{{ sourcePath }}</code>
          </div>
          <el-input
            v-model="body"
            type="textarea"
            :rows="16"
            spellcheck="false"
            class="editor__area"
            :placeholder="t('docker.compose.contentPlaceholder')"
          />
        </div>
      </el-form-item>

      <el-form-item v-if="!isEdit">
        <el-checkbox v-model="startAfterSave">{{ t('docker.compose.startAfterCreate') }}</el-checkbox>
      </el-form-item>
    </el-form>

    <!-- 从服务器挑文件：起点是当前用户的主目录 -->
    <ComposeFilePicker v-model="pickerVisible" @pick="onServerPicked" />

    <template #footer>
      <el-button @click="visible = false">{{ t('docker.common.cancel') }}</el-button>
      <el-button type="primary" :loading="saving" @click="submit">
        {{ t('docker.common.save') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { DocumentAdd, FolderOpened, Upload } from '@/icons'
import { composeSave, type ComposeLocation } from '@/api/docker'
import { readFile } from '@/api/file'
import { getUserInfo } from '@/api/user'
import ComposeFilePicker from './ComposeFilePicker.vue'

const props = defineProps<{
  modelValue: boolean
  /** 编辑模式：项目名不可改 */
  mode?: 'create' | 'edit'
  project?: string
  content?: string
  /** 编辑时项目当前所在的配置文件路径（只读展示） */
  path?: string
  /** 新建时预填的项目名（如从文件名推断） */
  presetName?: string
  /** 已存在的项目名：新建时重名要再确认一次，避免一键覆盖别人的配置 */
  existing?: string[]
}>()

const emit = defineEmits<{
  'update:modelValue': [boolean]
  /** 保存成功：父组件负责刷新列表、选中项目，必要时再 up */
  saved: [project: string, start: boolean]
}>()

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const isEdit = computed(() => props.mode === 'edit')

const name = ref('')
const body = ref('')
const startAfterSave = ref(false)
const saving = ref(false)
const fileRef = ref<HTMLInputElement | null>(null)
/** 服务器文件选择窗口 */
const pickerVisible = ref(false)
/** 内容是从哪儿来的（服务器路径 / 本机文件名），编辑时留空表示沿用项目配置 */
const sourcePath = ref('')

/** 新建时的存放位置：公共区域（`/opt/docker`）还是自己的家目录 */
const location = ref<ComposeLocation>('global')
/** 当前账号的家目录，用于预览落点（取不到就显示 `~`，不影响保存） */
const homeDir = ref('')

async function ensureHomeDir() {
  if (homeDir.value) return
  try {
    const resp = await getUserInfo()
    homeDir.value = (resp.data as any)?.home_dir || ''
  } catch {
    // 拿不到只影响预览，后端仍会用自己的 home_dir 落盘
  }
}

/** 落点预览：项目名还没填时给占位，让用户看懂两个选项的区别 */
const locationPath = computed(() => {
  const project = name.value.trim() || '<name>'
  if (location.value === 'user') {
    const home = homeDir.value.trim().replace(/\/+$/, '')
    return `${home || '~'}/${project}`
  }
  return `/opt/docker/${project}`
})

/** 打开时把编辑目标填进来：编辑用传入值，新建用预填名 + 模板留空 */
function onOpened() {
  name.value = isEdit.value ? (props.project ?? '') : (props.presetName ?? '')
  body.value = props.content ?? ''
  startAfterSave.value = !isEdit.value
  sourcePath.value = ''
  location.value = 'global'
  if (!isEdit.value) ensureHomeDir()
}

/** 与后端 `valid_project_name` 保持一致：字母数字开头，可含 `_ . -`，最长 63 */
const NAME_RE = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,62}$/

function pickFile() {
  fileRef.value?.click()
}

async function onFilePicked(ev: Event) {
  const input = ev.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = '' // 同一个文件连选两次也要触发 change
  if (!file) return

  body.value = await file.text()
  // 新建模式下顺手用文件名当项目名：`nginx-compose.yml` → `nginx-compose`
  if (!isEdit.value && !name.value.trim()) name.value = normalizeName(file.name)
  sourcePath.value = file.name
}

/**
 * 从服务器挑好文件后读盘回填。
 *
 * 服务器上的文件只是"拷贝进来的模板"：保存时会另写一份到面板的 stacks 目录，
 * 不会去改用户 home 里的原件（否则面板一改，用户自己的文件就跟着变了）。
 */
async function onServerPicked(serverPath: string) {
  try {
    const res = await readFile(serverPath)
    body.value = res.data.content ?? ''
    const basename = serverPath.split('/').pop() ?? ''
    if (!isEdit.value && !name.value.trim()) name.value = normalizeName(basename)
    sourcePath.value = serverPath
    pickerVisible.value = false
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.loadFailed'))
  }
}

/** 文件名 → 合法项目名（非法字符换成 `-`，并裁掉扩展名） */
function normalizeName(filename: string): string {
  const base = filename.replace(/\.(ya?ml)$/i, '')
  const cleaned = base.replace(/[^a-zA-Z0-9._-]/g, '-').replace(/^[^a-zA-Z0-9]+/, '')
  return cleaned.slice(0, 63)
}

/** 最小可用模板：不写模板用户容易连 YAML 缩进都没对齐，试跑一次就报错 */
function useTemplate() {
  body.value = [
    'services:',
    '  web:',
    '    image: nginx:latest',
    '    restart: unless-stopped',
    '    ports:',
    '      - "8080:80"',
    '',
  ].join('\n')
}

async function submit() {
  const project = name.value.trim()
  if (!isEdit.value && !NAME_RE.test(project)) {
    ElMessage.warning(t('docker.compose.nameInvalid'))
    return
  }
  if (!body.value.trim()) {
    ElMessage.warning(t('docker.compose.contentRequired'))
    return
  }
  // 新建时撞名：这里覆盖的是整个配置文件，确认一次再动手
  if (!isEdit.value && props.existing?.includes(project)) {
    try {
      await ElMessageBox.confirm(
        t('docker.compose.overwriteConfirm', { name: project }),
        t('docker.compose.createTitle'),
        { type: 'warning' },
      )
    } catch {
      return
    }
  }

  saving.value = true
  try {
    // 编辑模式项目名固定，用 props 里的值，避免用户改到一半的输入串了项目；
    // 位置只在新建时下发（编辑就地覆盖，不会被搬家）
    const target = isEdit.value ? (props.project ?? '') : project
    await composeSave(target, body.value, isEdit.value ? undefined : location.value)
    ElMessage.success(t('docker.compose.saved'))
    visible.value = false
    emit('saved', target, !isEdit.value && startAfterSave.value)
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.compose.saveFailed'))
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.field-hint {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.editor {
  width: 100%;
  border: 1px solid var(--el-border-color);
  border-radius: 6px;
  overflow: hidden;
}

.editor__toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  background: var(--el-fill-color-light);
  border-bottom: 1px solid var(--el-border-color);
  flex-wrap: wrap;
}

.editor__hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

/* 落点预览 / 编辑时当前位置：等宽字体交给全局 `.mono`，这里只管间距与换行 */
.loc-preview {
  margin-top: 6px;
  color: var(--el-text-color-regular);
  word-break: break-all;
}

/* 内容来源（服务器路径 / 本机文件名）：提醒用户保存后会另存到 stacks 目录 */
.editor__source {
  padding: 6px 10px;
  border-top: 1px dashed var(--el-border-color);
  font-size: 12px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}

.editor__source code {
  font-family: Menlo, Monaco, 'Courier New', monospace;
}

.editor__file {
  display: none;
}

/* 等宽正文：compose.yaml 的缩进对齐很重要 */
.editor__area :deep(textarea) {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 13px;
  line-height: 1.6;
  border: none;
  border-radius: 0;
  box-shadow: none;
}
</style>
