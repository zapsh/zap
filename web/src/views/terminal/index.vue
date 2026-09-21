<template>
  <div class="terminal-page">
    <!-- 左侧连接管理器 -->
    <div class="terminal-sidebar" :style="{ width: sidebarWidth + 'px' }">
      <div class="sidebar-header">
        <span class="sidebar-title">{{ t('terminal.connManager') }}</span>
        <div class="sidebar-actions">
          <el-button
            link
            type="primary"
            size="small"
            :icon="Key"
            @click="openKeyManager"
          >
            {{ t('terminal.myKeys') }}
          </el-button>
          <el-button
            type="primary"
            size="small"
            :icon="Plus"
            :disabled="isReadOnly"
            @click="showAddDialog = true"
          >
            {{ t('common.add') }}
          </el-button>
        </div>
      </div>

      <div class="sidebar-search">
        <el-input
          v-model="connKeyword"
          :placeholder="t('terminal.searchPlaceholder')"
          size="small"
          clearable
          :prefix-icon="Search"
        />
      </div>

      <div class="connection-list">
        <div
          v-for="conn in filteredConnections"
          :key="conn.id"
          class="connection-item"
          :class="{ active: activeConnId === conn.id, disabled: conn.status === 0 }"
          @dblclick="openTerminal(conn)"
          @click="activeConnId = conn.id"
        >
          <div class="conn-avatar" :class="avatarClass(conn.id)">{{ avatarText(conn.name) }}</div>
          <div class="conn-info">
            <span class="conn-name" :title="conn.name">{{ conn.name }}</span>
            <span class="conn-host" :title="`${conn.username}@${conn.host}:${conn.port}`">
              <i class="status-dot" :class="conn.status === 0 ? 'off' : 'on'" />
              {{ conn.username }}@{{ conn.host }}:{{ conn.port }}
              <span
                v-if="conn.auth_type === 'password' && !conn.has_password"
                class="pwd-badge"
                :title="t('terminal.pwdBadgeTitle')"
              >
                {{ t('terminal.pwdBadge') }}
              </span>
            </span>
          </div>

          <!-- 行内操作：hover 才出现，避免常驻按钮挤压/遮挡连接信息 -->
          <div class="conn-actions" @click.stop>
            <el-button
              class="row-btn"
              :icon="Link"
              size="small"
              text
              :title="t('terminal.connect')"
              @click="openTerminal(conn)"
            />
            <el-dropdown trigger="click" @command="onRowCommand(conn, $event)">
              <el-button
                class="row-btn"
                :icon="MoreFilled"
                size="small"
                text
                :title="t('terminal.moreActions')"
              />
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item :icon="Edit" command="edit" :disabled="isReadOnly">
                    {{ t('common.edit') }}
                  </el-dropdown-item>
                  <el-dropdown-item
                    v-if="conn.auth_type === 'key'"
                    :icon="Key"
                    command="pushKey"
                    :disabled="isReadOnly || (isLoopbackHost(conn.host) && !isAdmin)"
                  >
                    {{
                      isLoopbackHost(conn.host) ? t('terminal.localPush') : t('terminal.pushToHost')
                    }}
                  </el-dropdown-item>
                  <el-dropdown-item :icon="Monitor" command="test">{{
                    t('terminal.testConn')
                  }}</el-dropdown-item>
                  <el-dropdown-item :icon="Delete" command="delete" divided :disabled="isReadOnly">
                    {{ t('common.delete') }}
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>
        </div>

        <el-empty
          v-if="connections.length === 0"
          :description="t('terminal.emptyConn')"
          :image-size="60"
        />
        <div v-else-if="filteredConnections.length === 0" class="search-empty">
          <el-icon><Search /></el-icon>
          <span>{{ t('terminal.noMatch') }}</span>
        </div>
      </div>

      <!-- 拖拽调宽手柄 -->
      <div
        class="sidebar-resizer"
        :title="t('terminal.resizeTip')"
        @mousedown.prevent="startSidebarResize"
      />
    </div>

    <!-- 右侧终端区域 -->
    <div class="terminal-main">
      <!-- 标签栏 -->
      <div class="tabs-bar" v-if="tabs.length > 0">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          class="tab-item"
          :class="{ active: activeTabId === tab.id }"
          :title="
            tab.name +
            (tab.status === 'connecting'
              ? t('terminal.tabConnecting')
              : tab.status === 'disconnected'
                ? t('terminal.tabDisconnected')
                : '')
          "
          @click="switchTab(tab.id)"
          @auxclick="onTabAuxClick($event, tab.id)"
        >
          <i class="tab-dot" :class="tab.status"></i>
          <span class="tab-label">{{ tab.name }}</span>
          <span class="tab-close" @click.stop="closeTab(tab.id)">
            <el-icon :size="12"><Close /></el-icon>
          </span>
        </div>
      </div>

      <!-- 终端容器 -->
      <div class="terminal-container" ref="containerRef">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          :ref="(el) => setTerminalRef(tab.id, el)"
          class="terminal-instance"
          :class="{ hidden: activeTabId !== tab.id }"
          @mousedown="focusTab(tab)"
        ></div>

        <div v-if="tabs.length === 0" class="terminal-placeholder">
          <el-icon :size="48" style="color: var(--el-text-color-secondary)"><Monitor /></el-icon>
          <p>{{ t('terminal.placeholder') }}</p>
        </div>
      </div>
    </div>

    <!-- 添加/编辑连接对话框 -->
    <el-dialog
      v-model="showAddDialog"
      :title="editingConn ? t('terminal.editConn') : t('terminal.addConn')"
      width="500px"
      :close-on-click-modal="false"
    >
      <el-form :model="form" label-width="90px" ref="formRef" @submit.prevent>
        <el-form-item :label="t('terminal.formName')" required>
          <el-input v-model="form.name" :placeholder="t('terminal.namePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('terminal.formHost')" required>
          <el-input v-model="form.host" :placeholder="t('terminal.hostPlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('terminal.formPort')">
          <el-input-number v-model="form.port" :min="1" :max="65535" />
        </el-form-item>
        <el-form-item :label="t('terminal.formUsername')">
          <el-input v-model="form.username" placeholder="root" />
        </el-form-item>
        <el-form-item :label="t('terminal.formAuth')">
          <el-radio-group v-model="form.auth_type">
            <el-radio value="password">{{ t('terminal.authPassword') }}</el-radio>
            <el-radio value="key">{{ t('terminal.authKey') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item v-if="form.auth_type === 'password'" :label="t('terminal.formPassword')">
          <el-input
            v-model="form.password"
            type="password"
            show-password
            :placeholder="t('terminal.passwordPlaceholder')"
          />
        </el-form-item>
        <el-form-item v-if="form.auth_type === 'key'" :label="t('terminal.formKey')">
          <div class="key-select-wrap">
            <el-select v-model="form.ssh_key_name" :placeholder="t('terminal.selectKey')" clearable>
              <el-option
                v-for="key in sshKeys"
                :key="key.name"
                :label="key.name"
                :value="key.name"
              />
            </el-select>
            <div class="key-tip">
              <span v-if="form.host && isLoopbackHost(form.host)">
                {{ t('terminal.keyTipLocal') }}
              </span>
              <span v-else-if="form.ssh_key_name">{{ t('terminal.keyTipSaved') }}</span>
              <span v-else>{{ t('terminal.keyTipEmpty') }}</span>
              <el-button
                type="primary"
                link
                size="small"
                :icon="Key"
                :disabled="isReadOnly"
                @click="openKeyManager"
              >
                {{ t('terminal.manageKeys') }}
              </el-button>
              <el-button
                v-if="form.ssh_key_name"
                type="primary"
                link
                size="small"
                :disabled="isReadOnly || (form.host && isLoopbackHost(form.host) && !isAdmin)"
                @click="openPushKeyFromForm"
              >
                {{
                  form.host && isLoopbackHost(form.host)
                    ? t('terminal.localPush')
                    : t('terminal.pushToRemote')
                }}
              </el-button>
            </div>
          </div>
        </el-form-item>
        <el-form-item :label="t('terminal.formRemark')">
          <el-input
            v-model="form.remark"
            type="textarea"
            :rows="2"
            :placeholder="t('terminal.remarkPlaceholder')"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showAddDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button
          v-if="editingConn"
          type="success"
          @click="handleTest(editingConn.id)"
          :loading="testing"
        >
          {{ t('terminal.testConn') }}
        </el-button>
        <el-button type="primary" @click="handleSave" :loading="saving">
          {{ editingConn ? t('common.save') : t('common.add') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 推送公钥到主机对话框 -->
    <el-dialog
      v-model="showPushKeyDialog"
      :title="pushKeyIsLocal ? t('terminal.localPush') : t('terminal.pushToRemote')"
      width="460px"
      :close-on-click-modal="false"
    >
      <el-alert v-if="!pushKeyIsLocal" type="info" :closable="false" show-icon>
        {{ t('terminal.pushRemoteAlert1') }}
        <b style="margin: 0 4px">{{ pushKeyTarget }}</b>
        {{ t('terminal.pushRemoteAlert2') }}
      </el-alert>
      <el-alert v-else :type="canLocalPush ? 'warning' : 'error'" :closable="false" show-icon>
        {{ t('terminal.pushLocalAlert1') }}
        <b style="margin: 0 4px">{{ pushKeyTarget }}</b>
        {{ t('terminal.pushLocalAlert2') }}
      </el-alert>
      <el-form v-if="!pushKeyIsLocal" label-width="90px" style="margin-top: 16px" @submit.prevent>
        <el-form-item :label="t('terminal.sshPassword')" required>
          <el-input
            v-model="pushKeyPwd"
            type="password"
            show-password
            :placeholder="t('terminal.remotePwdPlaceholder')"
            @keyup.enter="confirmPushKey"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showPushKeyDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button
          type="primary"
          :loading="pushing"
          :disabled="pushKeyIsLocal && !canLocalPush"
          @click="confirmPushKey"
        >
          {{ pushKeyIsLocal ? t('terminal.writeLocal') : t('terminal.push') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 我的 SSH 密钥管理 -->
    <el-dialog
      v-model="showKeyManager"
      :title="t('terminal.keyManagerTitle')"
      width="780px"
      @open="loadMyKeys"
    >
      <el-alert type="info" :closable="false" show-icon style="margin-bottom: 12px">
        {{ t('terminal.keyAlert') }}
      </el-alert>
      <div class="keymgr-toolbar">
        <el-button
          type="primary"
          size="small"
          :icon="Plus"
          :disabled="isReadOnly"
          @click="openKeyGen"
        >
          {{ t('terminal.genKey') }}
        </el-button>
        <el-button size="small" :icon="Key" :disabled="isReadOnly" @click="openKeyImport">
          {{ t('terminal.importKey') }}
        </el-button>
        <div style="flex: 1"></div>
        <el-button size="small" text :loading="keyLoading" @click="loadMyKeys">{{
          t('common.refresh')
        }}</el-button>
      </div>
      <el-table
        :data="myKeys"
        v-loading="keyLoading"
        size="small"
        max-height="400"
        :empty-text="t('terminal.noKeys')"
      >
        <el-table-column prop="name" :label="t('common.name')" min-width="130" />
        <el-table-column
          prop="comment"
          :label="t('terminal.comment')"
          min-width="120"
          show-overflow-tooltip
        />
        <el-table-column :label="t('terminal.fingerprint')" min-width="210" show-overflow-tooltip>
          <template #default="{ row }">
            <code class="fp">{{ row.fingerprint }}</code>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="220" align="right">
          <template #default="{ row }">
            <el-button link type="primary" size="small" @click="copyPub(row)">{{
              t('terminal.copyPub')
            }}</el-button>
            <el-button link type="primary" size="small" @click="viewPrivate(row)">
              {{ t('terminal.viewPrivate') }}
            </el-button>
            <el-button
              link
              type="danger"
              size="small"
              :disabled="isReadOnly"
              @click="removeKey(row)"
            >
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <template #footer>
        <el-button type="primary" @click="showKeyManager = false">{{
          t('common.close')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 生成密钥 -->
    <el-dialog
      v-model="showKeyGen"
      :title="t('terminal.genKeyTitle')"
      width="480px"
      :close-on-click-modal="false"
    >
      <el-form label-width="90px" @submit.prevent>
        <el-form-item :label="t('terminal.keyName')" required>
          <el-input v-model="keyGenForm.name" :placeholder="t('terminal.keyNamePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('terminal.keyType')" required>
          <el-radio-group v-model="keyGenForm.key_type">
            <el-radio value="ed25519">{{ t('terminal.typeEd25519') }}</el-radio>
            <el-radio value="rsa">RSA</el-radio>
            <el-radio value="ecdsa">ECDSA</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item v-if="keyGenForm.key_type === 'rsa'" :label="t('terminal.rsaBits')">
          <el-select v-model="keyGenForm.bits" style="width: 120px">
            <el-option :value="2048" label="2048" />
            <el-option :value="4096" label="4096" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('terminal.comment')">
          <el-input v-model="keyGenForm.comment" :placeholder="t('terminal.commentPlaceholder')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showKeyGen = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="keySaving" @click="submitKeyGen">{{
          t('terminal.genAndSave')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 导入密钥 -->
    <el-dialog
      v-model="showKeyImport"
      :title="t('terminal.importKeyTitle')"
      width="560px"
      :close-on-click-modal="false"
    >
      <el-form label-width="90px" @submit.prevent>
        <el-form-item :label="t('terminal.keyName')" required>
          <el-input v-model="keyImportForm.name" :placeholder="t('terminal.keyNamePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('terminal.privateKey')" required>
          <el-input
            v-model="keyImportForm.private_key"
            type="textarea"
            :rows="9"
            :placeholder="t('terminal.privateKeyPlaceholder')"
            style="font-family: monospace"
          />
        </el-form-item>
        <el-form-item :label="t('terminal.publicKey')">
          <el-input
            v-model="keyImportForm.public_key"
            type="textarea"
            :rows="3"
            :placeholder="t('terminal.publicKeyPlaceholder')"
            style="font-family: monospace"
          />
        </el-form-item>
        <el-form-item :label="t('terminal.comment')">
          <el-input v-model="keyImportForm.comment" :placeholder="t('common.optional')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showKeyImport = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="keySaving" @click="submitKeyImport">
          {{ t('terminal.importAndSave') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 查看私钥 -->
    <el-dialog v-model="showPrivateView" :title="t('terminal.viewPrivateTitle')" width="620px">
      <el-alert type="warning" :closable="false" show-icon style="margin-bottom: 10px">
        {{ t('terminal.privateAlert', { name: privateViewKey?.name }) }}
      </el-alert>
      <el-input
        :model-value="privateViewKey?.content ?? ''"
        type="textarea"
        :rows="12"
        readonly
        style="font-family: monospace"
      />
      <template #footer>
        <el-button @click="showPrivateView = false">{{ t('common.close') }}</el-button>
        <el-button type="primary" @click="copyPrivate">{{ t('terminal.copyPrivate') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, onActivated, nextTick, computed } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Edit, Delete, Link, Monitor, Key, Search, MoreFilled, Close } from '@/icons'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import '@xterm/xterm/css/xterm.css'
import {
  getConnections,
  createConnection,
  updateConnection,
  deleteConnection,
  testConnection,
  pushKeyToHost,
  pushKeyDirect,
  getUserSshKeys,
  generateUserKey,
  importUserKey,
  deleteUserKey,
  getUserKeyPublic,
  getUserKeyPrivate,
  type SshConnection,
  type UserSshKey,
} from '@/api/terminal'
import { getToken } from '@/utils/auth'
import { wsUrl } from '@/utils/base'
import { useUserStore } from '@/stores/user'
import { useI18n } from 'vue-i18n'

// ── 状态 ───────────────────────────────────────────────────

const { t } = useI18n()
const userStore = useUserStore()
const isReadOnly = computed(() => userStore.roles.includes('demo'))
const isAdmin = computed(() => userStore.roles.includes('admin'))

const connections = ref<SshConnection[]>([])
const sshKeys = ref<{ name: string; scope: 'user' }[]>([])
const activeConnId = ref<number | null>(null)

// 搜索过滤
const connKeyword = ref('')
const filteredConnections = computed(() => {
  const kw = connKeyword.value.trim().toLowerCase()
  if (!kw) return connections.value
  return connections.value.filter((c) =>
    [c.name, c.host, c.username, `${c.host}:${c.port}`].join(' ').toLowerCase().includes(kw),
  )
})

// 侧栏拖拽宽度（px）
const sidebarWidth = ref(300)
let dragStart: { x: number; w: number } | null = null
function startSidebarResize(e: MouseEvent) {
  dragStart = { x: e.clientX, w: sidebarWidth.value }
  window.addEventListener('mousemove', onSidebarResize)
  window.addEventListener('mouseup', endSidebarResize)
}
function onSidebarResize(e: MouseEvent) {
  if (!dragStart) return
  const w = dragStart.w + (e.clientX - dragStart.x)
  sidebarWidth.value = Math.min(520, Math.max(230, w))
}
function endSidebarResize() {
  dragStart = null
  window.removeEventListener('mousemove', onSidebarResize)
  window.removeEventListener('mouseup', endSidebarResize)
}

// 连接项头像（首字母 + 按 id 分配渐变色）
const AVATAR_COLORS = 6
function avatarText(name: string) {
  return (name.trim()[0] || '?').toUpperCase()
}
function avatarClass(id: number) {
  return `ac-${Math.abs(id) % AVATAR_COLORS}`
}

const showAddDialog = ref(false)
const editingConn = ref<SshConnection | null>(null)
const saving = ref(false)
const testing = ref(false)

const showPushKeyDialog = ref(false)
const pushKeyConnId = ref<number | null>(null)
/** 表单直推来源快照（pushKeyConnId 为 null 时使用），来自添加/编辑对话框当前表单 */
const pushKeyForm = ref<{ host: string; port: number; username: string; key: string } | null>(null)
const pushKeyTarget = ref('')
const pushKeyIsLocal = ref(false)
const pushKeyPwd = ref('')
const pushing = ref(false)

const canLocalPush = computed(() => pushKeyIsLocal.value && isAdmin.value && !isReadOnly.value)

function isLoopbackHost(host: string): boolean {
  const h = host
    .trim()
    .replace(/^\[|\]$/g, '')
    .toLowerCase()
  return h === 'localhost' || h === '127.0.0.1' || h === '::1'
}

/** 行菜单「推送公钥」：基于已保存的连接 */
function openPushKey(connId?: number | null) {
  if (connId == null) return
  const conn = connections.value.find((c) => c.id === connId)
  if (!conn) return
  pushKeyConnId.value = conn.id
  pushKeyForm.value = null
  pushKeyTarget.value = `${conn.username}@${conn.host}:${conn.port}`
  pushKeyIsLocal.value = isLoopbackHost(conn.host)
  pushKeyPwd.value = ''
  showPushKeyDialog.value = true
}

/** 添加/编辑对话框内「推送公钥」：基于表单参数直推，连接无需先保存 */
function openPushKeyFromForm() {
  const f = form.value
  if (!f.host.trim()) {
    ElMessage.warning(t('terminal.hostRequired'))
    return
  }
  if (!f.ssh_key_name) {
    ElMessage.warning(t('terminal.keyRequired'))
    return
  }
  pushKeyConnId.value = null
  pushKeyForm.value = {
    host: f.host.trim(),
    port: f.port,
    username: f.username.trim() || 'root',
    key: f.ssh_key_name,
  }
  pushKeyTarget.value = `${f.username.trim() || 'root'}@${f.host.trim()}:${f.port}`
  pushKeyIsLocal.value = isLoopbackHost(f.host)
  pushKeyPwd.value = ''
  showPushKeyDialog.value = true
}

async function confirmPushKey() {
  if (pushKeyConnId.value == null && !pushKeyForm.value) return
  if (!pushKeyIsLocal.value && !pushKeyPwd.value) {
    ElMessage.warning(t('terminal.remotePwdRequired'))
    return
  }
  if (pushKeyIsLocal.value && !canLocalPush.value) {
    ElMessage.warning(t('terminal.localPushAdminOnly'))
    return
  }
  pushing.value = true
  try {
    if (pushKeyConnId.value != null) {
      await pushKeyToHost(pushKeyConnId.value, pushKeyIsLocal.value ? '' : pushKeyPwd.value)
    } else {
      const pf = pushKeyForm.value!
      await pushKeyDirect({
        host: pf.host,
        port: pf.port,
        username: pf.username,
        ssh_key_name: pf.key,
        password: pushKeyIsLocal.value ? '' : pushKeyPwd.value,
      })
    }
    ElMessage.success(pushKeyIsLocal.value ? t('terminal.pushedLocal') : t('terminal.pushedRemote'))
    showPushKeyDialog.value = false
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.pushFailed'))
  } finally {
    pushing.value = false
  }
}

const form = ref({
  name: '',
  host: '',
  port: 22,
  username: 'root',
  auth_type: 'password' as 'password' | 'key',
  password: '',
  ssh_key_name: '',
  remark: '',
})

// ── 标签管理 ───────────────────────────────────────────────

interface TerminalTab {
  id: string
  name: string
  connId: number
  term: Terminal
  fitAddon: FitAddon
  ws: WebSocket | null
  /** connecting=正在连接 connected=已连接 disconnected=已断开（可重新连接） */
  status: 'connecting' | 'connected' | 'disconnected'
}

const tabs = ref<TerminalTab[]>([])
const activeTabId = ref<string | null>(null)
const containerRef = ref<HTMLElement | null>(null)
const terminalRefs: Record<string, HTMLElement> = {}

function setTerminalRef(tabId: string, el: any) {
  if (el) {
    terminalRefs[tabId] = el as HTMLElement
  }
}

// ── 加载连接列表 ───────────────────────────────────────────

async function loadConnections() {
  try {
    const resp = await getConnections()
    connections.value = resp.data || []
  } catch {
    ElMessage.error(t('terminal.loadConnFailed'))
  }
}

async function loadSshKeys() {
  try {
    const resp = await getUserSshKeys()
    const d = resp.data
    sshKeys.value = d?.items || []
  } catch {
    // SSH keys may not be available
  }
}

// ── 我的 SSH 密钥管理 ─────────────────────────────────────────

const showKeyManager = ref(false)
const myKeys = ref<UserSshKey[]>([])
const keyLoading = ref(false)
const keySaving = ref(false)

function openKeyManager() {
  showKeyManager.value = true
  loadMyKeys()
}

async function loadMyKeys() {
  keyLoading.value = true
  try {
    const resp = await getUserSshKeys()
    const d = resp.data
    myKeys.value = d?.items || []
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.loadKeysFailed'))
  } finally {
    keyLoading.value = false
  }
}

const KEY_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/

async function copyPub(key: UserSshKey) {
  try {
    const resp = await getUserKeyPublic(key.name)
    const pub = resp.data?.public_key
    if (!pub) throw new Error(t('terminal.pubEmpty'))
    await navigator.clipboard.writeText(pub)
    ElMessage.success(t('terminal.pubCopied'))
  } catch (e: any) {
    ElMessage.error(e.message || t('common.copyFailed'))
  }
}

const showPrivateView = ref(false)
const privateViewKey = ref<{ name: string; content: string } | null>(null)

async function viewPrivate(key: UserSshKey) {
  try {
    const resp = await getUserKeyPrivate(key.name)
    const content = resp.data?.private_key
    if (!content) throw new Error(t('terminal.privEmpty'))
    privateViewKey.value = { name: key.name, content }
    showPrivateView.value = true
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.readPrivateFailed'))
  }
}

async function copyPrivate() {
  const k = privateViewKey.value
  if (!k) return
  try {
    await navigator.clipboard.writeText(k.content)
    ElMessage.success(t('terminal.privCopied'))
  } catch {
    ElMessage.error(t('common.copyFailed'))
  }
}

async function removeKey(key: UserSshKey) {
  try {
    await ElMessageBox.confirm(
      t('terminal.delKeyConfirm', { name: key.name }),
      t('terminal.delKeyTitle'),
      {
        type: 'warning',
        confirmButtonText: t('common.delete'),
        cancelButtonText: t('common.cancel'),
      },
    )
  } catch {
    return
  }
  try {
    await deleteUserKey(key.name)
    ElMessage.success(t('terminal.deleted'))
    await Promise.all([loadMyKeys(), loadSshKeys()])
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.deleteFailed'))
  }
}

// 生成
const showKeyGen = ref(false)
const keyGenForm = ref({ name: '', key_type: 'ed25519', bits: 4096, comment: '' })

function openKeyGen() {
  keyGenForm.value = { name: '', key_type: 'ed25519', bits: 4096, comment: '' }
  showKeyGen.value = true
}

async function submitKeyGen() {
  const f = keyGenForm.value
  const name = f.name.trim()
  if (!KEY_NAME_RE.test(name)) {
    ElMessage.warning(t('terminal.keyNameRule'))
    return
  }
  keySaving.value = true
  try {
    await generateUserKey({
      name,
      key_type: f.key_type,
      bits: f.key_type === 'rsa' ? f.bits : undefined,
      comment: f.comment.trim() || undefined,
    })
    ElMessage.success(t('terminal.keyGenerated'))
    showKeyGen.value = false
    await Promise.all([loadMyKeys(), loadSshKeys()])
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.genFailed'))
  } finally {
    keySaving.value = false
  }
}

// 导入
const showKeyImport = ref(false)
const keyImportForm = ref({ name: '', private_key: '', public_key: '', comment: '' })

function openKeyImport() {
  keyImportForm.value = { name: '', private_key: '', public_key: '', comment: '' }
  showKeyImport.value = true
}

async function submitKeyImport() {
  const f = keyImportForm.value
  const name = f.name.trim()
  if (!KEY_NAME_RE.test(name)) {
    ElMessage.warning(t('terminal.keyNameRule'))
    return
  }
  if (!f.private_key.trim()) {
    ElMessage.warning(t('terminal.privateKeyRequired'))
    return
  }
  keySaving.value = true
  try {
    await importUserKey({
      name,
      private_key: f.private_key,
      public_key: f.public_key.trim() || undefined,
      comment: f.comment.trim() || undefined,
    })
    ElMessage.success(t('terminal.keyImported'))
    showKeyImport.value = false
    await Promise.all([loadMyKeys(), loadSshKeys()])
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.importFailed'))
  } finally {
    keySaving.value = false
  }
}

// ── 连接 CRUD ──────────────────────────────────────────────

function resetForm() {
  form.value = {
    name: '',
    host: '',
    port: 22,
    username: 'root',
    auth_type: 'password',
    password: '',
    ssh_key_name: '',
    remark: '',
  }
  editingConn.value = null
}

function editConnection(conn: SshConnection) {
  form.value = {
    name: conn.name,
    host: conn.host,
    port: conn.port,
    username: conn.username,
    auth_type: conn.auth_type,
    password: '',
    ssh_key_name: conn.ssh_key_name,
    remark: conn.remark,
  }
  editingConn.value = conn
  showAddDialog.value = true
}

async function handleSave() {
  const f = form.value
  if (!f.name.trim()) {
    ElMessage.warning(t('terminal.nameRequired'))
    return
  }
  if (!f.host.trim()) {
    ElMessage.warning(t('terminal.hostRequired'))
    return
  }

  saving.value = true
  try {
    if (editingConn.value) {
      await updateConnection(editingConn.value.id, {
        name: f.name,
        host: f.host,
        port: f.port,
        username: f.username,
        auth_type: f.auth_type,
        password: f.password || undefined,
        ssh_key_name: f.ssh_key_name,
        remark: f.remark,
      })
      ElMessage.success(t('common.updateSuccess'))
    } else {
      await createConnection({
        name: f.name,
        host: f.host,
        port: f.port,
        username: f.username,
        auth_type: f.auth_type,
        password: f.password,
        ssh_key_name: f.ssh_key_name,
        remark: f.remark,
      })
      ElMessage.success(t('common.createSuccess'))
    }
    showAddDialog.value = false
    resetForm()
    await loadConnections()
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.opFailed'))
  } finally {
    saving.value = false
  }
}

async function handleDelete(id: number) {
  try {
    await deleteConnection(id)
    ElMessage.success(t('common.deleteSuccess'))
    // Close any open tabs for this connection
    tabs.value = tabs.value.filter((t) => t.connId !== id)
    if (activeTabId.value && !tabs.value.find((t) => t.id === activeTabId.value)) {
      activeTabId.value = tabs.value.length > 0 ? tabs.value[0].id : null
    }
    await loadConnections()
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.deleteFailed'))
  }
}

async function handleTest(id: number) {
  testing.value = true
  try {
    const resp = await testConnection(id)
    if (resp.data?.success) {
      ElMessage.success(t('terminal.connOk'))
    } else {
      ElMessage.error(resp.data?.message || t('terminal.connFailed'))
    }
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.testFailed'))
  } finally {
    testing.value = false
  }
}

// 连接项「更多」下拉命令分发
function onRowCommand(conn: SshConnection, cmd: string | number | object) {
  handleRowAction(conn, String(cmd))
}

function handleRowAction(conn: SshConnection, cmd: string) {
  switch (cmd) {
    case 'edit':
      editConnection(conn)
      break
    case 'pushKey':
      openPushKey(conn.id)
      break
    case 'test':
      handleTest(conn.id)
      break
    case 'delete':
      ElMessageBox.confirm(t('terminal.delConnConfirm'), t('common.tip'), {
        type: 'warning',
        confirmButtonText: t('common.delete'),
        cancelButtonText: t('common.cancel'),
      })
        .then(() => handleDelete(conn.id))
        .catch(() => {})
      break
  }
}

// ── 终端管理 ───────────────────────────────────────────────

// 把当前 pty 窗口尺寸通过 WebSocket 控制消息同步给后端
function syncResize(tab: TerminalTab) {
  const term = tab.term
  if (!term || !tab.ws || tab.ws.readyState !== WebSocket.OPEN) return
  const { cols, rows } = term
  if (!cols || !rows) return
  tab.ws.send(JSON.stringify({ type: 'resize', cols, rows }))
}

// 自动适配容器尺寸：等字体加载与 DOM 布局稳定后再 fit，并同步 pty 大小
function fitTerminal(tab: TerminalTab) {
  if (!tab.fitAddon) return
  const doFit = () => {
    tab.fitAddon?.fit()
    syncResize(tab)
  }
  requestAnimationFrame(doFit)
  // xterm 依赖等宽字体度量，字体未就绪时 fit 出的列/行数会偏小
  if (document.fonts?.ready) {
    document.fonts.ready.then(() => {
      if (tab.term) requestAnimationFrame(doFit)
    })
  }
}

function getWsUrl(connId: number): string {
  const token = getToken()
  return wsUrl(`/api/terminal/ws/${connId}?token=${token}&rows=24&cols=80`)
}

async function openTerminal(conn: SshConnection) {
  if (isReadOnly.value) {
    ElMessage.warning(t('terminal.demoTip'))
    return
  }
  // 已存在该连接的标签页
  const existing = tabs.value.find((t) => t.connId === conn.id)
  if (existing) {
    // 会话仍存活 → 直接切换过去
    if (existing.ws && existing.ws.readyState === WebSocket.OPEN) {
      activeTabId.value = existing.id
      existing.term.focus()
      return
    }
    // 正在建立连接中 → 防止重复点击重复建会话
    if (existing.ws && existing.ws.readyState === WebSocket.CONNECTING) {
      activeTabId.value = existing.id
      return
    }
    // 会话已断开（如 shell 内 exit）→ 复用标签页重新连接。
    // 密码认证且未保存密码 → 先弹窗输入本次会话临时密码（不落库）
    const tempPwd = await askPasswordIfNeeded(conn)
    if (needTempPassword(conn) && !tempPwd) return // 用户取消输入
    activeTabId.value = existing.id
    await nextTick()
    existing.term.clear()
    existing.term.writeln(
      '\r\n\x1b[36m' + t('terminal.reconnecting', { name: conn.name }) + '\x1b[0m',
    )
    connectTab(existing, conn, tempPwd)
    return
  }

  // 新连接：密码认证且未保存密码 → 先弹窗输入本次会话临时密码（不落库）
  const tempPwd = await askPasswordIfNeeded(conn)
  if (needTempPassword(conn) && !tempPwd) return // 用户取消输入

  const tabId = `term-${conn.id}-${Date.now()}`
  activeTabId.value = tabId

  // Create tab entry (terminal will be initialized after DOM update)
  const tab: TerminalTab = {
    id: tabId,
    name: conn.name,
    connId: conn.id,
    term: null as any,
    fitAddon: null as any,
    ws: null,
    status: 'connecting',
  }
  tabs.value.push(tab)

  await nextTick()

  const el = terminalRefs[tabId]
  if (!el) return

  // Initialize xterm
  const term = new Terminal({
    cursorBlink: true,
    cursorStyle: 'bar',
    fontSize: 14,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: {
      background: '#1e1e1e',
      foreground: '#d4d4d4',
      cursor: '#ffffff',
      selectionBackground: '#264f78',
    },
    allowProposedApi: true,
  })

  const fitAddon = new FitAddon()
  term.loadAddon(fitAddon)
  term.loadAddon(new WebLinksAddon())

  term.open(el)

  tab.term = term
  tab.fitAddon = fitAddon

  // 初始 fit 放在字体/布局就绪后，避免列数偏小导致终端窗口不撑满
  fitTerminal(tab)

  // Focus the terminal
  term.focus()

  // 终端输入 → WebSocket（只注册一次；发送时读取当前 ws，重连后依然生效）
  term.onData((data) => {
    const w = tab.ws
    if (w && w.readyState === WebSocket.OPEN) {
      ;(tab as any).inputWarned = false
      w.send(data)
      return
    }
    // 会话已断开时不要静默丢弃按键，给出一次性提示
    if (!(tab as any).inputWarned) {
      ;(tab as any).inputWarned = true
      term.writeln('\r\n\x1b[33m' + t('terminal.inputDropped') + '\x1b[0m')
    }
  })

  // 建立 WebSocket 会话
  connectTab(tab, conn, tempPwd)

  // 容器尺寸变化时重新 fit 并同步 pty 尺寸（观察父容器，而非单个实例）
  const resizeObserver = new ResizeObserver(() => {
    fitTerminal(tab)
  })
  if (containerRef.value) resizeObserver.observe(containerRef.value)
  ;(tab as any)._resizeObserver = resizeObserver
}

/** 密码认证且未保存密码的连接，需要弹窗临时输入（仅本次会话，不落库） */
function needTempPassword(conn: SshConnection): boolean {
  return conn.auth_type === 'password' && !conn.has_password
}

/** 需要临时密码时弹窗输入；无需输入或用户取消时返回 null */
async function askPasswordIfNeeded(conn: SshConnection): Promise<string | null> {
  if (!needTempPassword(conn)) return null
  try {
    const { value } = await ElMessageBox.prompt(
      t('terminal.pwdPrompt', { user: conn.username, host: conn.host }),
      t('terminal.connectTitle', { name: conn.name }),
      {
        inputType: 'password',
        confirmButtonText: t('terminal.connect'),
        cancelButtonText: t('common.cancel'),
        inputValidator: (v: string) => (v.trim() ? true : t('terminal.pwdNotEmpty')),
      },
    )
    return value ?? null
  } catch {
    return null // 用户点击取消
  }
}

// 建立（或重连）某标签页的 WebSocket 会话；复用已有的 xterm 实例。
// authPassword：未保存密码的连接，连接后将其作为临时密码下发（不落库）
function connectTab(tab: TerminalTab, conn: SshConnection, authPassword?: string | null) {
  // 已连接 / 正在连接则不重复建立
  if (tab.ws && tab.ws.readyState !== WebSocket.CLOSED) return

  const term = tab.term
  tab.status = 'connecting'

  const wsUrl = getWsUrl(conn.id)
  const ws = new WebSocket(wsUrl)
  // Receive binary as ArrayBuffer (no async FileReader overhead)
  ws.binaryType = 'arraybuffer'
  tab.ws = ws

  ws.onopen = () => {
    if (tab.ws !== ws) return // 已关闭/被新会话替换
    tab.status = 'connected'
    if (authPassword) {
      // 后端凭据里无密码：把本次输入的临时密码下发给后端完成 SSH 认证
      term.writeln('\x1b[36m' + t('terminal.authenticating') + '\x1b[0m')
      ;(tab as any).authSent = true
      ws.send(JSON.stringify({ type: 'auth', password: authPassword }))
    } else {
      term.writeln(
        '\x1b[32m' +
          t('terminal.connectedTo', { name: conn.name, host: conn.host, port: conn.port }) +
          '\x1b[0m',
      )
    }
    // 连接建立后同步一次当前实际窗口尺寸
    fitTerminal(tab)
    term.focus()
  }

  ws.onmessage = (event) => {
    if (tab.ws !== ws) return
    if (event.data instanceof ArrayBuffer) {
      term.write(new Uint8Array(event.data))
    } else if (typeof event.data === 'string') {
      // 后端控制消息（如索要临时密码）不回显到终端
      let ctrl: any = null
      try {
        ctrl = JSON.parse(event.data)
      } catch {
        ctrl = null
      }
      if (ctrl?.type === 'ask_password') {
        // 连接时已下发过临时密码 → 这条是竞态产物，忽略
        if ((tab as any).authSent) return
        void ElMessageBox.prompt(
          t('terminal.pwdPrompt', { user: conn.username, host: conn.host }),
          t('terminal.connectTitle', { name: conn.name }),
          {
            inputType: 'password',
            confirmButtonText: t('terminal.connect'),
            cancelButtonText: t('common.cancel'),
            inputValidator: (v: string) => (v.trim() ? true : t('terminal.pwdNotEmpty')),
          },
        )
          .then(({ value }) => {
            if (tab.ws !== ws || ws.readyState !== WebSocket.OPEN) return
            ;(tab as any).authSent = true
            ws.send(JSON.stringify({ type: 'auth', password: value ?? '' }))
          })
          .catch(() => ws.close())
        return
      }
      term.write(event.data)
    }
  }

  ws.onerror = () => {
    if (tab.ws !== ws) return
    term.writeln('\r\n\x1b[31m' + t('terminal.connError') + '\x1b[0m')
  }

  ws.onclose = () => {
    if (tab.ws !== ws) return
    tab.status = 'disconnected'
    term.writeln('\r\n\x1b[33m' + t('terminal.connClosed') + '\x1b[0m')
  }
}

/** 点击终端区域兜底取回焦点（切换标签/重连后 xterm 可能失去焦点） */
function focusTab(tab: TerminalTab) {
  tab.term?.focus()
}

function switchTab(tabId: string) {
  activeTabId.value = tabId
  const tab = tabs.value.find((t) => t.id === tabId)
  if (tab?.term) {
    nextTick(() => {
      tab.term.focus()
      fitTerminal(tab)
    })
  }
}

function closeTab(tabId: string) {
  const idx = tabs.value.findIndex((t) => t.id === tabId)
  if (idx === -1) return

  const tab = tabs.value[idx]

  // Cleanup
  if (tab.ws) {
    // 解除事件回调，避免 dispose 后 onclose 再写入已销毁的 terminal
    tab.ws.onopen = null
    tab.ws.onmessage = null
    tab.ws.onerror = null
    tab.ws.onclose = null
    tab.ws.close()
  }
  if ((tab as any)._resizeObserver) {
    ;(tab as any)._resizeObserver.disconnect()
  }
  if (tab.term) {
    tab.term.dispose()
  }

  tabs.value.splice(idx, 1)

  if (activeTabId.value === tabId) {
    activeTabId.value = tabs.value.length > 0 ? tabIdx(idx) : null
  }
}

function tabIdx(idx: number): string | null {
  const newIdx = Math.min(idx, tabs.value.length - 1)
  return tabs.value[newIdx]?.id ?? null
}

// 中键点击标签关闭
function onTabAuxClick(e: MouseEvent, tabId: string) {
  if (e.button === 1) closeTab(tabId)
}

// ── 生命周期 ───────────────────────────────────────────────

onMounted(() => {
  loadConnections()
  loadSshKeys()
})

// 本页是常驻存活的（切页不卸载，见 stores/tags.ts）：从缓存里切回来时重新量一次容器尺寸，
// 期间浏览器窗口可能变过，xterm 还是老的列/行，不重排就会不占满或与 pty 尺寸对不上。
onActivated(() => {
  const tab = tabs.value.find((t) => t.id === activeTabId.value)
  if (tab?.term) nextTick(() => fitTerminal(tab))
})

onBeforeUnmount(() => {
  // Cleanup all terminals
  for (const tab of tabs.value) {
    if (tab.ws) {
      tab.ws.onopen = null
      tab.ws.onmessage = null
      tab.ws.onerror = null
      tab.ws.onclose = null
      tab.ws.close()
    }
    if ((tab as any)._resizeObserver) (tab as any)._resizeObserver.disconnect()
    if (tab.term) tab.term.dispose()
  }
  // 清理侧栏拖拽监听
  if (dragStart) {
    window.removeEventListener('mousemove', onSidebarResize)
    window.removeEventListener('mouseup', endSidebarResize)
  }
})
</script>

<script lang="ts">
// 组件名要和 keep-alive 白名单对得上（include 按组件 name 匹配，不是路由 name）；
// 终端是常驻存活的页面：切走不卸载，SSH 会话与滚动内容都留着，onBeforeUnmount 也不触发。
// 见 stores/tags.ts。
export default { name: 'Terminal' }
</script>

<style scoped>
.terminal-page {
  display: flex;
  height: calc(100vh - 84px - 20px);
  background: var(--el-bg-color);
  border-radius: 4px;
  overflow: hidden;
}

/* ── 左侧栏 ─────────────────────────────── */
/* 注：xterm 终端本身固定深色主题（#1e1e1e），与面板主题无关，不随暗色模式变化 */

.terminal-sidebar {
  position: relative;
  flex-shrink: 0;
  min-width: 230px;
  border-right: 1px solid var(--el-border-color-lighter);
  display: flex;
  flex-direction: column;
  background: var(--el-bg-color-page);
}

.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px 12px;
}

.sidebar-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.keymgr-toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-bottom: 12px;
}

.keymgr-toolbar .fp,
.code {
  font-family: var(--el-font-family-mono, Menlo, Monaco, Consolas, monospace);
}

.fp {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.dim {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.sidebar-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}

/* ── 搜索 ─────────────────────────────────── */

.sidebar-search {
  padding: 0 12px 10px;
}

.sidebar-search .el-input__wrapper {
  border-radius: 8px;
  box-shadow: 0 0 0 1px var(--el-border-color-light) inset;
}

.sidebar-search .el-input__wrapper.is-focus {
  box-shadow: 0 0 0 1px #409eff inset;
}

/* ── 连接列表 ─────────────────────────────── */

.connection-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 8px 8px;
}

.connection-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  margin-bottom: 4px;
  border-radius: 8px;
  cursor: pointer;
  transition:
    background 0.15s,
    border-color 0.15s;
  border: 1px solid transparent;
}

.connection-item:hover {
  background: var(--el-color-primary-light-9);
  border-color: var(--el-color-primary-light-8);
}

.connection-item.active {
  background: var(--el-color-primary-light-8);
  border-color: var(--el-color-primary);
}

.connection-item.disabled {
  opacity: 0.55;
}

.connection-item:hover .conn-actions,
.connection-item:focus-within .conn-actions {
  display: flex;
}

/* 头像：首字母 + 渐变底色 */
.conn-avatar {
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  font-weight: 600;
  color: #fff;
  user-select: none;
}

.conn-avatar.ac-0 {
  background: linear-gradient(135deg, #409eff, #2f7fe6);
}
.conn-avatar.ac-1 {
  background: linear-gradient(135deg, #7c5cf0, #5a3fd6);
}
.conn-avatar.ac-2 {
  background: linear-gradient(135deg, #13c2c2, #08979c);
}
.conn-avatar.ac-3 {
  background: linear-gradient(135deg, #fa8c16, #d46b08);
}
.conn-avatar.ac-4 {
  background: linear-gradient(135deg, #52c41a, #389e0d);
}
.conn-avatar.ac-5 {
  background: linear-gradient(135deg, #f759ab, #d63096);
}

.conn-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
  /* 预留右侧操作按钮空间，避免 hover 时遮挡/挤压信息 */
  padding-right: 72px;
}

.conn-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.conn-host {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--el-text-color-secondary);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, 'Courier New', monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 未保存密码的连接角标：双击连接时弹窗输入 */
.pwd-badge {
  flex-shrink: 0;
  padding: 0 5px;
  font-size: 10px;
  line-height: 15px;
  color: #b88230;
  background: rgba(224, 193, 141, 0.16);
  border: 1px solid rgba(184, 130, 48, 0.45);
  border-radius: 3px;
  font-family: inherit;
}

.status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.status-dot.on {
  background: #67c23a;
  box-shadow: 0 0 0 2px rgba(103, 194, 58, 0.2);
}

.status-dot.off {
  background: var(--el-text-color-placeholder);
}

/* 行内操作：hover 浮层，不占文档流空间 */
.conn-actions {
  display: none;
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  align-items: center;
  gap: 2px;
  padding: 2px;
  border-radius: 8px;
  background: var(--el-bg-color-overlay);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1);
  z-index: 2;
}

.row-btn {
  width: 26px;
  height: 26px;
  padding: 0;
}

.search-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 32px 0;
  color: var(--el-text-color-placeholder);
  font-size: 13px;
}

/* 侧栏拖拽调宽手柄 */
.sidebar-resizer {
  position: absolute;
  top: 0;
  right: -3px;
  width: 6px;
  height: 100%;
  cursor: col-resize;
  z-index: 5;
  transition: background 0.15s;
}

.sidebar-resizer:hover,
.sidebar-resizer:active {
  background: rgba(64, 158, 255, 0.45);
}

.key-select-wrap {
  width: 100%;
}

.key-tip {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.6;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

/* ── 右侧终端区 ─────────────────────────── */

.terminal-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.tabs-bar {
  display: flex;
  align-items: stretch;
  height: 38px;
  background: var(--el-fill-color-light);
  border-bottom: 1px solid var(--el-border-color-light);
  overflow-x: auto;
  overflow-y: hidden;
  flex-shrink: 0;
}

.tabs-bar::-webkit-scrollbar {
  height: 4px;
}

.tabs-bar::-webkit-scrollbar-thumb {
  background: var(--el-text-color-placeholder);
  border-radius: 2px;
}

.tab-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  padding: 0 10px 0 16px;
  font-size: 13px;
  color: var(--el-text-color-regular);
  cursor: pointer;
  white-space: nowrap;
  user-select: none;
  flex-shrink: 0;
  border-right: 1px solid var(--el-border-color-lighter);
  transition:
    background 0.12s,
    color 0.12s;
}

.tab-item:hover {
  background: var(--el-fill-color-light);
  color: var(--el-text-color-primary);
}

.tab-item.active {
  background: var(--el-bg-color);
  color: var(--el-text-color-primary);
  font-weight: 600;
}

/* 激活标签顶部色条（与整体蓝色主题一致） */
.tab-item.active::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 3px;
  background: #409eff;
}

/* 连接状态点：绿=已连接 黄脉冲=连接中 红=已断开 */
.tab-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tab-dot.connected {
  background: #52c41a;
}

.tab-dot.connecting {
  background: #e6a23c;
  animation: tab-dot-pulse 1s ease-in-out infinite;
}

.tab-dot.disconnected {
  background: #f56c6c;
}

@keyframes tab-dot-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.tab-label {
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 180px;
}

.tab-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 4px;
  color: var(--el-text-color-secondary);
  cursor: pointer;
  transition:
    background 0.12s,
    color 0.12s;
}

.tab-item:hover .tab-close {
  color: var(--el-text-color-regular);
}

.tab-close:hover {
  background: rgba(0, 0, 0, 0.08);
  color: var(--el-text-color-primary);
}

.terminal-container {
  flex: 1;
  position: relative;
  background: #1e1e1e;
}

.terminal-instance {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
}

.terminal-instance.hidden {
  display: none;
}

.terminal-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--el-text-color-secondary);
  gap: 16px;
}

.terminal-placeholder p {
  font-size: 14px;
  margin: 0;
}
</style>
