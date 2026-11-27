<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
/**
 * 多节点 → 节点管理：登记受管节点（面板地址 + 该节点的 API Token）。
 *
 * Token 只写不读：列表接口不回传原文，编辑时留空即保持原值。
 * 添加时后端会先实测连通（打一次节点的 /system/info），连不上就不落库，
 * 避免库里存一堆采集失败、只会在大屏上显示离线的地址。
 */
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { formatBytes } from '@/utils/fmt'
import {
  addNode,
  deleteNodes,
  listNodes,
  refreshNodes,
  testNode,
  updateNode,
  type NodeItem,
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

const form = reactive({
  name: '',
  base_url: '',
  api_token: '',
  remark: '',
  insecure_tls: true,
})

const dialogTitle = computed(() =>
  editId.value === null ? t('cluster.addNode') : t('cluster.editNode'),
)
/** 编辑态 Token 留空 = 不改，占位文案必须说清楚，否则用户以为被清空了 */
const tokenPlaceholder = computed(() =>
  editId.value === null ? t('cluster.tokenPlaceholderNew') : t('cluster.tokenPlaceholderEdit'),
)
const onlineCount = computed(() => rows.value.filter((r) => r.online).length)

function ago(ts: number): string {
  return ts ? new Date(ts * 1000).toLocaleString() : '-'
}

function sizeText(bytes: number): string {
  return bytes ? formatBytes(bytes) : '-'
}

async function load() {
  loading.value = true
  try {
    const res = await listNodes()
    rows.value = res.data || []
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    loading.value = false
  }
}

function resetForm(r?: NodeItem) {
  Object.assign(form, {
    name: r?.name ?? '',
    base_url: r?.base_url ?? '',
    // Token 永不回填，留空即不修改
    api_token: '',
    remark: r?.remark ?? '',
    insecure_tls: r?.insecure_tls ?? true,
  })
  testResult.value = null
  testError.value = ''
}

function openAdd() {
  editId.value = null
  resetForm()
  dialog.value = true
}

function openEdit(r: NodeItem) {
  editId.value = r.id
  resetForm(r)
  dialog.value = true
}

/** 只测连通、不保存：失败也把原因显示出来，方便当场排查（Token 错 / 证书 / 端口不通） */
async function runTest() {
  testResult.value = null
  testError.value = ''
  if (!form.base_url.trim()) {
    testError.value = t('cluster.needUrl')
    return
  }
  if (!form.api_token.trim() && editId.value === null) {
    testError.value = t('cluster.needToken')
    return
  }
  testing.value = true
  try {
    const res = await testNode({
      base_url: form.base_url.trim(),
      api_token: form.api_token.trim(),
      insecure_tls: form.insecure_tls,
      id: editId.value ?? undefined,
    })
    testResult.value = res.data
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
  if (editId.value === null && !form.api_token.trim()) {
    ElMessage.warning(t('cluster.needToken'))
    return
  }
  saving.value = true
  try {
    if (editId.value === null) {
      await addNode({
        name: form.name.trim(),
        base_url: form.base_url.trim(),
        api_token: form.api_token.trim(),
        remark: form.remark.trim(),
        insecure_tls: form.insecure_tls,
      })
      ElMessage.success(t('cluster.added'))
    } else {
      await updateNode({
        id: editId.value,
        name: form.name.trim(),
        base_url: form.base_url.trim(),
        api_token: form.api_token.trim(),
        remark: form.remark.trim(),
        insecure_tls: form.insecure_tls,
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

onMounted(load)
</script>

<template>
  <div class="node-page">
    <el-alert type="info" :closable="false" show-icon :title="t('cluster.hint')" class="hint" />

    <div class="toolbar">
      <div class="toolbar-left">
        <el-button type="primary" @click="openAdd">{{ t('cluster.addNode') }}</el-button>
        <el-button :loading="collecting" @click="collect">{{ t('cluster.collectNow') }}</el-button>
        <el-button :loading="loading" @click="load">{{ t('common.refresh') }}</el-button>
      </div>
      <div class="toolbar-meta">
        {{ t('cluster.counts', { total: rows.length, online: onlineCount }) }}
      </div>
    </div>

    <el-table v-loading="loading" :data="rows" row-key="id" class="node-table">
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
      <el-table-column :label="t('cluster.colActions')" width="250" fixed="right">
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

    <el-dialog v-model="dialog" :title="dialogTitle" width="580px" destroy-on-close>
      <el-form :model="form" label-width="110px" @submit.prevent>
        <el-form-item :label="t('cluster.name')">
          <el-input v-model="form.name" :placeholder="t('cluster.namePlaceholder')" clearable />
        </el-form-item>
        <el-form-item :label="t('cluster.baseUrl')">
          <el-input v-model="form.base_url" placeholder="https://10.0.0.5:8443" clearable />
          <div class="form-tip">{{ t('cluster.baseUrlHint') }}</div>
        </el-form-item>
        <el-form-item :label="t('cluster.apiToken')">
          <el-input
            v-model="form.api_token"
            type="password"
            show-password
            :placeholder="tokenPlaceholder"
          />
          <div class="form-tip">{{ t('cluster.tokenHint') }}</div>
        </el-form-item>
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
      </el-form>
      <template #footer>
        <el-button :loading="testing" @click="runTest">{{ t('cluster.testConn') }}</el-button>
        <el-button @click="dialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="save">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.hint {
  margin-bottom: 14px;
}
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.toolbar-meta {
  font-size: 13px;
  color: var(--el-text-color-secondary);
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
</style>
