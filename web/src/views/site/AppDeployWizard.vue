<template>
  <el-dialog
    v-model="visible"
    :title="editing ? t('site.appWizardTitleEdit') : t('site.appWizardTitle')"
    width="780px"
    :close-on-click-modal="false"
    @closed="onClosed"
  >
    <el-steps :active="step" finish-status="success" simple class="wz-steps">
      <el-step :title="t('site.stepRuntime')" />
      <el-step :title="t('site.stepCode')" />
      <el-step :title="t('site.stepRun')" />
      <el-step :title="t('site.stepAccess')" />
    </el-steps>

    <!-- ── 1. 运行时 ───────────────────────────────── -->
    <el-form v-show="step === 0" label-width="120px">
      <el-form-item :label="t('site.appTarget')">
        <el-radio-group v-model="form.target" :disabled="editing">
          <el-radio value="site">{{ t('site.appTargetSite') }}</el-radio>
          <el-radio value="domain">{{ t('site.appTargetDomain') }}</el-radio>
        </el-radio-group>
      </el-form-item>
      <el-form-item v-if="form.target === 'site'" :label="t('site.appPickSite')" required>
        <el-select
          v-model="siteIdModel"
          filterable
          style="width: 100%"
          :placeholder="sites.length ? t('site.appPickSitePh') : t('site.appNoSite')"
        >
          <el-option v-for="s in sites" :key="s.id" :value="s.id" :label="s.name" />
        </el-select>
        <div v-if="!sites.length" class="wz-tip">{{ t('site.appNoSiteTip') }}</div>
      </el-form-item>
      <template v-else>
        <el-form-item :label="t('site.appDomain')" required>
          <el-input v-model="form.domain" placeholder="app.example.com" style="width: 100%" />
          <div class="wz-tip">{{ t('site.appDomainHint') }}</div>
        </el-form-item>
      </template>
      <el-form-item v-if="!isStatic" :label="t('site.appMount')">
        <div class="mount-row">
          <el-select v-model="form.match_mode" class="mount-mode">
            <el-option value="" label="/path" />
            <el-option value="exact" label="= /path" />
            <el-option value="prefer" label="^~ /path" />
          </el-select>
          <el-input v-model="form.mount_path" placeholder="/" class="mount-path" />
        </div>
        <div v-if="mountHasPrefix" class="mount-switch">
          <el-switch v-model="form.strip_prefix" size="small" />
          <span>{{ t('site.appStripPrefix') }}</span>
          <el-tooltip :content="t('site.locStripPrefixTip')" placement="top">
            <span class="mount-switch-hint">?</span>
          </el-tooltip>
        </div>
        <div class="wz-tip">{{ t('site.appMountTip') }}</div>
      </el-form-item>
      <el-form-item :label="t('site.appName')" required>
        <el-input v-model="form.name" :placeholder="t('site.appNamePh')" style="width: 100%" />
        <div class="wz-tip">{{ t('site.appNameTip') }}</div>
        <div v-if="!isAdmin" class="wz-tip">
          {{ t('site.appNamePrefixHint', { prefix: namePreviewPrefix }) }}
        </div>
      </el-form-item>
      <el-form-item :label="t('site.appType')" required>
        <el-radio-group v-model="form.app_type">
          <el-radio v-for="it in typeOptions" :key="it" :value="it">
            {{ typeLabel(it) }}
          </el-radio>
        </el-radio-group>
      </el-form-item>
      <el-form-item v-if="!isStatic" :label="t('site.appVersion')">
        <el-select v-model="form.runtime_version" style="width: 260px">
          <el-option value="" :label="t('site.appVersionDefault')" />
          <el-option v-for="v in versions" :key="v" :value="v" :label="versionLabel(v)" />
        </el-select>
        <span v-if="!versions.length" class="wz-tip-inline">{{ t('site.appVersionNone') }}</span>
      </el-form-item>
    </el-form>

    <!-- ── 2. 代码 ─────────────────────────────────── -->
    <el-form v-show="step === 1" label-width="120px">
      <el-form-item :label="t('site.appWorkdir')" required>
        <div class="wz-row">
          <el-input v-model="form.workdir" :placeholder="t('site.appWorkdirPh')" />
          <el-button :icon="Folder" @click="dirVisible = true">{{ t('site.appPickDir') }}</el-button>
        </div>
        <div class="wz-tip">{{ t('site.appWorkdirTip') }}</div>
      </el-form-item>
      <el-form-item :label="t('site.appUseGit')">
        <el-switch v-model="form.use_git" />
        <span class="wz-tip-inline">{{ t('site.appUseGitHint') }}</span>
      </el-form-item>
      <template v-if="form.use_git">
        <el-form-item :label="t('site.appRepoUrl')" required>
          <el-input v-model="form.repo_url" placeholder="https://github.com/user/repo.git" style="width: 100%" />
          <div class="wz-tip">{{ t('site.appRepoUrlHint') }}</div>
        </el-form-item>
        <el-form-item :label="t('site.appBranch')">
          <el-input v-model="form.branch" :placeholder="t('site.appBranchPh')" style="width: 100%" />
        </el-form-item>
        <el-form-item :label="t('site.appGitSubdir')">
          <el-input v-model="form.git_subdir" :placeholder="t('site.appGitSubdirPh')" style="width: 100%" />
        </el-form-item>
        <el-form-item v-if="isStatic" :label="t('site.appBuildOutput')">
          <el-input v-model="form.build_output" :placeholder="t('site.appBuildOutputPh')" style="width: 100%" />
          <div class="wz-tip">{{ t('site.appBuildOutputHint') }}</div>
        </el-form-item>
        <el-form-item :label="t('site.appGitDepth')">
          <el-input-number v-model="form.git_depth" :min="0" :max="50" style="width: 130px" />
          <span class="wz-tip-inline">{{ t('site.appGitDepthHint') }}</span>
        </el-form-item>
      </template>
      <el-form-item :label="t('site.appEntry')">
        <el-input
          v-model="form.entry"
          :placeholder="isPython ? 'main.py / wsgi:app' : 'server.js'"
          style="width: 100%"
        />
        <div class="wz-tip">{{ t('site.appEntryHint') }}</div>
      </el-form-item>
      <el-form-item v-if="isPython" :label="t('site.appCreateVenv')">
        <el-switch v-model="form.create_venv" />
        <span class="wz-tip-inline">{{ t('site.appCreateVenvHint') }}</span>
      </el-form-item>
      <el-form-item :label="t('site.appInstallDeps')">
        <el-switch v-model="form.install_deps" />
        <span class="wz-tip-inline">
          {{ isPython ? 'pip install -r requirements.txt' : 'npm install / npm ci' }}
        </span>
      </el-form-item>
    </el-form>

    <!-- ── 3. 运行 ─────────────────────────────────── -->
    <el-form v-show="step === 2" label-width="120px">
      <el-form-item :label="t('site.appBuildCmd')">
        <el-input v-model="form.build_cmd" :placeholder="isPython ? 'alembic upgrade head' : 'npm run build'" style="width: 100%" />
        <div class="wz-tip">{{ t('site.appBuildCmdHint') }}</div>
      </el-form-item>
      <el-form-item v-if="!isStatic" :label="t('site.appStartCmd')">
        <el-input v-model="form.command" :placeholder="t('site.appStartCmdPh')" style="width: 100%" />
        <div class="wz-tip">{{ t('site.appStartCmdHint') }}</div>
      </el-form-item>
      <el-form-item v-if="!isStatic" :label="t('site.appPort')">
        <div class="wz-row">
          <el-switch v-model="form.auto_port" :active-text="t('site.appAutoPort')" />
          <el-input-number
            v-if="!form.auto_port"
            v-model="form.port"
            :min="0"
            :max="65535"
            :disabled="form.auto_port"
          />
        </div>
        <div class="wz-tip">
          {{ portRangeText || t('site.appPortEnv') }}
        </div>
      </el-form-item>
      <el-form-item :label="t('site.appEnvVars')">
        <el-input
          v-model="form.env"
          type="textarea"
          :rows="5"
          placeholder="DEBUG=false&#10;DATABASE_URL=sqlite:///app.db"
        />
        <div class="wz-tip">{{ t('site.appEnvVarsHint') }}</div>
      </el-form-item>
      <el-form-item :label="t('site.appAutostart')">
        <el-switch v-model="form.autostart" />
      </el-form-item>
    </el-form>

    <!-- ── 4. 访问与确认 ───────────────────────────── -->
    <div v-show="step === 3" class="wz-summary">
      <el-descriptions :column="1" border size="small">
        <el-descriptions-item :label="t('site.appName')">{{ form.name }}</el-descriptions-item>
        <el-descriptions-item :label="t('site.appType')">
          {{ typeLabel(form.app_type) }}
          <span v-if="form.runtime_version"> · {{ versionLabel(form.runtime_version) }}</span>
        </el-descriptions-item>
        <el-descriptions-item :label="t('site.appWorkdir')">{{ form.workdir }}</el-descriptions-item>
        <el-descriptions-item v-if="form.use_git && form.repo_url" :label="t('site.appRepoUrl')">
          {{ form.repo_url }}
          <span v-if="form.branch" class="wz-dim">@{{ form.branch }}</span>
        </el-descriptions-item>
        <el-descriptions-item :label="t('site.appTarget')">
          <span v-if="form.target === 'site'">
            {{ siteNameOf(form.site_id) }}
          </span>
          <span v-else>
            {{ form.domain }} <span class="wz-dim">（{{ t('site.appDomainHintShort') }}）</span>
          </span>
        </el-descriptions-item>
        <el-descriptions-item :label="t('site.appPort')">
          {{ form.auto_port ? t('site.appAutoPort') : form.port || '-' }}
        </el-descriptions-item>
        <el-descriptions-item v-if="form.build_cmd" :label="t('site.appBuildCmd')">
          {{ form.build_cmd }}
        </el-descriptions-item>
        <el-descriptions-item v-if="form.command" :label="t('site.appStartCmd')">
          {{ form.command }}
        </el-descriptions-item>
      </el-descriptions>
      <div class="wz-note">{{ t('site.appRunAsHint') }}</div>
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ t('common.cancel') }}</el-button>
      <el-button v-if="step > 0" @click="step -= 1">{{ t('site.wizardPrev') }}</el-button>
      <el-button v-if="step < 3" type="primary" @click="next">{{ t('site.wizardNext') }}</el-button>
      <el-button v-else type="primary" :loading="saving" @click="submit">
        {{ t('site.appDeploy') }}
      </el-button>
    </template>

    <!-- 部署是后台长任务：提交后在此实时查看 git 拉取 / 安装 / 构建进度 -->
    <AppStoreLogDrawer ref="logDrawer" :simple="true" :stoppable="true" />
  </el-dialog>

  <!-- 项目目录：复用全局目录选择组件 -->
  <DirPicker
    v-model="dirVisible"
    :title="t('site.appPickDir')"
    :start-path="form.workdir"
    @confirm="onDirPick"
  />
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Folder } from '@/icons'
import { http } from '@/utils/request'
import DirPicker from '@/components/DirPicker.vue'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import { deploySiteApp, getAppRuntimes } from '@/api/site'

const props = defineProps<{
  modelValue: boolean
  /** 从站点进入时预填；0 = 由向导自己决定（选站点或填域名） */
  siteId?: number
  /** 编辑（重新部署）已有应用时的预填数据 */
  initial?: Record<string, unknown> | null
  /** 当前操作者是否 admin（admin 部署的应用名不带用户名前缀） */
  isAdmin?: boolean
}>()
const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  (e: 'done'): void
}>()

const { t } = useI18n()

interface SiteOption {
  id: number
  name: string
  /** 站点归属的 unix 用户名：非 admin 部署时用它做应用名前缀 */
  owner?: string
}

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v),
})

const step = ref(0)
const saving = ref(false)
const logDrawer = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)
const dirVisible = ref(false)
const sites = ref<SiteOption[]>([])

/** 没有可选站点时不回显 0（el-select 会把无匹配的 modelValue 原样显示出来） */
const siteIdModel = computed({
  get: () => (sites.value.length ? form.value.site_id || null : null),
  set: (v: number | null) => {
    form.value.site_id = v ?? 0
  },
})
const runtimes = ref<{ python: string[]; nodejs: string[]; types: string[]; port_min: number; port_max: number }>({
  python: [],
  nodejs: [],
  types: [],
  port_min: 0,
  port_max: 0,
})

const form = ref({
  target: 'site' as 'site' | 'domain',
  site_id: 0,
  domain: '',
  /** 站点上用哪个前缀反代这个应用，默认 / */
  mount_path: '/',
  match_mode: '',
  /** 剥掉挂载前缀再转发：挂 /njs 时后端只监听 / 也能正常访问 */
  strip_prefix: false,
  name: '',
  app_type: 'python',
  runtime_version: '',
  workdir: '',
  entry: '',
  create_venv: true,
  install_deps: true,
  build_cmd: '',
  command: '',
  auto_port: true,
  port: 0,
  env: '',
  autostart: true,
  // git 部署（公开仓库）
  use_git: false,
  repo_url: '',
  branch: '',
  git_subdir: '',
  build_output: '',
  git_depth: 0,
})

const editing = computed(() => !!props.initial)
// admin 部署不加前缀；其余角色后端会自动补「站点归属用户-」前缀（预览用）
const isAdmin = computed(() => !!(props as any)?.isAdmin)
const namePreviewPrefix = computed(() => {
  const s = sites.value.find((x) => x.id === form.value.site_id)
  return s?.owner ? `${s.owner}-` : 'user-'
})
const isPython = computed(() => form.value.app_type === 'python')
const isStatic = computed(() => form.value.app_type === 'static')

const typeOptions = computed(() => {
  const allowed = runtimes.value.types
  const all = ['python', 'nodejs', 'static']
  if (!allowed.length) return all
  return all.filter((x) => allowed.includes(x))
})

const versions = computed(() =>
  isPython.value ? runtimes.value.python : runtimes.value.nodejs,
)

const portRangeText = computed(() => {
  const { port_min: lo, port_max: hi } = runtimes.value
  if (lo > 0 && hi > 0) {
    return `${t('site.appPortRangeHint', { min: lo, max: hi })} · ${t('site.appPortEnv')}`
  }
  return ''
})

function typeLabel(v: string) {
  if (v === 'python') return 'Python'
  if (v === 'nodejs') return 'Node.js'
  if (v === 'static') return t('site.appTypeStatic')
  return v
}
function versionLabel(v: string) {
  return isPython.value ? `Python ${v}` : `Node.js ${v}`
}
function siteNameOf(id: number) {
  return sites.value.find((s) => s.id === id)?.name || `#${id}`
}
function onDirPick(p: string) {
  form.value.workdir = p
  dirVisible.value = false
}

async function loadMeta() {
  try {
    const res = await http.get<{ code: number; data: { rows: any[] } }>('/site/list')
    sites.value = (res.data?.rows || []).map((r) => ({
      id: r.id,
      name: r.name,
      owner: r.owner_username || '',
    }))
  } catch {
    sites.value = []
  }
  try {
    const res = await getAppRuntimes()
    const d = (res as any)?.data || {}
    runtimes.value = {
      python: d.python || [],
      nodejs: d.nodejs || [],
      types: d.types || [],
      port_min: d.port_min || 0,
      port_max: d.port_max || 0,
    }
    // 套餐限制了类型时，落到第一个允许的类型
    if (runtimes.value.types.length && !runtimes.value.types.includes(form.value.app_type)) {
      form.value.app_type = runtimes.value.types[0]
    }
  } catch {
    /* 探测失败就用内置类型列表 */
  }
}

function next() {
  if (step.value === 0) {
    if (!form.value.name.trim()) {
      ElMessage.warning(t('site.appNameRequired'))
      return
    }
    if (form.value.target === 'site' && !form.value.site_id) {
      ElMessage.warning(t('site.appPickSite'))
      return
    }
    if (form.value.target === 'domain' && !form.value.domain.trim()) {
      ElMessage.warning(t('site.appDomainRequired'))
      return
    }
  }
  if (step.value === 1 && !form.value.workdir.trim() && !form.value.use_git) {
    ElMessage.warning(t('site.appWorkdirRequired'))
    return
  }
  step.value += 1
}

async function submit() {
  saving.value = true
  try {
    const res = await deploySiteApp({
      site_id: form.value.target === 'site' ? form.value.site_id : 0,
      name: form.value.name.trim(),
      app_type: form.value.app_type,
      runtime_version: form.value.runtime_version,
      build_cmd: form.value.build_cmd.trim(),
      create_venv: form.value.create_venv,
      workdir: form.value.workdir.trim(),
      entry: form.value.entry.trim(),
      command: form.value.command.trim(),
      port: form.value.auto_port ? 0 : form.value.port,
      auto_port: form.value.auto_port,
      env: form.value.env,
      autostart: form.value.autostart,
      install_deps: form.value.install_deps,
      domain: form.value.target === 'domain' ? form.value.domain.trim() : '',
      mount_path: form.value.mount_path.trim() || '/',
      match_mode: form.value.match_mode,
      strip_prefix: form.value.strip_prefix,
      repo_url: form.value.use_git ? form.value.repo_url.trim() : '',
      branch: form.value.use_git ? form.value.branch.trim() : '',
      git_subdir: form.value.use_git ? form.value.git_subdir.trim() : '',
      git_depth: form.value.use_git ? form.value.git_depth : 0,
      build_output: isStatic.value ? form.value.build_output.trim() : '',
    })
    const d = (res as any)?.data
    // 部署会改站点配置（自动挂载反代 + 同步）：通知站点面板刷新
    window.dispatchEvent(new CustomEvent('zap:sites-changed'))
    // 部署是后台长任务：关闭向导并打开任务日志抽屉，实时查看 git 拉取 / 安装 / 构建进度
    const taskId = d?.task_id
    if (taskId) {
      ElMessage.success(t('site.appDeployStarted'))
      visible.value = false
      emit('done')
      logDrawer.value?.openDrawer(
        taskId,
        t('site.appDeployTaskTitle', { name: form.value.name }),
      )
      return
    }
    ElMessage.success(
      d?.name ? t('site.appDeployOk', { name: d.name }) : t('site.appDeploySubmitted'),
    )
    if (d?.mounted) {
      ElMessage.info(t('site.appMounted', { path: d.mounted, port: d.port }))
    } else if (d?.synced === false) {
      ElMessage.warning(t('site.appSyncFailed'))
    }
    visible.value = false
    emit('done')
  } catch (e: any) {
    // 后端带原因（端口被占、venv 创建失败…）就显示原因，别只弹一句「系统错误」
    ElMessage.error(e?.message || t('site.appDeployFailed'))
  } finally {
    saving.value = false
  }
}

function onClosed() {
  step.value = 0
}

// 有挂载前缀（非 `/`）时才显示「剥离前缀」开关
const mountHasPrefix = computed(() => {
  const p = form.value.mount_path.trim() || '/'
  return p !== '/'
})

// 挂载点从 `/` 改成子路径时默认勾上（用户手动改过之后不再自动动它）
watch(
  () => form.value.mount_path,
  (v, old) => {
    const cur = (v || '').trim() || '/'
    const prev = (old || '').trim() || '/'
    if (cur !== '/' && prev === '/') form.value.strip_prefix = true
    if (cur === '/') form.value.strip_prefix = false
  },
)

watch(visible, (v) => {
  if (!v) return
  loadMeta().then(() => {
    // 打开时按入口预填
    if (props.siteId) {
      form.value.target = 'site'
      form.value.site_id = props.siteId
      const s = sites.value.find((x) => x.id === props.siteId)
      if (s && !form.value.workdir) form.value.workdir = ''
    } else if (!props.initial) {
      // 独立部署：默认落在第一个站点上，用户可切到「新建反代站点」
      if (!form.value.site_id && sites.value.length) form.value.site_id = sites.value[0].id
    }
    if (props.initial) {
      const i = props.initial as any
      form.value.target = i.site_id ? 'site' : 'domain'
      form.value.site_id = i.site_id || 0
      form.value.domain = i.domain || ''
      form.value.name = i.name || ''
      form.value.app_type = i.app_type || 'python'
      form.value.runtime_version = i.runtime_version || ''
      form.value.mount_path = i.mount_path || '/'
      form.value.match_mode = i.match_mode || ''
      form.value.strip_prefix =
        i.strip_prefix ?? (form.value.mount_path.trim() || '/') !== '/'
      form.value.workdir = i.workdir || ''
      form.value.entry = i.entry || ''
      form.value.build_cmd = i.build_cmd || ''
      form.value.command = i.command || ''
      form.value.port = i.port || 0
      form.value.auto_port = false
      form.value.env = i.env || ''
      form.value.create_venv = i.create_venv !== false
      form.value.install_deps = i.install_deps !== false
      // git 部署元数据回显
      form.value.repo_url = i.repo_url || ''
      form.value.branch = i.branch || ''
      form.value.git_subdir = i.git_subdir || ''
      form.value.build_output = i.build_output || ''
      form.value.git_depth = i.git_depth || 0
      form.value.use_git = !!(i.repo_url || i.use_git)
    }
  })
})
</script>

<style scoped>
.wz-steps {
  margin-bottom: 20px;
}
.wz-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}
.wz-tip {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--el-text-color-secondary);
}
.wz-tip-inline {
  margin-left: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.wz-summary {
  padding: 2px 4px;
}
.wz-note {
  margin-top: 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.wz-dim {
  color: var(--el-text-color-secondary);
}

.mount-row {
  display: flex;
  gap: 8px;
  width: 100%;
}

.mount-mode {
  width: 110px;
}

.mount-path {
  flex: 1;
}

.mount-switch {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  font-size: 12px;
  color: #606266;
}

.mount-switch-hint {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 1px solid #c0c4cc;
  font-size: 10px;
  cursor: default;
}
</style>
