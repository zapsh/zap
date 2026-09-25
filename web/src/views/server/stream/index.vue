<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { useI18n } from 'vue-i18n'
import {
  addStreamRule,
  applyStream,
  deleteStreamRule,
  getStreamList,
  getStreamStatus,
  updateStreamRule,
  type StreamRule,
  type StreamStatus,
} from '@/api/stream'

const { t } = useI18n()

const loading = ref(false)
const saving = ref(false)
const items = ref<StreamRule[]>([])
const status = ref<StreamStatus | null>(null)
const dialogVisible = ref(false)
const editingId = ref<number | null>(null)
const formRef = ref<FormInstance>()

const form = reactive({
  name: '',
  listen_ip: '',
  listen_port: 13306,
  protocol: 'tcp',
  target_host: '',
  target_port: 3306,
  remark: '',
  status: 1,
})

const rules = computed<FormRules>(() => ({
  name: [{ required: true, message: t('stream.nameRequired'), trigger: 'blur' }],
  listen_port: [{ required: true, message: t('stream.portRequired'), trigger: 'blur' }],
  target_host: [{ required: true, message: t('stream.hostRequired'), trigger: 'blur' }],
  target_port: [{ required: true, message: t('stream.portRequired'), trigger: 'blur' }],
}))

/** 当前 Nginx 能不能用四层转发：装了 + 带 stream 模块 */
const usable = computed(() => !!status.value?.installed && !!status.value?.supported)

async function loadStatus() {
  try {
    const res = await getStreamStatus()
    status.value = res.data
  } catch {
    status.value = null
  }
}

async function load() {
  loading.value = true
  try {
    const res = await getStreamList()
    items.value = res.data.items ?? []
  } catch {
    /* 拦截器已提示 */
  } finally {
    loading.value = false
  }
}

function resetForm() {
  form.name = ''
  form.listen_ip = ''
  form.listen_port = 13306
  form.protocol = 'tcp'
  form.target_host = ''
  form.target_port = 3306
  form.remark = ''
  form.status = 1
  formRef.value?.clearValidate()
}

function openAdd() {
  resetForm()
  editingId.value = null
  dialogVisible.value = true
}

function openEdit(row: StreamRule) {
  resetForm()
  editingId.value = row.id
  form.name = row.name
  form.listen_ip = row.listen_ip === '0.0.0.0' ? '' : row.listen_ip
  form.listen_port = row.listen_port
  form.protocol = row.protocol
  form.target_host = row.target_host
  form.target_port = row.target_port
  form.remark = row.remark
  form.status = row.status
  dialogVisible.value = true
}

async function submitForm() {
  if (!formRef.value) return
  try {
    await formRef.value.validate()
  } catch {
    return
  }
  const payload = {
    name: form.name.trim(),
    listen_ip: form.listen_ip.trim(),
    listen_port: Number(form.listen_port),
    protocol: form.protocol,
    target_host: form.target_host.trim(),
    target_port: Number(form.target_port),
    remark: form.remark.trim(),
    status: form.status,
  }
  saving.value = true
  try {
    if (editingId.value) {
      const res = await updateStreamRule({ id: editingId.value, ...payload })
      ElMessage.success(res.message || t('stream.updated'))
    } else {
      const res = await addStreamRule(payload)
      ElMessage.success(res.message || t('stream.created'))
    }
    dialogVisible.value = false
    await Promise.all([load(), loadStatus()])
  } catch {
    /* 拦截器已提示 */
  } finally {
    saving.value = false
  }
}

async function remove(row: StreamRule) {
  try {
    await ElMessageBox.confirm(t('stream.deleteConfirm'), t('common.confirm'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    const res = await deleteStreamRule(row.id)
    ElMessage.success(res.message || t('stream.deleted'))
    await Promise.all([load(), loadStatus()])
  } catch {
    /* 拦截器已提示 */
  }
}

/** 改启用状态：直接走 update，后端会重新渲染 */
async function toggle(row: StreamRule) {
  try {
    await updateStreamRule({ id: row.id, status: row.status === 1 ? 0 : 1 })
    await Promise.all([load(), loadStatus()])
  } catch {
    /* 拦截器已提示 */
  }
}

async function reapply() {
  try {
    const res = await applyStream()
    ElMessage.success(res.message || t('stream.applied'))
    await loadStatus()
  } catch {
    /* 拦截器已提示 */
  }
}

onMounted(async () => {
  await Promise.all([load(), loadStatus()])
})
</script>

<template>
  <div class="page">
    <el-alert
      v-if="!usable"
      type="warning"
      show-icon
      :closable="false"
      :title="t('stream.unsupportedTitle')"
      :description="
        status?.installed ? t('stream.noStreamModule') : t('stream.noNginx')
      "
      style="margin-bottom: 16px"
    />

    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <span>{{ t('stream.title') }}</span>
          <div>
            <el-button :loading="loading" @click="load">{{ t('common.refresh') }}</el-button>
            <el-button :disabled="!usable" @click="reapply">
              {{ t('stream.apply') }}
            </el-button>
            <el-button type="primary" :disabled="!usable" @click="openAdd">
              {{ t('stream.add') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-table v-loading="loading" :data="items" border>
        <el-table-column :label="t('stream.name')" prop="name" min-width="140" />
        <el-table-column :label="t('stream.listen')" min-width="160">
          <template #default="{ row }">
            {{ row.listen_ip }}:{{ row.listen_port }}
          </template>
        </el-table-column>
        <el-table-column :label="t('stream.protocol')" width="90" align="center">
          <template #default="{ row }">
            <el-tag size="small" :type="row.protocol === 'udp' ? 'warning' : 'info'" effect="plain">
              {{ row.protocol }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('stream.target')" min-width="180">
          <template #default="{ row }">
            {{ row.target_host }}:{{ row.target_port }}
          </template>
        </el-table-column>
        <el-table-column :label="t('stream.remark')" prop="remark" min-width="160" show-overflow-tooltip />
        <el-table-column :label="t('common.status')" width="100" align="center">
          <template #default="{ row }">
            <el-switch
              :model-value="row.status === 1"
              :disabled="!usable"
              @change="toggle(row)"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('stream.actions')" width="150" align="center">
          <template #default="{ row }">
            <el-button link type="primary" @click="openEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button link type="danger" @click="remove(row)">{{ t('common.delete') }}</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <span class="muted">{{ t('stream.empty') }}</span>
        </template>
      </el-table>

      <div class="tip">{{ t('stream.tip') }}</div>
    </el-card>

    <el-dialog
      v-model="dialogVisible"
      :title="editingId ? t('stream.edit') : t('stream.add')"
      width="520px"
    >
      <el-form ref="formRef" :model="form" :rules="rules" label-width="110px">
        <el-form-item :label="t('stream.name')" prop="name">
          <el-input v-model="form.name" />
        </el-form-item>
        <el-form-item :label="t('stream.listenIp')">
          <el-input v-model="form.listen_ip" placeholder="0.0.0.0" />
          <div class="form-tip">{{ t('stream.listenIpTip') }}</div>
        </el-form-item>
        <el-form-item :label="t('stream.listenPort')" prop="listen_port">
          <el-input-number v-model="form.listen_port" :min="1" :max="65535" />
        </el-form-item>
        <el-form-item :label="t('stream.protocol')">
          <el-radio-group v-model="form.protocol">
            <el-radio value="tcp">TCP</el-radio>
            <el-radio value="udp">UDP</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="t('stream.targetHost')" prop="target_host">
          <el-input v-model="form.target_host" placeholder="10.0.0.5" />
        </el-form-item>
        <el-form-item :label="t('stream.targetPort')" prop="target_port">
          <el-input-number v-model="form.target_port" :min="1" :max="65535" />
        </el-form-item>
        <el-form-item :label="t('stream.remark')">
          <el-input v-model="form.remark" />
        </el-form-item>
        <el-form-item :label="t('common.status')">
          <el-radio-group v-model="form.status">
            <el-radio :value="1">{{ t('common.enable') }}</el-radio>
            <el-radio :value="0">{{ t('common.disable') }}</el-radio>
          </el-radio-group>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="submitForm">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.page {
  padding: 16px;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.card-header > div {
  display: flex;
  gap: 8px;
}
.form-tip,
.tip {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.muted {
  color: var(--el-text-color-secondary);
}
</style>
