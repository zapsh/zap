<template>
  <div class="system-update">
    <!-- 非管理员：能看版本与升级历史，但执行类操作已禁用 -->
    <el-alert
      v-if="!isAdmin"
      class="mb"
      type="info"
      :closable="false"
      :show-icon="true"
      :title="t('sysUpdate.readonlyTip')"
    />

    <!-- 当前版本与手动升级 -->
    <el-card shadow="never" class="mb">
      <template #header>
        <div class="card-header">
          <span>{{ t('sysUpdate.versionCard') }}</span>
          <el-button v-if="status.upgrading" type="warning" plain :loading="true" size="small">
            {{ t('sysUpdate.upgrading') }}
          </el-button>
          <template v-else>
            <!-- 检查 / 升级 = 执行动作，仅管理员；其他人只读 -->
            <el-tooltip :disabled="isAdmin" :content="t('sysUpdate.adminOnly')" placement="top">
              <span>
                <el-button
                  type="primary"
                  plain
                  size="small"
                  :loading="checking"
                  :disabled="!isAdmin"
                  @click="onCheck"
                >
                  {{ hasChecked ? t('sysUpdate.recheck') : t('sysUpdate.check') }}
                </el-button>
              </span>
            </el-tooltip>
            <el-tooltip :disabled="isAdmin" :content="t('sysUpdate.adminOnly')" placement="top">
              <span>
                <el-button
                  type="danger"
                  plain
                  size="small"
                  :disabled="!isAdmin || !hasChecked"
                  @click="onApply"
                >
                  {{ t('sysUpdate.applyNow') }}
                </el-button>
              </span>
            </el-tooltip>
          </template>
        </div>
      </template>

      <el-descriptions :column="2" border>
        <el-descriptions-item label="Zap">
          <span class="ver-highlight">v{{ status.zapd_version || '-' }}</span>
          <el-tooltip v-if="!status.zapexec_version" :content="t('sysUpdate.zapexecUnreachable')">
            <el-icon class="warn-icon"><Warning /></el-icon>
          </el-tooltip>
          <span v-else-if="status.zapexec_version !== status.zapd_version" class="ver-sub">
            {{ t('sysUpdate.zapexecVersion', { version: status.zapexec_version }) }}
          </span>
        </el-descriptions-item>
        <el-descriptions-item label="Web">
          <span class="ver-highlight">v{{ WEB_VERSION || '-' }}</span>
        </el-descriptions-item>
      </el-descriptions>

      <el-alert
        v-if="checkMsg"
        class="mt"
        :type="checkMsg.type"
        :closable="false"
        :show-icon="true"
        :title="checkMsg.text"
      />
    </el-card>

    <!-- 自动更新设置 -->
    <el-card shadow="never" class="mb">
      <template #header>
        <div class="card-header">
          <span>{{ t('sysUpdate.autoCard') }}</span>
          <el-tooltip :disabled="isAdmin" :content="t('sysUpdate.adminOnly')" placement="top">
            <span>
              <el-button
                type="primary"
                size="small"
                :loading="saving"
                :disabled="!isAdmin"
                @click="onSaveConfig"
              >
                {{ t('sysUpdate.saveConfig') }}
              </el-button>
            </span>
          </el-tooltip>
        </div>
      </template>

      <el-form label-width="120px" class="auto-form" @submit.prevent>
        <el-form-item :label="t('sysUpdate.enableAuto')">
          <el-switch v-model="form.auto" :disabled="!isAdmin" />
          <span class="form-hint">{{ t('sysUpdate.enableAutoHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('sysUpdate.cron')">
          <el-input
            v-model="form.cron"
            class="w-320"
            :disabled="!isAdmin"
            :placeholder="t('sysUpdate.cronPlaceholder')"
          />
          <span class="form-hint">{{ t('sysUpdate.cronHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('sysUpdate.channel')">
          <el-input
            v-model="form.channel"
            class="w-480"
            :disabled="!isAdmin"
            placeholder="https://mirrors.zap.cn/zap/releases"
          />
          <span class="form-hint">{{ t('sysUpdate.channelHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('sysUpdate.lastCheck')">
          <span class="muted">
            <template v-if="status.config.last_check_at">
              {{ fmtTime(status.config.last_check_at) }} ·
              {{
                status.config.last_check_has_update
                  ? t('sysUpdate.foundNewVersion', { version: status.config.last_check_version })
                  : status.config.last_check_version
                    ? t('sysUpdate.upToDate')
                    : t('sysUpdate.checkFailed')
              }}
            </template>
            <template v-else>{{ t('sysUpdate.neverChecked') }}</template>
            <span v-if="status.config.last_error" class="err-text">
              ({{ status.config.last_error }})
            </span>
          </span>
        </el-form-item>
      </el-form>
    </el-card>

    <!-- 升级日志（进行中） -->
    <el-card v-if="liveLog" shadow="never" class="mb">
      <template #header>
        <div class="card-header">
          <span>{{ t('sysUpdate.liveLogTitle', { id: liveRunId }) }}</span>
          <el-button size="small" @click="closeLiveLog">{{ t('sysUpdate.close') }}</el-button>
        </div>
      </template>
      <pre class="log-box">{{ liveLog }}</pre>
    </el-card>

    <!-- 升级历史 -->
    <el-card shadow="never">
      <template #header>
        <span>{{ t('sysUpdate.historyTitle') }}</span>
      </template>
      <el-table :data="status.recent_runs" size="small" :empty-text="t('sysUpdate.emptyHistory')">
        <el-table-column :label="t('sysUpdate.colVersion')" width="140">
          <template #default="{ row }">v{{ row.pkg }}</template>
        </el-table-column>
        <el-table-column :label="t('sysUpdate.colResult')" width="110">
          <template #default="{ row }">
            <el-tag :type="tagType(row.status)" size="small">
              {{
                row.status === 'running'
                  ? t('sysUpdate.statusRunning')
                  : row.status === 'success'
                    ? t('sysUpdate.statusSuccess')
                    : t('sysUpdate.statusFailed')
              }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('sysUpdate.colStartedAt')">
          <template #default="{ row }">{{ fmtTime(row.started_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="120" align="right">
          <template #default="{ row }">
            <el-button link type="primary" @click="openHistoryLog(row)">
              {{ t('sysUpdate.viewLog') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 历史日志对话框 -->
    <el-dialog
      v-model="historyVisible"
      :title="t('sysUpdate.historyLogTitle')"
      width="720px"
      append-to-body
      destroy-on-close
    >
      <pre class="log-box">{{ historyLog || t('sysUpdate.noLogContent') }}</pre>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Warning } from '@/icons'
import { useUserStore } from '@/stores/user'
import {
  applyUpdate,
  checkForUpdate,
  getUpdateLog,
  getUpdateStatus,
  saveUpdateConfig,
  type UpdateStatusData,
  type UpdateRunInfo,
} from '@/api/systemUpdate'

const WEB_VERSION = import.meta.env.VITE_WEB_VERSION || ''

const { t } = useI18n()
const userStore = useUserStore()

/**
 * 更新是「管理员执行、其他人可看」：状态与升级日志的后端门禁已放宽到登录用户，
 * 这里只负责把执行类按钮置灰（真被绕过后端还会再挡一次 admin）。
 */
const isAdmin = computed(() => userStore.roles.includes('admin'))

const status = reactive<UpdateStatusData>({
  zapd_version: '',
  zapexec_version: '',
  config: {
    auto: false,
    cron: '0 3 * * *',
    channel: 'https://mirrors.zap.cn/zap/releases',
    last_check_at: 0,
    last_check_version: '',
    last_check_has_update: false,
    last_error: '',
  },
  upgrading: false,
  current_run: null,
  recent_runs: [],
})

const form = reactive({ auto: false, cron: '', channel: '' })

const checking = ref(false)
const saving = ref(false)
const hasChecked = ref(false)
const checkMsg = ref<{ type: 'success' | 'info' | 'warning' | 'error'; text: string } | null>(null)

const liveLog = ref('')
const liveRunId = ref('')
const liveOffset = ref(0)
let pollTimer: ReturnType<typeof setInterval> | null = null

const historyVisible = ref(false)
const historyLog = ref('')

function fmtTime(ts?: number | null): string {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

function tagType(s: string): 'success' | 'danger' | 'primary' {
  if (s === 'success') return 'success'
  if (s === 'failed') return 'danger'
  return 'primary'
}

function applyConfigFromServer() {
  const c = status.config
  form.auto = c.auto
  form.cron = c.cron || '0 3 * * *'
  form.channel = c.channel || 'https://mirrors.zap.cn/zap/releases'
}

async function load() {
  try {
    const res = await getUpdateStatus()
    Object.assign(status, res.data)
    applyConfigFromServer()
    // 若存在进行中的升级（本页打开前触发），接续展示日志
    if (res.data.current_run && res.data.current_run.status === 'running') {
      startPoll(res.data.current_run.run_id, res.data.current_run.log_path || '')
    }
  } catch {
    // 拦截器已提示
  }
}

async function onCheck() {
  checking.value = true
  try {
    const res = await checkForUpdate()
    const d = res.data
    hasChecked.value = true
    if (d.has_update) {
      checkMsg.value = {
        type: 'warning',
        text: t('sysUpdate.newVersionMsg', { current: d.current, latest: d.latest }),
      }
    } else {
      checkMsg.value = { type: 'success', text: t('sysUpdate.latestMsg', { version: d.current }) }
    }
    // 同步 last_check
    const st = await getUpdateStatus()
    Object.assign(status.config, st.data.config)
  } catch (e: any) {
    checkMsg.value = {
      type: 'error',
      text: t('sysUpdate.checkError', { msg: String(e?.message || e) }),
    }
  } finally {
    checking.value = false
  }
}

function validateCron(expr: string): boolean {
  const parts = expr.trim().split(/\s+/)
  if (parts.length !== 5) return false
  return parts.every((p) => /^[\d*,\-/]+$/.test(p))
}

async function onSaveConfig() {
  if (!validateCron(form.cron)) {
    ElMessage.warning(t('sysUpdate.invalidCron'))
    return
  }
  if (!/^https?:\/\//.test(form.channel)) {
    ElMessage.warning(t('sysUpdate.invalidChannel'))
    return
  }
  saving.value = true
  try {
    await saveUpdateConfig({
      auto: form.auto,
      cron: form.cron.trim(),
      channel: form.channel.trim().replace(/\/+$/, ''),
    })
    ElMessage.success(t('sysUpdate.configSaved'))
    Object.assign(status.config, {
      auto: form.auto,
      cron: form.cron.trim(),
      channel: form.channel.trim().replace(/\/+$/, ''),
    })
  } catch {
    // 拦截器已提示
  } finally {
    saving.value = false
  }
}

async function onApply() {
  try {
    await ElMessageBox.confirm(t('sysUpdate.applyConfirm'), t('sysUpdate.applyConfirmTitle'), {
      type: 'warning',
      confirmButtonText: t('sysUpdate.applyNow'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  try {
    const res = await applyUpdate()
    status.upgrading = true
    ElMessage.success(t('sysUpdate.applyStarted', { version: res.data.latest }))
    startPoll(res.data.run_id, res.data.log_path)
  } catch (e: any) {
    checkMsg.value = {
      type: 'error',
      text: t('sysUpdate.applyError', { msg: String(e?.message || e) }),
    }
  }
}

function startPoll(runId: string, logPath: string) {
  stopPoll()
  liveRunId.value = runId
  liveOffset.value = 0
  liveLog.value = `${t('sysUpdate.waitingUpgrader')}\n`
  pollTimer = setInterval(pollLog, 1500)
  pollLog()
}

async function pollLog() {
  if (!liveRunId.value) return
  try {
    const res = await getUpdateLog(liveRunId.value, liveOffset.value)
    const d = res.data
    if (d.log) {
      liveLog.value += d.log
      liveOffset.value = d.offset
    }
    if (d.done) {
      stopPoll()
      const ok = d.exit_code === 0
      ElMessage({
        type: ok ? 'success' : 'error',
        message: ok ? t('sysUpdate.upgradeOk') : t('sysUpdate.upgradeFailed'),
        duration: 6000,
      })
      // 稍等面板重启后刷新状态
      setTimeout(async () => {
        stopPoll()
        status.upgrading = false
        await load()
      }, 1500)
    }
  } catch {
    // zapd 正在重启导致请求中断 → 提示用户刷新查看
    stopPoll()
    ElMessage.warning(t('sysUpdate.connInterrupted'))
  }
}

function stopPoll() {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

function closeLiveLog() {
  stopPoll()
  liveLog.value = ''
  liveRunId.value = ''
}

async function openHistoryLog(row: UpdateRunInfo) {
  historyVisible.value = true
  historyLog.value = ''
  try {
    const res = await getUpdateLog(row.run_id, 0)
    historyLog.value = res.data.log || t('sysUpdate.logCleared')
  } catch {
    historyLog.value = t('sysUpdate.logReadFailed')
  }
}

onMounted(() => {
  load()
})

onUnmounted(() => {
  stopPoll()
})
</script>

<style scoped>
.mb {
  margin-bottom: 16px;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.ver-highlight {
  font-weight: 600;
  color: var(--el-color-primary);
}
.warn-icon {
  margin-left: 4px;
  color: var(--el-color-warning);
  vertical-align: -2px;
}
.ver-sub {
  margin-left: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.mt {
  margin-top: 16px;
}
.auto-form .el-form-item {
  margin-bottom: 6px;
}
.w-320 {
  width: 320px;
}
.w-480 {
  width: 480px;
}
.form-hint {
  margin-left: 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.muted {
  color: var(--el-text-color-secondary);
}
.err-text {
  color: var(--el-color-danger);
}
.log-box {
  max-height: 420px;
  overflow: auto;
  margin: 0;
  padding: 12px;
  border-radius: 6px;
  background: #0d1117;
  color: #c9d1d9;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
}
</style>
