<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { useI18n } from 'vue-i18n'
import {
  addStreamRule,
  applyStream,
  deleteStreamRule,
  getStreamCerts,
  getStreamList,
  getStreamStatus,
  updateStreamRule,
  type StreamCertOption,
  type StreamRule,
  type StreamStatus,
} from '@/api/stream'
// 含 nginx 语法（{ } / $）的提示文案走这里，不经过 vue-i18n 解析（见 utils/nginx-text.ts）
import { nginxText } from '@/utils/nginx-text'

const { t, locale } = useI18n()

const loading = ref(false)
const saving = ref(false)
const items = ref<StreamRule[]>([])
const status = ref<StreamStatus | null>(null)
const certOptions = ref<StreamCertOption[]>([])
const dialogVisible = ref(false)
const editingId = ref<number | null>(null)
const formRef = ref<FormInstance>()

const form = reactive({
  name: '',
  listen_ip: '',
  listen_port: 13306,
  protocol: 'tcp',
  // 后端地址：`10.0.1.10:3306` 或 upstream 名（不带端口）
  target: '',
  // basic / advanced
  mode: 'basic',
  raw: '',
  // single = 单后端直接转发；group = upstream 负载组
  backend_mode: 'group',
  targets: '',
  listen_opts: '',
  proxy_connect_timeout: '',
  proxy_timeout: '',
  proxy_responses: 0,
  ssl_enable: false,
  ssl_certificate_id: 0,
  ssl_certificate: '',
  ssl_certificate_key: '',
  ssl_protocols: '',
  ssl_ciphers: '',
  ssl_preread: false,
  proxy_pass: '',
  extra: '',
  remark: '',
  status: 1,
})

/** 高级模式：整段自定义配置，结构化字段不参与渲染 */
const isAdvanced = computed(() => form.mode === 'advanced')
/** 负载组模式才需要填一组成员 */
const isGroup = computed(() => form.backend_mode === 'group')

// 规则保持静态：高级模式下「后端地址」不是必填，改成提交时手动判断，
// 避免 rules 随模式变化（之前那版 computed rules 会在切换时炸）
const rules: FormRules = {
  name: [{ required: true, message: t('stream.nameRequired'), trigger: 'blur' }],
  listen_port: [{ required: true, message: t('stream.portRequired'), trigger: 'blur' }],
}

/** 模式切换：只认 advanced，其它脏值一律当基础，保证界面与保存值一致 */
function onModeChange(v: string | number | boolean | undefined) {
  form.mode = v === 'advanced' ? 'advanced' : 'basic'
  formRef.value?.clearValidate()
}

/** 后端模式切换：只认 single，其它当负载组 */
function onBackendModeChange(v: string | number | boolean | undefined) {
  form.backend_mode = v === 'single' ? 'single' : 'group'
}

/** 当前 Nginx 能不能用四层转发：装了 + 带 stream 模块 */
const usable = computed(() => !!status.value?.installed && !!status.value?.supported)

/** 保存/应用结果提示：applied=false = 入库成功但 Nginx 没生效，用警告并停留更久 */
function notify(res: { message?: string; data?: unknown }, fallback: string) {
  const applied = (res.data as { applied?: boolean } | undefined)?.applied !== false
  ElMessage({
    type: applied ? 'success' : 'warning',
    message: res.message || fallback,
    duration: applied ? 3000 : 8000,
    showClose: !applied,
  })
}

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

/** 证书库下拉：失败不影响建规则（还能手工填路径） */
async function loadCerts() {
  try {
    const res = await getStreamCerts()
    certOptions.value = res.data.items ?? []
  } catch {
    certOptions.value = []
  }
}

function resetForm() {
  form.name = ''
  form.listen_ip = ''
  form.listen_port = 13306
  form.protocol = 'tcp'
  form.target = ''
  form.mode = 'basic'
  form.raw = ''
  form.backend_mode = 'group'
  form.targets = ''
  form.listen_opts = ''
  form.proxy_connect_timeout = ''
  form.proxy_timeout = ''
  form.proxy_responses = 0
  form.ssl_enable = false
  form.ssl_certificate_id = 0
  form.ssl_certificate = ''
  form.ssl_certificate_key = ''
  form.ssl_protocols = ''
  form.ssl_ciphers = ''
  form.ssl_preread = false
  form.proxy_pass = ''
  form.extra = ''
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
  form.target = row.target || row.target_host
  // 只认这两个值，避免库里存了脏值导致「列表显示基础、表单里是高级」
  form.mode = row.mode === 'advanced' ? 'advanced' : 'basic'
  form.raw = row.raw || ''
  form.backend_mode = row.backend_mode === 'single' ? 'single' : 'group'
  form.targets = row.targets || ''
  form.listen_opts = row.listen_opts || ''
  form.proxy_connect_timeout = row.proxy_connect_timeout || ''
  form.proxy_timeout = row.proxy_timeout || ''
  form.proxy_responses = row.proxy_responses || 0
  form.ssl_enable = !!row.ssl_enable
  form.ssl_certificate_id = row.ssl_certificate_id || 0
  form.ssl_certificate = row.ssl_certificate || ''
  form.ssl_certificate_key = row.ssl_certificate_key || ''
  form.ssl_protocols = row.ssl_protocols || ''
  form.ssl_ciphers = row.ssl_ciphers || ''
  form.ssl_preread = !!row.ssl_preread
  form.proxy_pass = row.proxy_pass || ''
  form.extra = row.extra || ''
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
  if (isAdvanced.value && !form.raw.trim()) {
    ElMessage.warning(t('stream.advancedRequired'))
    return
  }
  // 基础模式才要求后端地址（高级模式自己写配置，后端地址只是展示）
  if (!isAdvanced.value && !form.target.trim()) {
    ElMessage.warning(t('stream.hostRequired'))
    return
  }
  if (form.ssl_enable && !form.ssl_certificate_id && !form.ssl_certificate.trim()) {
    ElMessage.warning(t('stream.certRequired'))
    return
  }
  const base = {
    name: form.name.trim(),
    listen_ip: form.listen_ip.trim(),
    listen_port: Number(form.listen_port),
    protocol: form.protocol,
    target: form.target.trim(),
    remark: form.remark.trim(),
    status: form.status,
  }
  // 高级模式只认 raw；切回基础模式要把 raw 清掉，避免残留内容继续生效
  const advanced = { mode: 'advanced', raw: form.raw.trim() }
  const basic = {
    mode: 'basic',
    raw: '',
    backend_mode: form.backend_mode,
    targets: isGroup.value ? form.targets.trim() : '',
    listen_opts: form.listen_opts.trim(),
    proxy_connect_timeout: form.proxy_connect_timeout.trim(),
    proxy_timeout: form.proxy_timeout.trim(),
    proxy_responses: Number(form.proxy_responses) || 0,
    ssl_enable: form.ssl_enable,
    ssl_certificate_id: Number(form.ssl_certificate_id) || 0,
    ssl_certificate: form.ssl_certificate.trim(),
    ssl_certificate_key: form.ssl_certificate_key.trim(),
    ssl_protocols: form.ssl_protocols.trim(),
    ssl_ciphers: form.ssl_ciphers.trim(),
    ssl_preread: form.ssl_preread,
    proxy_pass: form.proxy_pass.trim(),
    extra: form.extra.trim(),
  }
  const payload = { ...base, ...(isAdvanced.value ? advanced : basic) }
  saving.value = true
  try {
    if (editingId.value) {
      const res = await updateStreamRule({ id: editingId.value, ...payload })
      notify(res, t('stream.updated'))
    } else {
      const res = await addStreamRule(payload)
      notify(res, t('stream.created'))
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
    notify(res, t('stream.deleted'))
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
    notify(res, t('stream.applied'))
    await loadStatus()
  } catch {
    /* 拦截器已提示 */
  }
}

onMounted(async () => {
  await Promise.all([load(), loadStatus(), loadCerts()])
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
        status?.installed
          ? t('stream.noStreamModule')
          : status?.error || t('stream.noNginx')
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
        <el-table-column :label="t('stream.mode')" width="90" align="center">
          <template #default="{ row }">
            <el-tag
              size="small"
              :type="row.mode === 'advanced' ? 'danger' : 'success'"
              effect="plain"
            >
              {{ row.mode === 'advanced' ? t('stream.modeAdvanced') : t('stream.modeBasic') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('stream.target')" min-width="180">
          <template #default="{ row }">
            {{ row.target }}
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
      <div v-if="status?.installed" class="tip">
        {{ t('stream.confFile') }}：{{ status.file }}
        <span v-if="status.conf">（{{ status.conf }}）</span>
      </div>
    </el-card>

    <el-dialog
      v-model="dialogVisible"
      :title="editingId ? t('stream.edit') : t('stream.add')"
      width="640px"
      destroy-on-close
    >
      <!-- 模式切换放在表单外：切换时只有字段区重建，切换器本身不动 -->
      <div class="mode-row">
        <span class="mode-label">{{ t('stream.mode') }}</span>
        <el-radio-group :model-value="form.mode" @change="onModeChange">
          <el-radio-button value="basic">{{ t('stream.modeBasic') }}</el-radio-button>
          <el-radio-button value="advanced">{{ t('stream.modeAdvanced') }}</el-radio-button>
        </el-radio-group>
      </div>

      <!-- key 绑模式：切换时整个表单重建，避免残留校验状态 -->
      <el-form
        :key="form.mode"
        ref="formRef"
        :model="form"
        :rules="rules"
        label-width="130px"
      >
        <el-form-item v-if="isAdvanced" :label="t('stream.advancedTitle')">
          <el-input
            v-model="form.raw"
            type="textarea"
            :autosize="{ minRows: 10, maxRows: 24 }"
            :placeholder="nginxText(locale, 'advancedPlaceholder')"
          />
          <div class="form-tip">{{ nginxText(locale, 'advancedTip') }}</div>
        </el-form-item>

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

        <template v-if="!isAdvanced">
          <el-form-item :label="t('stream.backendMode')">
            <el-radio-group
              :model-value="form.backend_mode"
              @change="onBackendModeChange"
            >
              <el-radio-button value="single">{{ t('stream.backendSingle') }}</el-radio-button>
              <el-radio-button value="group">{{ t('stream.backendGroup') }}</el-radio-button>
            </el-radio-group>
            <div class="form-tip">{{ t('stream.backendModeTip') }}</div>
          </el-form-item>

          <el-form-item v-if="isGroup" :label="t('stream.targets')">
            <el-input
              v-model="form.targets"
              type="textarea"
              :autosize="{ minRows: 2, maxRows: 8 }"
              :placeholder="t('stream.targetsPlaceholder')"
            />
            <div class="form-tip">{{ t('stream.targetsTip') }}</div>
          </el-form-item>
        </template>

        <!-- 不挂 prop：高级模式下非必填，改在提交时手动校验
             （prop 与 rules 对不上时 Element Plus 校验会抛错） -->
        <el-form-item :label="t('stream.targetHost')">
          <el-input v-model="form.target" placeholder="10.0.1.10:3306" />
          <div class="form-tip">{{ t('stream.targetTip') }}</div>
        </el-form-item>

        <template v-if="!isAdvanced">
          <el-form-item :label="t('stream.listenOpts')">
            <el-input v-model="form.listen_opts" placeholder="reuseport" />
            <div class="form-tip">{{ t('stream.listenOptsTip') }}</div>
          </el-form-item>
          <el-form-item :label="t('stream.proxyConnectTimeout')">
            <el-input v-model="form.proxy_connect_timeout" placeholder="5s" />
          </el-form-item>
          <el-form-item :label="t('stream.proxyTimeout')">
            <el-input v-model="form.proxy_timeout" placeholder="1h" />
          </el-form-item>
          <el-form-item :label="t('stream.proxyResponses')">
            <el-input-number v-model="form.proxy_responses" :min="0" :max="1000" />
            <div class="form-tip">{{ t('stream.proxyResponsesTip') }}</div>
          </el-form-item>

          <el-form-item :label="t('stream.sslEnable')">
            <el-switch v-model="form.ssl_enable" />
            <div class="form-tip">{{ t('stream.sslEnableTip') }}</div>
          </el-form-item>
          <template v-if="form.ssl_enable">
            <el-form-item :label="t('stream.certFromStore')">
              <el-select
                v-model="form.ssl_certificate_id"
                clearable
                filterable
                :placeholder="t('stream.certFromStorePlaceholder')"
                style="width: 100%"
              >
                <el-option
                  v-for="c in certOptions"
                  :key="c.id"
                  :label="c.name + (c.domains ? '（' + c.domains + '）' : '')"
                  :value="c.id"
                />
              </el-select>
              <div class="form-tip">{{ t('stream.certFromStoreTip') }}</div>
            </el-form-item>
            <template v-if="!form.ssl_certificate_id">
              <el-form-item :label="t('stream.sslCertificate')">
                <el-input v-model="form.ssl_certificate" placeholder="/etc/nginx/ssl/tls.crt" />
              </el-form-item>
              <el-form-item :label="t('stream.sslCertificateKey')">
                <el-input v-model="form.ssl_certificate_key" placeholder="/etc/nginx/ssl/tls.key" />
              </el-form-item>
            </template>
            <el-form-item :label="t('stream.sslProtocols')">
              <el-input v-model="form.ssl_protocols" placeholder="TLSv1.2 TLSv1.3" />
            </el-form-item>
            <el-form-item :label="t('stream.sslCiphers')">
              <el-input v-model="form.ssl_ciphers" placeholder="HIGH:!aNULL:!MD5" />
            </el-form-item>
          </template>

          <el-form-item :label="t('stream.sslPreread')">
            <el-switch v-model="form.ssl_preread" />
            <div class="form-tip">{{ nginxText(locale, 'sslPrereadTip') }}</div>
          </el-form-item>
          <el-form-item v-if="form.ssl_preread" :label="t('stream.proxyPass')">
            <el-input v-model="form.proxy_pass" placeholder="$backend" />
            <div class="form-tip">{{ nginxText(locale, 'proxyPassTip') }}</div>
          </el-form-item>

          <el-form-item :label="t('stream.extra')">
            <el-input
              v-model="form.extra"
              type="textarea"
              :autosize="{ minRows: 2, maxRows: 10 }"
              placeholder="proxy_buffer_size 16k;"
            />
            <div class="form-tip">{{ nginxText(locale, 'extraTip') }}</div>
          </el-form-item>
        </template>

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
.mode-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 18px;
}
.mode-label {
  width: 130px;
  font-size: 14px;
  color: var(--el-text-color-primary);
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
