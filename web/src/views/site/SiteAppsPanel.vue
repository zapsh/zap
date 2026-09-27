<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete, Plus, Refresh } from '@/icons'
import {
  deploySiteApp,
  getSiteAppCaps,
  getSiteAppLog,
  getSiteApps,
  removeSiteApp,
  siteAppAction,
} from '@/api/site'
import type { SiteApp } from '@/api/site'

const props = defineProps<{ siteId: number }>()
const { t } = useI18n()

const caps = ref<{ allowed: boolean; types: string[]; max_apps: number }>({
  allowed: false,
  types: [],
  max_apps: 0,
})
const apps = ref<SiteApp[]>([])
const loading = ref(false)

// ── 部署 / 编辑 ─────────────────────────────────────────
const dialog = ref(false)
const editing = ref('')
const form = ref({
  name: '',
  app_type: 'python',
  workdir: '',
  entry: '',
  command: '',
  port: 0,
  env: '',
  autostart: true,
  install_deps: false,
})
const submitting = ref(false)

const typeOptions = computed(() =>
  (caps.value.types.length ? caps.value.types : ['python', 'nodejs']).map((v) => ({
    value: v,
    label: v === 'python' ? 'Python' : v === 'nodejs' ? 'Node.js' : v,
  })),
)

const placeholders = computed(() =>
  form.value.app_type === 'nodejs'
    ? { entry: 'server.js', command: 'npm start' }
    : { entry: 'main.py', command: '' },
)

async function load() {
  if (!props.siteId) return
  loading.value = true
  try {
    const c = await getSiteAppCaps(props.siteId)
    caps.value = (c.data || caps.value) as typeof caps.value
    const l = await getSiteApps(props.siteId)
    apps.value = (l.data as any)?.apps ?? []
  } finally {
    loading.value = false
  }
}

function openAdd() {
  editing.value = ''
  form.value = {
    name: '',
    app_type: caps.value.types[0] ?? 'python',
    workdir: '',
    entry: '',
    command: '',
    port: 0,
    env: '',
    autostart: true,
    install_deps: false,
  }
  dialog.value = true
}

function openEdit(a: SiteApp) {
  editing.value = a.name
  form.value = {
    name: a.name,
    app_type: a.app_type,
    workdir: a.workdir,
    entry: a.entry,
    command: a.command,
    port: a.port,
    env: a.env,
    autostart: a.autostart,
    install_deps: false,
  }
  dialog.value = true
}

async function submit() {
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(form.value.name)) {
    ElMessage.warning(t('site.appNameInvalid'))
    return
  }
  submitting.value = true
  try {
    const r = await deploySiteApp({
      site_id: props.siteId,
      name: form.value.name,
      app_type: form.value.app_type,
      workdir: form.value.workdir || undefined,
      entry: form.value.entry || undefined,
      command: form.value.command || undefined,
      port: form.value.port || undefined,
      env: form.value.env || undefined,
      autostart: form.value.autostart,
      install_deps: form.value.install_deps,
    })
    ElMessage.success(r.data.message || t('site.appDeployed'))
    dialog.value = false
    await load()
  } finally {
    submitting.value = false
  }
}

async function act(a: SiteApp, action: string) {
  await siteAppAction(props.siteId, a.name, action)
  ElMessage.success(t('common.success'))
  await load()
}

async function drop(a: SiteApp) {
  await ElMessageBox.confirm(t('site.appDeleteConfirm', { name: a.name }), t('common.tip'), {
    type: 'warning',
  })
  await removeSiteApp(props.siteId, a.name)
  ElMessage.success(t('common.success'))
  await load()
}

// ── 日志 ───────────────────────────────────────────────
const logVisible = ref(false)
const logName = ref('')
const logLines = ref<string[]>([])

async function openLog(a: SiteApp) {
  logName.value = a.name
  const r = await getSiteAppLog(props.siteId, a.name)
  logLines.value = (r.data as any)?.lines ?? []
  logVisible.value = true
}

function stateType(s: string): 'success' | 'danger' | 'info' {
  if (s === 'active') return 'success'
  if (s === 'unknown') return 'info'
  return 'danger'
}

onMounted(load)
watch(() => props.siteId, load)
</script>

<template>
  <div class="apps-panel">
    <div v-if="!caps.allowed" class="apps-gated">
      <el-empty :image-size="70" :description="t('site.appGated')" />
    </div>

    <template v-else>
      <div class="apps-bar">
        <el-button type="primary" size="small" :icon="Plus" @click="openAdd">
          {{ t('site.appAdd') }}
        </el-button>
        <el-button size="small" :icon="Refresh" :loading="loading" @click="load">
          {{ t('common.refresh') }}
        </el-button>
        <span class="apps-hint">{{ t('site.appRunAsHint') }}</span>
      </div>

      <el-table :data="apps" size="small" v-loading="loading" style="width: 100%">
        <el-table-column prop="name" :label="t('site.appName')" min-width="120" />
        <el-table-column :label="t('site.appType')" width="90">
          <template #default="{ row }">
            {{ row.app_type === 'nodejs' ? 'Node.js' : row.app_type }}
          </template>
        </el-table-column>
        <el-table-column :label="t('site.appPort')" width="90">
          <template #default="{ row }">{{ row.port || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('site.appState')" width="110">
          <template #default="{ row }">
            <el-tag :type="stateType(row.state)" size="small">{{ row.state }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="workdir" :label="t('site.appWorkdir')" min-width="180" show-overflow-tooltip />
        <el-table-column :label="t('common.operation')" width="300">
          <template #default="{ row }">
            <el-button v-if="!row.active" link type="success" size="small" @click="act(row, 'start')">
              {{ t('site.appStart') }}
            </el-button>
            <el-button v-else link type="warning" size="small" @click="act(row, 'stop')">
              {{ t('site.appStop') }}
            </el-button>
            <el-button link type="primary" size="small" @click="act(row, 'restart')">
              {{ t('site.appRestart') }}
            </el-button>
            <el-button link size="small" @click="openLog(row)">{{ t('site.appLog') }}</el-button>
            <el-button link size="small" @click="openEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button link type="danger" size="small" :icon="Delete" @click="drop(row)" />
          </template>
        </el-table-column>
        <template #empty>
          <span class="apps-empty">{{ t('site.appEmpty') }}</span>
        </template>
      </el-table>

      <el-dialog v-model="dialog" :title="editing ? t('site.appEdit') : t('site.appAdd')" width="560px">
        <el-form label-width="130px" @submit.prevent>
          <el-form-item :label="t('site.appName')" required>
            <el-input v-model="form.name" :disabled="!!editing" placeholder="myapp" />
          </el-form-item>
          <el-form-item :label="t('site.appType')">
            <el-select v-model="form.app_type" style="width: 100%">
              <el-option v-for="o in typeOptions" :key="o.value" :label="o.label" :value="o.value" />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('site.appWorkdir')">
            <el-input v-model="form.workdir" :placeholder="t('site.appWorkdirPh')" />
            <div class="apps-tip">{{ t('site.appWorkdirTip') }}</div>
          </el-form-item>
          <el-form-item :label="t('site.appEntry')">
            <el-input v-model="form.entry" :placeholder="placeholders.entry" />
          </el-form-item>
          <el-form-item :label="t('site.appCommand')">
            <el-input v-model="form.command" :placeholder="placeholders.command" />
            <div class="apps-tip">{{ t('site.appCommandTip') }}</div>
          </el-form-item>
          <el-form-item :label="t('site.appPort')">
            <el-input-number v-model="form.port" :min="0" :max="65535" />
            <span class="apps-tip-inline">{{ t('site.appPortTip') }}</span>
          </el-form-item>
          <el-form-item :label="t('site.appEnv')">
            <el-input v-model="form.env" type="textarea" :rows="3" placeholder="DEBUG=0&#10;SECRET=xxx" />
          </el-form-item>
          <el-form-item :label="t('site.appAutostart')">
            <el-switch v-model="form.autostart" />
          </el-form-item>
          <el-form-item :label="t('site.appInstallDeps')">
            <el-switch v-model="form.install_deps" />
            <span class="apps-tip-inline">{{ t('site.appInstallDepsTip') }}</span>
          </el-form-item>
        </el-form>
        <template #footer>
          <el-button @click="dialog = false">{{ t('common.cancel') }}</el-button>
          <el-button type="primary" :loading="submitting" @click="submit">
            {{ t('site.appDeploy') }}
          </el-button>
        </template>
      </el-dialog>

      <el-drawer v-model="logVisible" :title="`${t('site.appLog')} — ${logName}`" size="60%">
        <pre class="apps-log">{{ logLines.join('\n') || t('site.appLogEmpty') }}</pre>
      </el-drawer>
    </template>
  </div>
</template>

<style scoped>
.apps-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}
.apps-hint,
.apps-tip,
.apps-tip-inline {
  color: var(--el-text-color-secondary, #909399);
  font-size: 12px;
}
.apps-tip-inline {
  margin-left: 8px;
}
.apps-empty {
  color: #909399;
  font-size: 13px;
}
.apps-log {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
</style>
