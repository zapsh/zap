<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!--
  sshd_config 编辑器：可视化（原地改键） + 文件编辑（全文）两种模式。
  两条路径都在服务端强制 `sshd -t` 校验 + 自动备份,失败不落盘,避免把人锁在门外。
-->
<template>
  <el-dialog
    v-model="visible"
    :title="t('serverSsh.configTitle')"
    width="900px"
    top="4vh"
    :close-on-click-modal="false"
    @open="onOpen"
  >
    <div v-loading="loading">
      <div class="mode-bar">
        <el-radio-group v-model="mode" size="small">
          <el-radio-button value="visual">{{ t('serverSsh.modeVisual') }}</el-radio-button>
          <el-radio-button value="file">{{ t('serverSsh.modeFile') }}</el-radio-button>
        </el-radio-group>
        <span class="path mono">{{ path }}</span>
        <el-tag v-if="loaded" size="small" :type="valid ? 'success' : 'danger'">
          {{ valid ? t('serverSsh.validOk') : t('serverSsh.validBad') }}
        </el-tag>
      </div>

      <el-alert type="warning" show-icon :closable="false" :title="t('serverSsh.lockWarn')" class="mt-2" />

      <!-- 可视化 -->
      <el-tabs v-show="mode === 'visual'" v-model="tab" class="mt-2">
        <el-tab-pane v-for="g in GROUPS" :key="g.title" :label="g.title" :name="g.title">
          <el-form label-width="230px" class="opt-form">
            <el-form-item v-for="d in g.items" :key="d.key">
              <template #label>
                <span class="mono">{{ d.key }}</span>
                <el-tooltip v-if="d.tip" :content="d.tip" placement="top">
                  <el-icon class="tip-icon"><InfoFilled /></el-icon>
                </el-tooltip>
              </template>
              <!-- yes / no 三态：不改动 -->
              <el-select v-if="d.type === 'yesno'" v-model="values[d.key]" size="small" class="w160">
                <el-option :label="t('serverSsh.keep')" value="" />
                <el-option label="yes" value="yes" />
                <el-option label="no" value="no" />
              </el-select>
              <el-select
                v-else-if="d.options"
                v-model="values[d.key]"
                size="small"
                clearable
                filterable
                allow-create
                default-first-option
                class="w220"
              >
                <el-option v-for="o in d.options" :key="o" :label="o" :value="o" />
              </el-select>
              <el-input
                v-else
                v-model="values[d.key]"
                size="small"
                clearable
                class="w220 mono"
                :placeholder="t('serverSsh.keep')"
              />
              <span class="cur-value">
                <template v-if="current[d.key] !== undefined">
                  {{ t('serverSsh.current') }}<b class="mono">{{ current[d.key] }}</b>
                </template>
                <el-tooltip
                  v-else-if="commented[d.key] !== undefined"
                  :content="commented[d.key]"
                  placement="top"
                >
                  <el-tag size="small" type="info" class="commented-tag">
                    {{ t('serverSsh.commentedValue') }}
                    <b class="mono">{{ commented[d.key] }}</b>
                  </el-tag>
                </el-tooltip>
                <span v-else class="unset">{{ t('serverSsh.unset') }}</span>
              </span>
            </el-form-item>
          </el-form>
        </el-tab-pane>
      </el-tabs>

      <!-- 文件编辑 -->
      <div v-show="mode === 'file'" class="mt-2">
        <CodeEditor v-model="content" lang="ini" :path="path" :active="visible && mode === 'file'" />
      </div>

      <div v-if="checkOut" class="check-out mono">{{ checkOut }}</div>
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ t('common.cancel') }}</el-button>
      <el-button :loading="checking" plain @click="doValidate">
        {{ t('serverSsh.checkOnly') }}
      </el-button>
      <el-button :loading="saving" @click="doSave(false)">{{ t('serverSsh.saveOnly') }}</el-button>
      <el-button type="primary" :loading="saving" @click="doSave(true)">
        {{ t('serverSsh.saveAndRestart') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { InfoFilled } from '@/icons'
import CodeEditor from '@/components/CodeEditor.vue'
import { http } from '@/utils/request'

const { t } = useI18n()

const visible = defineModel<boolean>('modelValue', { required: true })
const emit = defineEmits<{ (e: 'saved'): void }>()

type OptType = 'yesno' | 'text' | 'num'
interface Opt {
  key: string
  type: OptType
  options?: string[]
  tip?: string
}
interface Group {
  title: string
  items: Opt[]
}

const GROUPS: Group[] = [
  {
    title: '网络与端口',
    items: [
      { key: 'Port', type: 'num', tip: '监听端口,改了要用新端口重连' },
      { key: 'ListenAddress', type: 'text', tip: '留空表示监听所有地址,如 0.0.0.0' },
      { key: 'AddressFamily', type: 'text', options: ['any', 'inet', 'inet6'] },
      { key: 'TCPKeepAlive', type: 'yesno' },
    ],
  },
  {
    title: '登录与认证',
    items: [
      {
        key: 'PermitRootLogin',
        type: 'text',
        options: ['yes', 'no', 'prohibit-password', 'forced-commands-only'],
        tip: '禁止 root 直登前请先确认有可用的普通账号',
      },
      {
        key: 'PasswordAuthentication',
        type: 'yesno',
        tip: '关闭前必须确认密钥登录可用,否则会被锁在外面',
      },
      { key: 'PubkeyAuthentication', type: 'yesno' },
      { key: 'AuthorizedKeysFile', type: 'text', tip: '如 .ssh/authorized_keys,可多个空格分隔' },
      { key: 'KbdInteractiveAuthentication', type: 'yesno' },
      {
        key: 'ChallengeResponseAuthentication',
        type: 'yesno',
        tip: '旧名,新版等价于 KbdInteractiveAuthentication',
      },
      { key: 'HostbasedAuthentication', type: 'yesno' },
      { key: 'IgnoreRhosts', type: 'yesno' },
      { key: 'PermitEmptyPasswords', type: 'yesno' },
      { key: 'UsePAM', type: 'yesno' },
      { key: 'StrictModes', type: 'yesno' },
      { key: 'MaxAuthTries', type: 'num' },
      { key: 'LoginGraceTime', type: 'num' },
    ],
  },
  {
    title: '日志与提示',
    items: [
      { key: 'SyslogFacility', type: 'text', options: ['AUTH', 'AUTHPRIV', 'DAEMON', 'USER', 'LOCAL0'] },
      { key: 'LogLevel', type: 'text', options: ['QUIET', 'FATAL', 'ERROR', 'INFO', 'VERBOSE', 'DEBUG'] },
      { key: 'PrintLastLog', type: 'yesno' },
      { key: 'PrintMotd', type: 'yesno' },
      { key: 'Banner', type: 'text', tip: '登录前展示的横幅文件路径,如 /etc/issue.net' },
      { key: 'AcceptEnv', type: 'text', tip: '如 LANG LC_*' },
    ],
  },
  {
    title: '会话与转发',
    items: [
      { key: 'ClientAliveInterval', type: 'num', tip: '秒,0 表示不主动探测' },
      { key: 'ClientAliveCountMax', type: 'num' },
      { key: 'MaxSessions', type: 'num' },
      { key: 'MaxStartups', type: 'text', tip: '如 10:30:60' },
      { key: 'AllowTcpForwarding', type: 'yesno' },
      { key: 'AllowAgentForwarding', type: 'yesno' },
      { key: 'GatewayPorts', type: 'yesno' },
      { key: 'X11Forwarding', type: 'yesno' },
      { key: 'PermitTunnel', type: 'yesno' },
      { key: 'PermitUserEnvironment', type: 'yesno' },
      { key: 'Compression', type: 'text', options: ['yes', 'no', 'delayed'] },
    ],
  },
  {
    title: '高级',
    items: [
      { key: 'Subsystem', type: 'text', tip: '如 sftp /usr/lib/openssh/sftp-server' },
      { key: 'PidFile', type: 'text' },
      { key: 'HostKey', type: 'text', tip: '私钥路径,多个空格分隔' },
    ],
  },
]

const mode = ref<'visual' | 'file'>('visual')
const tab = ref(GROUPS[0].title)
const loading = ref(false)
const saving = ref(false)
const checking = ref(false)
const loaded = ref(false)
const content = ref('')
const values = ref<Record<string, string>>({})
const path = ref('/etc/ssh/sshd_config')
const checkOut = ref('')
/** 可视化模式校验通过后的预览全文（切到文件模式对照） */
const preview = ref('')

const valid = ref(true)

/** 文件里的指令取值：active = 生效值（sshd 取首次出现）；commented = 仅被注释的写法 */
function parseDirectives() {
  const active: Record<string, string> = {}
  const commented: Record<string, string> = {}
  for (const raw of content.value.split('\n')) {
    const line = raw.trim()
    if (!line) continue
    const isCommented = line.startsWith('#')
    const body = (isCommented ? line.slice(1).trim() : line).trim()
    if (!body) continue
    const sp = body.search(/[ \t]/)
    if (sp <= 0) continue
    const key = body.slice(0, sp)
    const val = body.slice(sp + 1).trim()
    if (!/^[A-Za-z][A-Za-z0-9]*$/.test(key)) continue
    if (isCommented) {
      if (!(key in commented)) commented[key] = val
    } else if (!(key in active)) {
      active[key] = val
    }
  }
  return { active, commented }
}

const current = computed(() => parseDirectives().active)
const commented = computed(() => parseDirectives().commented)

async function loadCfg() {
  loading.value = true
  try {
    const res = await http.get<{
      code: number
      data: { path: string; content: string; valid: boolean }
    }>('/system/config/ssh/config')
    path.value = res.data.path
    content.value = res.data.content
    valid.value = res.data.valid
    loaded.value = true
    // 可视化表单初始留空（= 不改动）,避免误以为当前值就是要写的值
    values.value = {}
  } finally {
    loading.value = false
  }
}

function onOpen() {
  checkOut.value = ''
  if (!loaded.value) loadCfg()
}

/** 非零Serialization：把用户填的项组装成 [[key, value]] */
function buildSet(): [string, string][] {
  return Object.entries(values.value)
    .filter(([, v]) => v !== undefined && v !== '')
    .map(([k, v]) => [k, v] as [string, string])
}

async function doValidate() {
  checking.value = true
  checkOut.value = ''
  try {
    // dry_run=true：只跑 sshd -t,不落盘
    if (mode.value === 'file') {
      const res = await http.post<{ code: number; message: string }>(
        '/system/config/ssh/config/save',
        { content: content.value, dry_run: true },
      )
      checkOut.value = res.message || t('serverSsh.validOk')
    } else {
      const res = await http.post<{ code: number; message: string; data?: { content?: string } }>(
        '/system/config/ssh/config/apply',
        { set: buildSet(), dry_run: true },
      )
      checkOut.value = res.message || t('serverSsh.validOk')
      // 把「保存后会长什么样」留一份,方便切到文件模式对照
      if (res.data?.content) preview.value = res.data.content
    }
    valid.value = true
  } catch (e: unknown) {
    valid.value = false
    checkOut.value = (e as { message?: string })?.message ?? t('serverSsh.validBad')
  } finally {
    checking.value = false
  }
}

async function doSave(restart: boolean) {
  saving.value = true
  checkOut.value = ''
  try {
    if (mode.value === 'file') {
      const res = await http.post<{ code: number; message: string; data?: { content?: string } }>(
        '/system/config/ssh/config/save',
        { content: content.value, restart },
      )
      ElMessage.success(restart ? t('serverSsh.configRestarted') : t('serverSsh.configSaved'))
      checkOut.value = res.message ?? ''
    } else {
      const res = await http.post<{ code: number; message: string; data?: { content?: string } }>(
        '/system/config/ssh/config/apply',
        { set: buildSet(), restart },
      )
      if (res.data?.content) content.value = res.data.content
      ElMessage.success(restart ? t('serverSsh.configRestarted') : t('serverSsh.configSaved'))
      values.value = {}
    }
    // 重新拉一次拿最新全文与状态
    await loadCfg()
    emit('saved')
  } catch (e: unknown) {
    const msg = (e as { message?: string })?.message ?? t('serverSsh.saveFailed')
    checkOut.value = msg
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.mode-bar {
  display: flex;
  align-items: center;
  gap: 10px;
}
.path {
  flex: 1;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mt-2 {
  margin-top: 8px;
}
.mono {
  font-family: 'JetBrains Mono', Consolas, Menlo, monospace;
  font-size: 12px;
}
.opt-form {
  max-height: 46vh;
  overflow: auto;
  padding-right: 6px;
}
.w160 {
  width: 160px;
}
.w220 {
  width: 220px;
}
.tip-icon {
  margin-left: 3px;
  color: var(--el-text-color-secondary);
}
.cur-value {
  margin-left: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.cur-value b {
  margin-left: 3px;
  color: var(--el-text-color-primary);
}
.commented-tag b {
  margin-left: 4px;
  font-weight: 500;
}
.unset {
  color: var(--el-text-color-disabled);
}
.check-out {
  max-height: 140px;
  overflow: auto;
  margin-top: 10px;
  padding: 8px 10px;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--el-fill-color-light);
  border-radius: 4px;
}
</style>
