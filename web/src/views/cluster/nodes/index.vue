<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
/**
 * 集群 → 节点管理：登记受管节点（面板地址 + 该节点的凭据）。
 *
 * 凭据二选一：手工填节点上创建的 API Token，或直接填节点登录账号密码——
 * 保存时后端用账号密码登录节点、自动签发一个 Token（账号密码本身不落库）。
 * 后者覆盖两个场景：面板刚装好还没登录过、节点上的 Token 被删后重连换发。
 *
 * 节点可按分组管理、按归属隔离；支持搜索排序、批量启停 / 改组 / 下发脚本；
 * 凭据到期与离线 / 高负载告警在列表和告警记录里可见。
 */
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { formatBytes } from '@/utils/fmt'
import { nodeContext, nodeViewPath, setNodeContext } from '@/utils/base'
import {
  addNode,
  addNodeGroup,
  batchRunScript,
  batchUpdateNodes,
  deleteNodeGroup,
  deleteNodes,
  getAlertConfig,
  listAlertEvents,
  listNodeGroups,
  listNodeOwners,
  listNodes,
  refreshNodes,
  setAlertConfig,
  testNode,
  updateNode,
  updateNodeGroup,
  type AlertConfig,
  type NodeAlertEvent,
  type NodeGroupItem,
  type NodeItem,
  type NodeListParams,
  type NodePayload,
  type NodeSnapshot,
} from '@/api/node'

const { t } = useI18n()

const loading = ref(false)
const saving = ref(false)
const testing = ref(false)
const collecting = ref(false)
const rows = ref<NodeItem[]>([])
const dialog = ref(false)
const editId = ref<number | null>(null)
const testResult = ref<NodeSnapshot | null>(null)
const testError = ref('')

const filters = reactive({
  keyword: '',
  group_id: 0,
  /** -1 = 全部，0 = 停用，1 = 启用 */
  status: -1,
  /** -1 = 全部，0 = 离线，1 = 在线 */
  online: -1,
  sort: 'id' as NonNullable<NodeListParams['sort']>,
})
const groups = ref<NodeGroupItem[]>([])
const owners = ref<{ id: number; username: string }[]>([])
const selected = ref<NodeItem[]>([])
const selectedIds = computed(() => selected.value.map((r) => r.id))

const form = reactive({
  name: '',
  base_url: '',
  /** 接入方式：手工填 API Token，或填账号密码由主控在节点上自动签发 */
  auth_mode: 'token' as 'token' | 'password',
  api_token: '',
  username: '',
  password: '',
  totp_code: '',
  remark: '',
  insecure_tls: true,
  group_id: 0,
  owner_id: 0,
  cred_expire_days: 0,
})
/** 账号密码模式的测试结论（不签发 Token，只验证能否登录，没有快照可展示） */
const testNote = ref('')

const groupDialog = ref(false)
const groupForm = reactive({ id: 0, name: '', sort_order: 0 })
const alertDialog = ref(false)
const alertForm = reactive<AlertConfig>({
  cpu: 85,
  memory: 90,
  disk: 90,
  offline_seconds: 300,
  cooldown_seconds: 1800,
  email: false,
})
const recordsDrawer = ref(false)
const alertRecords = ref<NodeAlertEvent[]>([])
const batchGroupDialog = ref(false)
const batchGroupId = ref(0)
const scriptDialog = ref(false)
const scriptPath = ref('')

const dialogTitle = computed(() =>
  editId.value === null ? t('cluster.addNode') : t('cluster.editNode'),
)
/** 编辑态 Token 留空 = 不改，占位文案必须说清楚，否则用户以为被清空了 */
const tokenPlaceholder = computed(() =>
  editId.value === null ? t('cluster.tokenPlaceholderNew') : t('cluster.tokenPlaceholderEdit'),
)
const onlineCount = computed(() => rows.value.filter((r) => r.online).length)
const router = useRouter()

/**
 * 接管这台机器：设定节点上下文后**整页重载**。
 *
 * 重载是必须的——接管后 `/user/info` 与 `/system/menus` 都改由节点回答，
 * 身份和菜单树只在应用启动时拉一次，SPA 内跳路由拿不到远端结果。
 */
function takeover(r: NodeItem) {
  setNodeContext({ id: r.id, name: r.name, baseUrl: r.base_url })
  window.location.href = nodeViewPath(router, '/dashboard')
}

function ago(ts: number): string {
  return ts ? new Date(ts * 1000).toLocaleString() : '-'
}

function sizeText(bytes: number): string {
  return bytes ? formatBytes(bytes) : '-'
}

async function load() {
  loading.value = true
  const params: NodeListParams = { sort: filters.sort }
  if (filters.keyword.trim()) params.keyword = filters.keyword.trim()
  if (filters.group_id) params.group_id = filters.group_id
  if (filters.status >= 0) params.status = filters.status
  if (filters.online >= 0) params.online = filters.online
  try {
    const res = await listNodes(params)
    rows.value = res.data || []
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    loading.value = false
  }
}

async function loadMeta() {
  try {
    groups.value = (await listNodeGroups()).data || []
  } catch {
    /* 拦截器已弹窗 */
  }
  try {
    owners.value = (await listNodeOwners()).data || []
  } catch {
    /* 无授权时不影响其他功能 */
  }
}

function credLabel(status: string): string {
  const key = `cred${status.charAt(0).toUpperCase()}${status.slice(1)}`
  return t(`cluster.${key}`)
}

function resetForm(r?: NodeItem) {
  Object.assign(form, {
    name: r?.name ?? '',
    base_url: r?.base_url ?? '',
    auth_mode: 'token' as const,
    // Token 与密码永不回填：留空即不修改 / 不引导
    api_token: '',
    username: '',
    password: '',
    totp_code: '',
    remark: r?.remark ?? '',
    insecure_tls: r?.insecure_tls ?? true,
    group_id: r?.group_id ?? 0,
    owner_id: r?.owner_id ?? 0,
    cred_expire_days: 0,
  })
  testResult.value = null
  testError.value = ''
  testNote.value = ''
}

function openAdd() {
  editId.value = null
  resetForm()
  dialog.value = true
}

function openEdit(r: NodeItem, rotate = false) {
  editId.value = r.id
  resetForm(r)
  // 「轮换凭据」直接落到账号密码页签，避免用户误以为手工 Token 已存在
  if (rotate) form.auth_mode = 'password'
  dialog.value = true
}

/** 只测连通、不保存：失败也把原因显示出来，方便当场排查（Token 错 / 证书 / 端口不通） */
async function runTest() {
  testResult.value = null
  testError.value = ''
  testNote.value = ''
  if (!form.base_url.trim()) {
    testError.value = t('cluster.needUrl')
    return
  }
  if (form.auth_mode === 'password') {
    if (!form.username.trim() || !form.password) {
      testError.value = t('cluster.needCred')
      return
    }
  } else if (!form.api_token.trim() && editId.value === null) {
    testError.value = t('cluster.needToken')
    return
  }
  testing.value = true
  try {
    const res = await testNode({
      base_url: form.base_url.trim(),
      api_token: form.auth_mode === 'token' ? form.api_token.trim() : undefined,
      username: form.auth_mode === 'password' ? form.username.trim() : undefined,
      password: form.auth_mode === 'password' ? form.password : undefined,
      totp_code: form.auth_mode === 'password' ? form.totp_code.trim() : undefined,
      insecure_tls: form.insecure_tls,
      id: editId.value ?? undefined,
    })
    // 账号密码模式只验证登录、不签 Token：没有快照，用文字结论代替
    if (res.data) testResult.value = res.data
    else testNote.value = res.message || t('cluster.credOk')
  } catch (e: any) {
    testError.value = e?.message || t('cluster.testFailed')
  } finally {
    testing.value = false
  }
}

async function save() {
  if (!form.name.trim() || !form.base_url.trim()) {
    ElMessage.warning(t('cluster.needNameUrl'))
    return
  }
  // 凭据按接入方式二选一；编辑态什么都不填 = 保持原 Token 不变
  const cred: Pick<NodePayload, 'api_token' | 'username' | 'password' | 'totp_code'> =
    form.auth_mode === 'password'
      ? {
          api_token: '',
          username: form.username.trim(),
          password: form.password,
          totp_code: form.totp_code.trim(),
        }
      : { api_token: form.api_token.trim() }
  const hasUser = !!cred.username?.trim()
  const hasPass = !!cred.password
  // 引导凭据要么不填（编辑 = 保持原 Token），要么填全；只填一半直接拦
  if (form.auth_mode === 'password' && (hasUser || hasPass) && !(hasUser && hasPass)) {
    ElMessage.warning(t('cluster.needCred'))
    return
  }
  if (editId.value === null) {
    if (form.auth_mode === 'password' && !hasUser) {
      ElMessage.warning(t('cluster.needCred'))
      return
    }
    if (form.auth_mode === 'token' && !cred.api_token) {
      ElMessage.warning(t('cluster.needToken'))
      return
    }
  }
  saving.value = true
  try {
    const extra = {
      group_id: form.group_id,
      owner_id: form.owner_id || undefined,
      cred_expire_days: form.cred_expire_days || undefined,
    }
    if (editId.value === null) {
      await addNode({
        name: form.name.trim(),
        base_url: form.base_url.trim(),
        remark: form.remark.trim(),
        insecure_tls: form.insecure_tls,
        ...cred,
        ...extra,
      })
      ElMessage.success(t('cluster.added'))
    } else {
      await updateNode({
        id: editId.value,
        name: form.name.trim(),
        base_url: form.base_url.trim(),
        remark: form.remark.trim(),
        insecure_tls: form.insecure_tls,
        ...cred,
        ...extra,
      })
      ElMessage.success(t('cluster.saved'))
    }
    dialog.value = false
    await load()
  } catch {
    /* 拦截器已弹窗（含后端返回的连通失败原因） */
  } finally {
    saving.value = false
  }
}

async function toggleStatus(r: NodeItem) {
  try {
    await updateNode({ id: r.id, status: r.status === 1 ? 0 : 1 })
    ElMessage.success(t('cluster.saved'))
    await load()
  } catch {
    /* 拦截器已弹窗 */
  }
}

async function remove(r: NodeItem) {
  try {
    await ElMessageBox.confirm(
      t('cluster.deleteConfirm', { name: r.name }),
      t('cluster.deleteTitle'),
      {
        type: 'warning',
        confirmButtonText: t('common.confirm'),
        cancelButtonText: t('common.cancel'),
      },
    )
  } catch {
    return
  }
  try {
    await deleteNodes([r.id])
    ElMessage.success(t('cluster.deleted'))
    await load()
  } catch {
    /* 拦截器已弹窗 */
  }
}

/** 手动跑一轮采集：新加的节点不用等下个周期 */
async function collect() {
  collecting.value = true
  try {
    await refreshNodes()
    await load()
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    collecting.value = false
  }
}

function onSelect(rows: NodeItem[]) {
  selected.value = rows
}

async function batchStatus(status: number) {
  try {
    const res = await batchUpdateNodes({ ids: selectedIds.value, status })
    ElMessage.success(t('cluster.batchDone', { count: res.data?.updated ?? 0 }))
    await load()
  } catch {
    /* 拦截器已弹窗 */
  }
}

function openBatchGroup() {
  batchGroupId.value = 0
  batchGroupDialog.value = true
}

async function saveBatchGroup() {
  try {
    const res = await batchUpdateNodes({ ids: selectedIds.value, group_id: batchGroupId.value })
    ElMessage.success(t('cluster.batchDone', { count: res.data?.updated ?? 0 }))
    batchGroupDialog.value = false
    await load()
  } catch {
    /* 拦截器已弹窗 */
  }
}

function openScript() {
  scriptPath.value = ''
  scriptDialog.value = true
}

async function runBatchScript() {
  if (!scriptPath.value.trim()) {
    ElMessage.warning(t('cluster.scriptPathRequired'))
    return
  }
  try {
    const res = await batchRunScript(selectedIds.value, scriptPath.value.trim())
    const failed = res.data?.results?.filter((r) => !r.ok) || []
    if (failed.length) {
      ElMessage.warning(t('cluster.scriptPartialFail', { count: failed.length }))
    } else {
      ElMessage.success(t('cluster.scriptDone'))
    }
    scriptDialog.value = false
  } catch {
    /* 拦截器已弹窗 */
  }
}

function openGroupDialog() {
  Object.assign(groupForm, { id: 0, name: '', sort_order: 0 })
  groupDialog.value = true
}

function editGroup(g: NodeGroupItem) {
  Object.assign(groupForm, { id: g.id, name: g.name, sort_order: g.sort_order })
}

async function saveGroup() {
  if (!groupForm.name.trim()) {
    ElMessage.warning(t('cluster.groupNameRequired'))
    return
  }
  try {
    if (groupForm.id) {
      await updateNodeGroup(groupForm.id, groupForm.name.trim(), groupForm.sort_order)
    } else {
      await addNodeGroup(groupForm.name.trim(), groupForm.sort_order)
    }
    Object.assign(groupForm, { id: 0, name: '', sort_order: 0 })
    await loadMeta()
  } catch {
    /* 拦截器已弹窗 */
  }
}

async function deleteGroup(g: NodeGroupItem) {
  try {
    await ElMessageBox.confirm(
      t('cluster.groupDeleteConfirm', { name: g.name }),
      t('cluster.groupManage'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteNodeGroup(g.id)
    await loadMeta()
    await load()
  } catch {
    /* 拦截器已弹窗 */
  }
}

async function openAlert() {
  try {
    const cfg = (await getAlertConfig()).data
    if (cfg) Object.assign(alertForm, cfg)
    alertDialog.value = true
  } catch {
    /* 拦截器已弹窗 */
  }
}

async function saveAlert() {
  try {
    await setAlertConfig({ ...alertForm })
    ElMessage.success(t('cluster.alertSaved'))
    alertDialog.value = false
  } catch {
    /* 拦截器已弹窗 */
  }
}

async function openRecords() {
  try {
    alertRecords.value = (await listAlertEvents()).data || []
    recordsDrawer.value = true
  } catch {
    /* 拦截器已弹窗 */
  }
}

onMounted(async () => {
  await loadMeta()
  await load()
})
</script>

<template>
  <div class="node-page">
    <el-alert type="info" :closable="false" show-icon :title="t('cluster.hint')" class="hint" />

    <div class="toolbar">
      <div class="toolbar-left">
        <el-button type="primary" @click="openAdd">{{ t('cluster.addNode') }}</el-button>
        <el-button :loading="collecting" @click="collect">{{ t('cluster.collectNow') }}</el-button>
        <el-button :loading="loading" @click="load">{{ t('common.refresh') }}</el-button>
        <el-button @click="openGroupDialog">{{ t('cluster.groupManage') }}</el-button>
        <el-button @click="openAlert">{{ t('cluster.alertSettings') }}</el-button>
        <el-button @click="openRecords">{{ t('cluster.alertRecords') }}</el-button>
      </div>
      <div class="toolbar-meta">
        {{ t('cluster.counts', { total: rows.length, online: onlineCount }) }}
      </div>
    </div>

    <div class="filter-bar">
      <el-input
        v-model="filters.keyword"
        :placeholder="t('cluster.filterKeyword')"
        clearable
        class="filter-input"
        @keyup.enter="load"
      />
      <el-select v-model="filters.group_id" class="filter-select" @change="load">
        <el-option :label="t('cluster.allGroups')" :value="0" />
        <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
      </el-select>
      <el-select v-model="filters.status" class="filter-select" @change="load">
        <el-option :label="t('cluster.statusAll')" :value="-1" />
        <el-option :label="t('cluster.enabled')" :value="1" />
        <el-option :label="t('cluster.disabled')" :value="0" />
      </el-select>
      <el-select v-model="filters.online" class="filter-select" @change="load">
        <el-option :label="t('cluster.onlineAll')" :value="-1" />
        <el-option :label="t('cluster.online')" :value="1" />
        <el-option :label="t('cluster.offline')" :value="0" />
      </el-select>
      <el-select v-model="filters.sort" class="filter-select" @change="load">
        <el-option :label="t('cluster.sortDefault')" value="id" />
        <el-option :label="t('cluster.sortByName')" value="name" />
        <el-option :label="t('cluster.sortByCpu')" value="cpu" />
        <el-option :label="t('cluster.sortByMemory')" value="memory" />
        <el-option :label="t('cluster.sortBySeen')" value="last_seen" />
      </el-select>
      <el-button @click="load">{{ t('cluster.filter') }}</el-button>
    </div>

    <div v-if="selected.length" class="batch-bar">
      <span class="batch-count">{{ t('cluster.selectedCount', { count: selected.length }) }}</span>
      <el-button size="small" type="success" @click="batchStatus(1)">
        {{ t('cluster.batchEnable') }}
      </el-button>
      <el-button size="small" @click="batchStatus(0)">{{ t('cluster.batchDisable') }}</el-button>
      <el-button size="small" @click="openBatchGroup">{{ t('cluster.batchSetGroup') }}</el-button>
      <el-button size="small" @click="openScript">{{ t('cluster.batchScript') }}</el-button>
      <el-button size="small" text @click="selected = []">{{
        t('cluster.clearSelection')
      }}</el-button>
    </div>

    <el-table
      v-loading="loading"
      :data="rows"
      row-key="id"
      class="node-table"
      @selection-change="onSelect"
    >
      <el-table-column type="selection" width="46" />
      <el-table-column :label="t('cluster.colName')" min-width="200">
        <template #default="{ row }">
          <div class="cell-name">
            <span class="dot" :class="row.online ? 'on' : 'off'"></span>
            <span class="name">{{ row.name }}</span>
            <el-tag v-if="row.status !== 1" type="info" size="small">
              {{ t('cluster.disabled') }}
            </el-tag>
          </div>
          <div class="cell-sub">{{ row.base_url }}</div>
        </template>
      </el-table-column>
      <el-table-column :label="t('cluster.group')" width="120">
        <template #default="{ row }">
          <el-tag v-if="row.group_name" size="small" type="info">{{ row.group_name }}</el-tag>
          <span v-else class="cell-sub">-</span>
        </template>
      </el-table-column>
      <el-table-column :label="t('cluster.colStatus')" width="110">
        <template #default="{ row }">
          <el-tooltip
            v-if="!row.online && row.last_error"
            :content="row.last_error"
            placement="top"
          >
            <el-tag type="danger" size="small">{{ t('cluster.offline') }}</el-tag>
          </el-tooltip>
          <el-tag v-else-if="row.online" type="success" size="small">
            {{ t('cluster.online') }}
          </el-tag>
          <el-tag v-else type="info" size="small">{{ t('cluster.unknown') }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('cluster.credStatus')" width="110">
        <template #default="{ row }">
          <el-tag
            :type="
              row.cred_status === 'valid'
                ? 'success'
                : row.cred_status === 'unknown'
                  ? 'info'
                  : 'danger'
            "
            size="small"
          >
            {{ credLabel(row.cred_status) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('cluster.colVersion')" width="100">
        <template #default="{ row }">{{ row.version || '-' }}</template>
      </el-table-column>
      <el-table-column :label="t('cluster.colOs')" min-width="160" show-overflow-tooltip>
        <template #default="{ row }">{{ row.os_name || '-' }}</template>
      </el-table-column>
      <el-table-column :label="t('cluster.colSpec')" width="160">
        <template #default="{ row }">
          <div class="cell-sub">
            {{ row.cpu_num ? `${row.cpu_num} ${t('cluster.cores')}` : '-' }} ·
            {{ sizeText(row.memory_total_b) }}
          </div>
          <div class="cell-sub">
            {{ t('cluster.diskTotal', { size: sizeText(row.disk_total_b) }) }}
          </div>
        </template>
      </el-table-column>
      <el-table-column :label="t('cluster.colUptime')" width="130">
        <template #default="{ row }">{{ row.uptime || '-' }}</template>
      </el-table-column>
      <el-table-column :label="t('cluster.colLastSeen')" width="180">
        <template #default="{ row }">{{ ago(row.last_seen) }}</template>
      </el-table-column>
      <el-table-column :label="t('cluster.colActions')" width="280" fixed="right">
        <template #default="{ row }">
          <!-- 接管：整页重载后面板直接操作这台机器 -->
          <el-tooltip v-if="row.online && row.status === 1" :content="t('cluster.nodeViewTip')">
            <el-button
              link
              type="warning"
              :disabled="nodeContext?.id === row.id"
              @click="takeover(row)"
            >
              {{ nodeContext?.id === row.id ? t('cluster.nodeViewActive') : t('cluster.takeover') }}
            </el-button>
          </el-tooltip>
          <el-button link type="primary" @click="openEdit(row)">{{ t('common.edit') }}</el-button>
          <el-button
            v-if="row.cred_status === 'expired' || row.cred_status === 'invalid'"
            link
            type="warning"
            @click="openEdit(row, true)"
          >
            {{ t('cluster.rotateCred') }}
          </el-button>
          <el-button link type="primary" @click="toggleStatus(row)">
            {{ row.status === 1 ? t('cluster.disable') : t('cluster.enable') }}
          </el-button>
          <el-button link type="danger" @click="remove(row)">{{ t('common.delete') }}</el-button>
        </template>
      </el-table-column>
      <template #empty>
        <el-empty :description="t('cluster.empty')">
          <el-button type="primary" @click="openAdd">{{ t('cluster.addNode') }}</el-button>
        </el-empty>
      </template>
    </el-table>

    <!-- 节点表单 -->
    <el-dialog v-model="dialog" :title="dialogTitle" width="580px" destroy-on-close>
      <el-form :model="form" label-width="110px" @submit.prevent>
        <el-form-item :label="t('cluster.name')">
          <el-input v-model="form.name" :placeholder="t('cluster.namePlaceholder')" clearable />
        </el-form-item>
        <el-form-item :label="t('cluster.baseUrl')">
          <el-input v-model="form.base_url" placeholder="https://10.0.0.5:8443" clearable />
          <div class="form-tip">{{ t('cluster.baseUrlHint') }}</div>
        </el-form-item>
        <el-form-item :label="t('cluster.group')">
          <el-select v-model="form.group_id" clearable class="full">
            <el-option :label="t('cluster.ungrouped')" :value="0" />
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="owners.length" :label="t('cluster.owner')">
          <el-select v-model="form.owner_id" clearable class="full">
            <el-option v-for="o in owners" :key="o.id" :label="o.username" :value="o.id" />
          </el-select>
          <div class="form-tip">{{ t('cluster.ownerHint') }}</div>
        </el-form-item>
        <el-form-item :label="t('cluster.credMode')">
          <el-radio-group v-model="form.auth_mode">
            <el-radio value="token">{{ t('cluster.credModeToken') }}</el-radio>
            <el-radio value="password">{{ t('cluster.credModePassword') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <template v-if="form.auth_mode === 'token'">
          <el-form-item :label="t('cluster.apiToken')">
            <el-input
              v-model="form.api_token"
              type="password"
              show-password
              :placeholder="tokenPlaceholder"
            />
            <div class="form-tip">{{ t('cluster.tokenHint') }}</div>
          </el-form-item>
        </template>
        <template v-else>
          <el-form-item :label="t('cluster.nodeUsername')">
            <el-input v-model="form.username" autocomplete="off" clearable />
          </el-form-item>
          <el-form-item :label="t('cluster.nodePassword')">
            <el-input
              v-model="form.password"
              type="password"
              show-password
              autocomplete="new-password"
            />
          </el-form-item>
          <el-form-item :label="t('cluster.nodeTotp')">
            <el-input v-model="form.totp_code" maxlength="8" clearable placeholder="123456" />
            <div class="form-tip">{{ t('cluster.credModeHint') }}</div>
          </el-form-item>
          <el-form-item :label="t('cluster.credExpireDays')">
            <el-input-number v-model="form.cred_expire_days" :min="0" :max="3650" />
            <div class="form-tip">{{ t('cluster.credExpireHint') }}</div>
          </el-form-item>
        </template>
        <el-form-item :label="t('cluster.insecureTls')">
          <el-switch v-model="form.insecure_tls" />
          <span class="switch-tip">{{ t('cluster.insecureHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('cluster.remark')">
          <el-input v-model="form.remark" clearable />
        </el-form-item>

        <div v-if="testResult" class="test-ok">
          <div class="test-title">{{ t('cluster.testOk') }}</div>
          <div class="test-line">
            {{ testResult.os_name || '-' }} · {{ testResult.host_name || '-' }}
          </div>
          <div class="test-line">
            {{ t('cluster.colVersion') }}: {{ testResult.version || '-' }} ·
            {{ testResult.cpu_num }} {{ t('cluster.cores') }} ·
            {{ sizeText(testResult.memory_total_b) }}
          </div>
          <div class="test-line">
            {{ t('cluster.uptime') }}: {{ testResult.uptime || '-' }} ·
            {{ testResult.public_ip || '-' }}
          </div>
        </div>
        <el-alert
          v-else-if="testError"
          type="error"
          :closable="false"
          show-icon
          :title="testError"
          class="test-err"
        />
        <el-alert
          v-else-if="testNote"
          type="success"
          :closable="false"
          show-icon
          :title="testNote"
          class="test-err"
        />
      </el-form>
      <template #footer>
        <el-button :loading="testing" @click="runTest">{{ t('cluster.testConn') }}</el-button>
        <el-button @click="dialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="save">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>

    <!-- 分组管理 -->
    <el-dialog v-model="groupDialog" :title="t('cluster.groupManage')" width="520px">
      <el-table :data="groups" size="small" class="dialog-table">
        <el-table-column :label="t('cluster.groupName')" min-width="160">
          <template #default="{ row }">{{ row.name }}</template>
        </el-table-column>
        <el-table-column :label="t('cluster.colName')" width="100">
          <template #default="{ row }">{{ row.count }}</template>
        </el-table-column>
        <el-table-column width="150">
          <template #default="{ row }">
            <el-button link type="primary" @click="editGroup(row)">{{
              t('common.edit')
            }}</el-button>
            <el-button link type="danger" @click="deleteGroup(row)">
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="group-form">
        <el-input v-model="groupForm.name" :placeholder="t('cluster.groupName')" clearable />
        <el-input-number v-model="groupForm.sort_order" :min="0" class="group-order" />
        <el-button type="primary" @click="saveGroup">
          {{ groupForm.id ? t('common.save') : t('cluster.addGroup') }}
        </el-button>
        <el-button
          v-if="groupForm.id"
          @click="editGroup({ id: 0, name: '', sort_order: 0, count: 0 })"
        >
          {{ t('common.cancel') }}
        </el-button>
      </div>
    </el-dialog>

    <!-- 批量改组 / 批量脚本 -->
    <el-dialog v-model="batchGroupDialog" :title="t('cluster.batchSetGroup')" width="420px">
      <el-select v-model="batchGroupId" class="full">
        <el-option :label="t('cluster.ungrouped')" :value="0" />
        <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
      </el-select>
      <template #footer>
        <el-button @click="batchGroupDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="saveBatchGroup">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>
    <el-dialog v-model="scriptDialog" :title="t('cluster.batchScript')" width="480px">
      <el-input v-model="scriptPath" :placeholder="t('cluster.scriptPath')" clearable />
      <div class="form-tip">{{ t('cluster.scriptHint') }}</div>
      <template #footer>
        <el-button @click="scriptDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="runBatchScript">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 告警设置 / 告警记录 -->
    <el-dialog v-model="alertDialog" :title="t('cluster.alertSettings')" width="480px">
      <el-form :model="alertForm" label-width="150px" @submit.prevent>
        <el-form-item :label="t('cluster.alertCpu')">
          <el-input-number v-model="alertForm.cpu" :min="0" :max="100" />
        </el-form-item>
        <el-form-item :label="t('cluster.alertMemory')">
          <el-input-number v-model="alertForm.memory" :min="0" :max="100" />
        </el-form-item>
        <el-form-item :label="t('cluster.alertDisk')">
          <el-input-number v-model="alertForm.disk" :min="0" :max="100" />
        </el-form-item>
        <el-form-item :label="t('cluster.alertOffline')">
          <el-input-number v-model="alertForm.offline_seconds" :min="0" :step="60" />
        </el-form-item>
        <el-form-item :label="t('cluster.alertCooldown')">
          <el-input-number v-model="alertForm.cooldown_seconds" :min="60" :step="300" />
        </el-form-item>
        <el-form-item :label="t('cluster.alertEmail')">
          <el-switch v-model="alertForm.email" />
        </el-form-item>
      </el-form>
      <div class="form-tip">{{ t('cluster.alertHint') }}</div>
      <template #footer>
        <el-button @click="alertDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="saveAlert">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>
    <el-drawer v-model="recordsDrawer" :title="t('cluster.alertRecords')" size="60%">
      <el-table :data="alertRecords" size="small">
        <el-table-column prop="created_at" :label="t('cluster.colLastSeen')" width="180">
          <template #default="{ row }">{{ ago(row.created_at) }}</template>
        </el-table-column>
        <el-table-column prop="node_name" :label="t('cluster.colName')" min-width="140" />
        <el-table-column prop="kind" label="Kind" width="110" />
        <el-table-column prop="message" :label="t('cluster.alertMessage')" min-width="260" />
      </el-table>
    </el-drawer>
  </div>
</template>

<style scoped>
.hint {
  margin-bottom: 14px;
}
.toolbar,
.filter-bar,
.batch-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.toolbar {
  justify-content: space-between;
}
.toolbar-left {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.toolbar-meta,
.batch-count {
  font-size: 13px;
  color: var(--el-text-color-secondary);
}
.filter-input {
  width: 220px;
}
.filter-select {
  width: 150px;
}
.full {
  width: 100%;
}
.node-table {
  width: 100%;
}
.cell-name {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
}
.cell-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.dot.on {
  background: var(--el-color-success);
}
.dot.off {
  background: var(--el-text-color-placeholder);
}
.form-tip {
  font-size: 12px;
  line-height: 1.5;
  color: var(--el-text-color-secondary);
}
.switch-tip {
  margin-left: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.test-ok {
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--el-color-success-light-9);
  border: 1px solid var(--el-color-success-light-7);
  font-size: 13px;
}
.test-title {
  font-weight: 600;
  color: var(--el-color-success);
  margin-bottom: 4px;
}
.test-line {
  color: var(--el-text-color-regular);
  line-height: 1.7;
}
.test-err {
  margin-top: 4px;
}
.dialog-table {
  margin-bottom: 12px;
}
.group-form {
  display: flex;
  align-items: center;
  gap: 8px;
}
.group-order {
  width: 120px;
}
</style>
