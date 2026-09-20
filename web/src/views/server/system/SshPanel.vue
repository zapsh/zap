<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'

interface SshInfo {
  installed: boolean
  running: boolean
  port: number
  version: string
}

const { t } = useI18n()
const router = useRouter()

const sshInfo = ref<SshInfo | null>(null)
const acting = ref(false)

// ── 安装 ─────────────────────────────────────────────────────
const installDialog = ref(false)
const installLog = ref('')
const installDone = ref(false)
const installOk = ref(false)
const logBoxRef = ref<HTMLElement>()
let installTimer: ReturnType<typeof setTimeout> | null = null
let installOffset = 0

// ── 配置编辑 ─────────────────────────────────────────────────
const configDialog = ref(false)
const configContent = ref('')
const configSaving = ref(false)

async function loadSsh() {
  try {
    const res = await http.get<{ code: number; data: SshInfo }>('/system/config/ssh/status')
    sshInfo.value = res.data
  } catch {
    /* handled */
  }
}

async function restartSsh() {
  try {
    await ElMessageBox.confirm(t('serverSsh.restartConfirm'), t('serverSsh.warnTitle'), {
      type: 'warning',
      confirmButtonText: t('serverSsh.confirmRestart'),
    })
  } catch {
    return
  }
  acting.value = true
  try {
    const res = await http.post<{ code: number; message: string }>('/system/config/ssh/restart')
    ElMessage.success(res.message ?? t('serverSsh.restartOk'))
    loadSsh()
  } catch {
    /* handled */
  } finally {
    acting.value = false
  }
}

// 通过通用服务接口启动/停止 sshd（部分系统为 ssh.service，失败时自动重试）
async function actionSsh(action: 'start' | 'stop') {
  try {
    await ElMessageBox.confirm(
      action === 'start' ? t('serverSsh.startConfirm') : t('serverSsh.stopConfirm'),
      action === 'stop' ? t('serverSsh.warnTitle') : t('common.tip'),
      { type: action === 'stop' ? 'warning' : 'info' },
    )
  } catch {
    return
  }
  acting.value = true
  try {
    const run = (svc: string) =>
      http.post<{ code: number; message: string }>('/system/config/services/action', {
        name: svc,
        action,
      })
    const res = await run('sshd.service').catch(() => run('ssh.service'))
    ElMessage.success(
      res.message ??
        t('serverSsh.actionOk', {
          action: action === 'start' ? t('serverSsh.start') : t('serverSsh.stop'),
        }),
    )
    loadSsh()
  } catch {
    /* handled */
  } finally {
    acting.value = false
  }
}

function openTerminal() {
  router.push('/terminal')
}

async function startInstall() {
  try {
    await ElMessageBox.confirm(t('serverSsh.installConfirm'), t('serverSsh.installTitle'), {
      type: 'info',
    })
  } catch {
    return
  }
  installLog.value = ''
  installDone.value = false
  installOk.value = false
  installOffset = 0
  try {
    const res = await http.post<{ code: number; data: { run_id: string } }>(
      '/system/config/ssh/install',
    )
    const runId = res.data.run_id
    installDialog.value = true
    pollInstallLog(runId)
  } catch {
    /* handled */
  }
}

async function pollInstallLog(runId: string) {
  try {
    const res = await http.get<{
      code: number
      data: { content: string; done: boolean; status: string; exit_code: number }
    }>(`/system/config/ssh/install/log/${runId}?offset=${installOffset}`)
    if (res.data.content) {
      installLog.value += res.data.content
      installOffset += res.data.content.length
      nextTick(() => logBoxRef.value?.scrollTo({ top: logBoxRef.value.scrollHeight }))
    }
    if (res.data.done) {
      installDone.value = true
      installOk.value = res.data.status === 'success' || res.data.exit_code === 0
      loadSsh()
      return
    }
    installTimer = setTimeout(() => pollInstallLog(runId), 1000)
  } catch {
    installTimer = setTimeout(() => pollInstallLog(runId), 1500)
  }
}

function closeInstallDialog() {
  installDialog.value = false
  if (installTimer) {
    clearTimeout(installTimer)
    installTimer = null
  }
}

async function editConfig() {
  try {
    const res = await http.get<{ code: number; data: { content: string } }>('/system/files/read', {
      params: { path: '/etc/ssh/sshd_config' },
    })
    configContent.value = res.data.content
    configDialog.value = true
  } catch {
    /* handled */
  }
}

async function saveConfig(restart = false) {
  configSaving.value = true
  try {
    await http.post('/system/files/write', {
      path: '/etc/ssh/sshd_config',
      content: configContent.value,
    })
    ElMessage.success(t('serverSsh.configSaved'))
    if (restart) {
      // 直接重启（重启内部已有确认弹窗）
      try {
        await http.post('/system/config/ssh/restart')
        ElMessage.success(t('serverSsh.configRestarted'))
      } catch {
        /* handled */
      }
      configDialog.value = false
      loadSsh()
    }
  } catch {
    /* handled */
  } finally {
    configSaving.value = false
  }
}

onMounted(loadSsh)
onUnmounted(() => {
  if (installTimer) clearTimeout(installTimer)
})
</script>

<template>
  <div class="ssh-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>{{ t('serverSsh.title') }}</span>
          <el-tag
            v-if="sshInfo"
            :type="sshInfo.installed ? (sshInfo.running ? 'success' : 'danger') : 'info'"
            size="small"
          >
            {{
              sshInfo.installed
                ? sshInfo.running
                  ? t('serverSsh.running')
                  : t('serverSsh.stopped')
                : t('serverSsh.notInstalled')
            }}
          </el-tag>
        </div>
      </template>

      <template v-if="!sshInfo || sshInfo.installed">
        <el-descriptions v-if="sshInfo" :column="2" border>
          <el-descriptions-item :label="t('serverSsh.runState')">
            <el-tag :type="sshInfo.running ? 'success' : 'danger'">
              {{ sshInfo.running ? t('serverSsh.running') : t('serverSsh.stopped') }}
            </el-tag>
          </el-descriptions-item>
          <el-descriptions-item :label="t('serverSsh.listenPort')">{{
            sshInfo.port
          }}</el-descriptions-item>
          <el-descriptions-item :label="t('serverSsh.version')" :span="2">
            {{ sshInfo.version }}
          </el-descriptions-item>
        </el-descriptions>
        <el-empty v-else :description="t('serverSsh.loadingStatus')" :image-size="60" />

        <div v-if="sshInfo" style="margin-top: 16px; display: flex; gap: 12px; flex-wrap: wrap">
          <el-button
            type="success"
            :loading="acting"
            :disabled="!sshInfo?.installed || sshInfo?.running"
            @click="actionSsh('start')"
          >
            {{ t('serverSsh.start') }}
          </el-button>
          <el-button
            type="danger"
            :loading="acting"
            :disabled="!sshInfo?.running"
            @click="actionSsh('stop')"
          >
            {{ t('serverSsh.stop') }}
          </el-button>
          <el-button
            type="warning"
            :loading="acting"
            :disabled="!sshInfo?.running"
            @click="restartSsh"
          >
            {{ t('serverSsh.restart') }}
          </el-button>
          <el-button type="primary" plain :disabled="!sshInfo?.installed" @click="editConfig">
            {{ t('serverSsh.editConfig') }}
          </el-button>
        </div>
      </template>

      <template v-else>
        <el-empty :description="t('serverSsh.notDetected')" :image-size="80">
          <div class="empty-actions">
            <el-button type="primary" @click="startInstall">
              {{ t('serverSsh.installServer') }}
            </el-button>
            <el-button @click="openTerminal">{{ t('serverSsh.openTerminal') }}</el-button>
          </div>
          <div class="empty-tip">{{ t('serverSsh.manualInstallTip') }}</div>
        </el-empty>
      </template>
    </el-card>

    <!-- 安装进度对话框 -->
    <el-dialog
      v-model="installDialog"
      :title="t('serverSsh.installTitle')"
      width="680px"
      :close-on-click-modal="false"
      :close-on-press-escape="false"
    >
      <pre ref="logBoxRef" class="install-log">{{ installLog }}</pre>
      <div v-if="installDone" class="install-result" :class="installOk ? 'ok' : 'err'">
        {{ installOk ? t('serverSsh.installOk') : t('serverSsh.installFailed') }}
      </div>
      <template #footer>
        <el-button v-if="!installDone" disabled>{{ t('serverSsh.installing') }}</el-button>
        <el-button @click="closeInstallDialog">{{ t('serverSsh.close') }}</el-button>
        <el-button v-if="installDone" type="primary" @click="closeInstallDialog">
          {{ t('serverSsh.finish') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 配置编辑对话框 -->
    <el-dialog v-model="configDialog" :title="t('serverSsh.configTitle')" width="780px" top="5vh">
      <el-input
        v-model="configContent"
        type="textarea"
        :rows="22"
        class="config-editor"
        spellcheck="false"
      />
      <div class="config-tip">{{ t('serverSsh.configTip') }}</div>
      <template #footer>
        <el-button @click="configDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button :loading="configSaving" @click="saveConfig(false)">
          {{ t('serverSsh.saveOnly') }}
        </el-button>
        <el-button type="primary" :loading="configSaving" @click="saveConfig(true)">
          {{ t('serverSsh.saveAndRestart') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.ssh-container {
  padding: 0;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.empty-actions {
  display: flex;
  gap: 12px;
  justify-content: center;
  margin-top: 8px;
}
.empty-tip {
  margin-top: 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.install-log {
  margin: 0;
  height: 300px;
  overflow: auto;
  background: #0d1117;
  color: #c9d1d9;
  font-family: 'JetBrains Mono', Consolas, Menlo, monospace;
  font-size: 12px;
  line-height: 1.6;
  padding: 12px;
  border-radius: 6px;
  white-space: pre-wrap;
  word-break: break-all;
}
.install-result {
  margin-top: 12px;
  font-weight: 600;
}
.install-result.ok {
  color: #67c23a;
}
.install-result.err {
  color: #f56c6c;
}
.config-editor :deep(textarea) {
  font-family: 'JetBrains Mono', Consolas, Menlo, monospace;
  font-size: 12px;
  line-height: 1.6;
}
.config-tip {
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
}
</style>
