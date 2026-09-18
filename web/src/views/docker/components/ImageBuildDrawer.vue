<template>
  <el-drawer
    v-model="visible"
    :title="t('docker.build.title')"
    size="52%"
    :close-on-click-modal="false"
    @closed="onClosed"
  >
    <el-alert type="info" :closable="false" show-icon class="build-tip">
      <template #title>
        <span>{{ t('docker.build.tip') }}</span>
        <div v-if="!isAdmin" class="ns-line mono">
          {{ t('docker.build.nsHint', { prefix: nsPrefix }) }}
        </div>
      </template>
    </el-alert>

    <el-form label-position="top" size="default">
      <el-form-item :label="t('docker.build.name')" required>
        <el-input v-model="form.name" :placeholder="t('docker.build.namePlaceholder')" @blur="normalizeName" />
        <div v-if="previewName" class="form-hint mono">{{ t('docker.build.preview') }} {{ previewName }}</div>
      </el-form-item>

      <el-form-item :label="t('docker.build.context')" required>
        <el-input v-model="form.context_dir" :placeholder="t('docker.build.contextPlaceholder')">
          <template #append>
            <el-button :icon="FolderOpened" @click="pickerVisible = true">
              {{ t('docker.build.pickPath') }}
            </el-button>
          </template>
        </el-input>
        <div class="form-hint">{{ t('docker.build.contextHint') }}</div>
      </el-form-item>

      <el-form-item :label="t('docker.build.containerfile')">
        <el-input v-model="form.containerfile" :placeholder="t('docker.build.containerfilePlaceholder')" />
        <div class="form-hint">{{ t('docker.build.containerfileHint') }}</div>
      </el-form-item>

      <el-form-item :label="t('docker.build.tags')">
        <div class="tag-editor">
          <el-tag
            v-for="(tg, i) in form.tags"
            :key="`${tg}-${i}`"
            closable
            size="small"
            type="success"
            effect="plain"
            @close="form.tags.splice(i, 1)"
          >
            {{ tg }}
          </el-tag>
          <el-input
            v-if="tagInputVisible"
            ref="tagInputRef"
            v-model="tagInput"
            size="small"
            class="tag-input"
            @keyup.enter="commitTag"
            @blur="commitTag"
          />
          <el-button v-else size="small" :icon="Plus" @click="showTagInput">
            {{ t('docker.build.addTag') }}
          </el-button>
        </div>
      </el-form-item>

      <el-form-item :label="t('docker.build.args')">
        <div class="arg-editor">
          <div v-for="(a, i) in form.buildArgs" :key="i" class="arg-row">
            <el-input v-model="a.key" size="small" class="arg-key" :placeholder="t('docker.build.argKey')" />
            <span class="arg-eq">=</span>
            <el-input v-model="a.value" size="small" class="arg-value" :placeholder="t('docker.build.argValue')" />
            <el-button link type="danger" :icon="Delete" @click="form.buildArgs.splice(i, 1)" />
          </div>
          <el-button size="small" :icon="Plus" @click="form.buildArgs.push({ key: '', value: '' })">
            {{ t('docker.build.addArg') }}
          </el-button>
        </div>
      </el-form-item>

      <el-form-item :label="t('docker.build.platform')">
        <el-select v-model="form.platform" :style="{ width: '220px' }">
          <el-option :label="t('docker.build.platformHost')" value="" />
          <el-option label="linux/amd64" value="linux/amd64" />
          <el-option label="linux/arm64" value="linux/arm64" />
          <el-option label="linux/arm64/v8" value="linux/arm64/v8" />
          <el-option label="linux/arm/v7" value="linux/arm/v7" />
          <el-option label="linux/386" value="linux/386" />
        </el-select>
      </el-form-item>

      <el-form-item>
        <el-checkbox v-model="form.no_cache">{{ t('docker.build.noCache') }}</el-checkbox>
        <el-checkbox v-model="form.pull">{{ t('docker.build.pullBase') }}</el-checkbox>
      </el-form-item>
    </el-form>

    <template #footer>
      <div class="build-footer">
        <el-button @click="visible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :icon="Build" :loading="starting" @click="start">
          {{ t('docker.build.start') }}
        </el-button>
      </div>
    </template>

    <!-- 构建上下文目录选择：普通用户出不了家目录（后端另有包含校验兜底） -->
    <DirPicker
      v-model="pickerVisible"
      :title="t('docker.build.pickTitle')"
      :confirm-text="t('common.confirm')"
      @confirm="onPickDir"
    />

    <!-- 实时日志：复用应用商店的运行日志抽屉（同一套 WS + xterm） -->
    <AppStoreLogDrawer ref="logDrawerRef" simple />
  </el-drawer>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElInput } from 'element-plus'
import { Build, Delete, FolderOpened, Plus } from '@/icons'
import { imageBuild } from '@/api/docker'
import { getRuns, type RunItem } from '@/api/appstore'
import DirPicker from '@/components/DirPicker.vue'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import { useUserStore } from '@/stores/user'

/**
 * 镜像构建抽屉：填参数 → 交给后端起后台 `docker build` → 用 run_id 看实时日志。
 *
 * 多用户约定（后端才是最终防线，这里只做提示 / 预校验）：
 * - 非管理员的目标镜像会被加上 `<命名空间>/` 前缀，避免多个人构建同名镜像互相覆盖；
 * - 构建上下文必须是绝对路径，普通用户只能选自家目录内的路径。
 */
const props = defineProps<{ modelValue: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [boolean]; built: [] }>()

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v),
})

const form = reactive({
  name: '',
  tags: [] as string[],
  context_dir: '',
  containerfile: '',
  platform: '' as string,
  no_cache: false,
  pull: false,
  buildArgs: [] as { key: string; value: string }[],
})

const starting = ref(false)
const pickerVisible = ref(false)

// ── 额外 tag 的输入态 ───────────────────────────────────────
const tagInputVisible = ref(false)
const tagInput = ref('')
const tagInputRef = ref<InstanceType<typeof ElInput> | null>(null)

const logDrawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)

/** 命名空间前缀（仅用于提示；真实前缀由后端派生并在返回里给出） */
const nsPrefix = computed(() => `${(userStore.name || '').toLowerCase().replace(/[^a-z0-9_-]/g, '-')}/`)

/** 输入失焦时顺手小写化（docker 仓库名不接受大写） */
function normalizeName() {
  form.name = form.name.trim().toLowerCase()
}

/** 预览最终镜像名：非管理员会被加上命名空间前缀 */
const previewName = computed(() => {
  const name = form.name.trim()
  if (!name) return ''
  const tagged = name.split('/').pop()?.includes(':') ? name : `${name}:latest`
  return isAdmin.value ? tagged : `${nsPrefix.value}${tagged}`
})

function showTagInput() {
  tagInputVisible.value = true
  nextTick(() => tagInputRef.value?.focus())
}

function commitTag() {
  const val = tagInput.value.trim().toLowerCase()
  if (val && !form.tags.includes(val)) form.tags.push(val)
  tagInput.value = ''
  tagInputVisible.value = false
}

function onPickDir(path: string) {
  form.context_dir = path
  pickerVisible.value = false
}

// ── 提交 ────────────────────────────────────────────────────
function validate(): string {
  const name = form.name.trim()
  if (!name) return t('docker.build.needName')
  // 与后端 `valid_image_ref` 同一套白名单：小写字母数字与 . _ - /，可选 :tag
  if (!/^[a-z0-9._/-]+(:[a-z0-9._-]+)?$/i.test(name)) {
    return t('docker.build.badName')
  }
  if (!isAdmin.value && name !== name.toLowerCase()) return t('docker.build.badCase')
  for (const tg of form.tags) {
    if (!/^[a-z0-9._/-]+(:[a-z0-9._-]+)?$/i.test(tg)) return t('docker.build.badTag', { tag: tg })
  }
  if (!form.context_dir.trim()) return t('docker.build.needContext')
  if (!form.context_dir.startsWith('/')) return t('docker.build.needAbsolute')
  for (const a of form.buildArgs) {
    const key = a.key.trim()
    if (key && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) return t('docker.build.badArgKey', { key })
  }
  return ''
}

async function start() {
  const err = validate()
  if (err) {
    ElMessage.warning(err)
    return
  }
  starting.value = true
  try {
    const resp = await imageBuild({
      name: form.name.trim(),
      tags: [...form.tags],
      context_dir: form.context_dir.trim(),
      containerfile: form.containerfile.trim() || undefined,
      build_args: form.buildArgs
        .filter((a) => a.key.trim())
        .map((a) => ({ key: a.key.trim(), value: a.value })),
      platform: form.platform,
      no_cache: form.no_cache,
      pull: form.pull,
    })
    ElMessage.success(t('docker.build.started'))
    // 真正的镜像名以后端为准（非管理员会被加上命名空间前缀）
    const finalTags: string[] = resp.data?.tags || []
    visible.value = false
    logDrawerRef.value?.openDrawer(
      resp.data?.run_id || '',
      `${t('docker.build.logTitle')}${finalTags.length ? ' · ' + finalTags[0] : ''}`,
    )
    trackRun(resp.data?.run_id || '')
  } catch (e: any) {
    ElMessage.error(e?.message || t('docker.build.failed'))
  } finally {
    starting.value = false
  }
}

// ── 构建结束后刷新镜像列表 ──────────────────────────────────
const pollTimer = ref<number | null>(null)

/** 轮询运行记录：构建结束（成功或失败）就提示并让列表刷新 */
function trackRun(runId: string) {
  if (!runId || pollTimer.value !== null) return
  pollTimer.value = window.setInterval(async () => {
    try {
      const resp = await getRuns({ page: 1, page_size: 50 })
      const item: RunItem | undefined = (resp.data?.items || []).find((r: RunItem) => r.run_id === runId)
      if (!item || item.status === 'running') return
      if (pollTimer.value !== null) {
        window.clearInterval(pollTimer.value)
        pollTimer.value = null
      }
      if (item.status === 'success') {
        ElMessage.success(t('docker.build.success'))
        emit('built')
      } else {
        ElMessage.error(t('docker.build.failCode', { code: item.exit_code }))
      }
    } catch {
      // 后端抖动，下一轮再试
    }
  }, 3000)
}

function onClosed() {
  if (pollTimer.value !== null) {
    window.clearInterval(pollTimer.value)
    pollTimer.value = null
  }
}

onBeforeUnmount(onClosed)

watch(visible, (v) => {
  if (v) {
    form.name = ''
    form.tags = []
    form.context_dir = ''
    form.containerfile = ''
    form.platform = ''
    form.no_cache = false
    form.pull = false
    form.buildArgs = []
  }
})
</script>

<style scoped>
.build-tip {
  margin-bottom: 14px;
}

.ns-line {
  margin-top: 4px;
  font-size: 12px;
}

.form-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.6;
}

.tag-editor,
.arg-editor {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  width: 100%;
}

.tag-input {
  width: 180px;
}

.arg-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
}

.arg-key {
  width: 200px;
}

.arg-value {
  flex: 1;
  min-width: 160px;
}

.arg-eq {
  color: var(--el-text-color-secondary);
}

.build-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}
</style>
