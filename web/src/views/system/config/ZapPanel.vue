<template>
  <div class="zap-config">
    <el-card v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('zapCfg.title') }}</span>
          <span class="sub">{{ t('zapCfg.subtitle') }}</span>
          <el-button
            class="header-action"
            size="small"
            type="warning"
            plain
            :loading="restarting"
            @click="restartPanel"
          >
            {{ t('zapCfg.restart') }}
          </el-button>
        </div>
      </template>

      <el-tabs v-model="activeTab">
        <!-- ── 服务设置 ─────────────────────────────────── -->
        <el-tab-pane :label="t('zapCfg.tabServer')" name="server">
          <el-alert
            type="info"
            :closable="false"
            show-icon
            :title="t('zapCfg.serverHint')"
            style="margin-bottom: 16px"
          />
          <el-form :model="server" label-width="150px" style="max-width: 660px" @submit.prevent>
            <el-form-item :label="t('zapCfg.bindIp')">
              <el-select
                v-model="server.address"
                filterable
                allow-create
                default-first-option
                style="width: 280px"
              >
                <el-option
                  v-for="opt in addressOptions"
                  :key="opt.value"
                  :label="opt.label"
                  :value="opt.value"
                />
              </el-select>
              <div class="hint">{{ t('zapCfg.bindIpHint') }}</div>
            </el-form-item>
            <el-form-item :label="t('zapCfg.listenPort')">
              <el-input-number
                v-model="server.port"
                :min="1"
                :max="65535"
                controls-position="right"
                style="width: 180px"
              />
              <div class="hint">{{ t('zapCfg.portHint') }}</div>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingServer" @click="saveServer">
                {{ t('zapCfg.saveServer') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>

        <!-- ── SSL 证书 ─────────────────────────────────── -->
        <el-tab-pane :label="t('zapCfg.tabSsl')" name="ssl">
          <el-alert
            type="info"
            :closable="false"
            show-icon
            :title="t('zapCfg.sslHint')"
            style="margin-bottom: 16px"
          />

          <div class="section-title">{{ t('zapCfg.currentCert') }}</div>
          <el-descriptions :column="2" border size="small" style="max-width: 760px">
            <el-descriptions-item :label="t('zapCfg.commonName')">
              {{ current.common_name || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.domains')">
              {{ current.domains || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.issuer')">
              {{ current.issuer || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.certType')">
              <el-tag v-if="!current.cert_exists" type="info" size="small">
                {{ t('zapCfg.noCert') }}
              </el-tag>
              <el-tag v-else :type="current.self_signed ? 'warning' : 'success'" size="small">
                {{ current.self_signed ? t('zapCfg.selfSigned') : t('zapCfg.caSigned') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.validity')">
              <span v-if="current.not_after">
                {{ fmtDate(current.not_before) }} ~ {{ fmtDate(current.not_after) }}
              </span>
              <span v-else>-</span>
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.daysLeft')">
              <el-tag v-if="!current.not_after" type="info" size="small">-</el-tag>
              <el-tag
                v-else
                size="small"
                :type="
                  current.days_left < 0 ? 'danger' : current.days_left < 30 ? 'warning' : 'success'
                "
              >
                {{
                  current.days_left < 0
                    ? t('zapCfg.expired')
                    : t('zapCfg.daysUnit', { days: current.days_left })
                }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.certKeyPair')">
              <el-tag v-if="current.key_match === null" type="info" size="small">
                {{ t('zapCfg.unchecked') }}
              </el-tag>
              <el-tag v-else :type="current.key_match ? 'success' : 'danger'" size="small">
                {{ current.key_match ? t('zapCfg.match') : t('zapCfg.mismatch') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="t('zapCfg.filePath')">
              {{ current.cert_file || '-' }} / {{ current.key_file || '-' }}
            </el-descriptions-item>
          </el-descriptions>
          <el-alert
            v-if="current.error"
            type="warning"
            :closable="false"
            show-icon
            :title="current.error"
            style="margin-top: 12px; max-width: 760px"
          />

          <div class="section-title">{{ t('zapCfg.changeCert') }}</div>
          <el-form :model="sslForm" label-width="150px" style="max-width: 760px" @submit.prevent>
            <el-form-item :label="t('zapCfg.certSource')">
              <el-radio-group v-model="sslForm.source">
                <el-radio value="self-signed">{{ t('zapCfg.srcSelfSigned') }}</el-radio>
                <el-radio value="library">{{ t('zapCfg.srcLibrary') }}</el-radio>
                <el-radio value="manual">{{ t('zapCfg.srcManual') }}</el-radio>
              </el-radio-group>
            </el-form-item>

            <el-form-item v-if="sslForm.source === 'library'" :label="t('zapCfg.selectCert')">
              <el-select
                v-model="sslForm.cert_id"
                :placeholder="t('zapCfg.selectCertPlaceholder')"
                style="width: 420px"
              >
                <el-option
                  v-for="c in certs"
                  :key="c.id"
                  :label="c.domains ? `${c.name} (${c.domains})` : c.name"
                  :value="c.id"
                />
              </el-select>
              <div class="hint">{{ t('zapCfg.certLibraryEmpty') }}</div>
            </el-form-item>

            <template v-if="sslForm.source === 'manual'">
              <el-form-item :label="t('zapCfg.certPem')">
                <el-input
                  v-model="sslForm.cert_content"
                  type="textarea"
                  :rows="6"
                  :placeholder="t('zapCfg.certPemPlaceholder')"
                />
              </el-form-item>
              <el-form-item :label="t('zapCfg.keyPem')">
                <el-input
                  v-model="sslForm.key_content"
                  type="textarea"
                  :rows="6"
                  :placeholder="t('zapCfg.keyPemPlaceholder')"
                />
              </el-form-item>
            </template>

            <el-form-item :label="t('zapCfg.certFilePath')">
              <el-input v-model="sslForm.cert_file" placeholder="conf/zap.crt" clearable />
            </el-form-item>
            <el-form-item :label="t('zapCfg.keyFilePath')">
              <el-input v-model="sslForm.key_file" placeholder="conf/zap.key" clearable />
              <div class="hint">{{ t('zapCfg.keyFileHint') }}</div>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingSsl" @click="saveSsl">
                {{ t('zapCfg.saveSsl') }}
              </el-button>
              <el-button :loading="regening" @click="regenSelfSigned">
                {{ t('zapCfg.regenSelfSigned') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>

        <!-- ── 访问前缀 ─────────────────────────────────── -->
        <el-tab-pane :label="t('zapCfg.tabPrefix')" name="prefix">
          <el-alert
            type="info"
            :closable="false"
            show-icon
            :title="t('zapCfg.prefixHint')"
            style="margin-bottom: 16px"
          />
          <el-form label-width="150px" style="max-width: 660px" @submit.prevent>
            <el-form-item :label="t('zapCfg.urlPrefix')">
              <el-input
                v-model="server.url_prefix"
                :placeholder="t('zapCfg.urlPrefixPlaceholder')"
                clearable
                style="width: 280px"
              />
              <div class="hint">{{ t('zapCfg.urlPrefixHint') }}</div>
            </el-form-item>
            <el-form-item :label="t('zapCfg.panelUrl')">
              <span class="preview">{{ previewUrl }}</span>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingPrefix" @click="savePrefix">
                {{ t('zapCfg.savePrefix') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>

        <!-- ── 配置文件 ─────────────────────────────────── -->
        <el-tab-pane :label="t('zapCfg.tabFile')" name="file">
          <el-alert
            type="info"
            :closable="false"
            show-icon
            :title="t('zapCfg.fileHint')"
            style="margin-bottom: 16px"
          />
          <el-descriptions :column="1" border size="small" style="max-width: 760px">
            <el-descriptions-item :label="t('zapCfg.filePath')">
              {{ configPath || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('common.status')">
              <el-tag :type="configExists ? 'success' : 'danger'" size="small">
                {{ configExists ? t('zapCfg.loaded') : t('zapCfg.notExist') }}
              </el-tag>
            </el-descriptions-item>
          </el-descriptions>
          <el-input
            v-model="configContent"
            class="config-view"
            type="textarea"
            :rows="18"
            readonly
            :placeholder="t('zapCfg.configEmptyPlaceholder')"
          />
          <div style="margin-top: 12px">
            <el-button size="small" @click="copyConfig">{{ t('zapCfg.copyContent') }}</el-button>
            <el-button size="small" @click="load">{{ t('common.refresh') }}</el-button>
          </div>
        </el-tab-pane>
      </el-tabs>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  getZapSettings,
  regenerateSelfSignedCert,
  restartZapdService,
  saveZapSettings,
} from '@/api/systemZap'
import type { ZapCertOption, ZapSslCurrent, ZapSslSource } from '@/api/systemZap'
import { getLocale } from '@/i18n'

const { t } = useI18n()

const activeTab = ref('server')
const loading = ref(false)
const savingServer = ref(false)
const savingSsl = ref(false)
const savingPrefix = ref(false)
const restarting = ref(false)
const regening = ref(false)

const server = reactive({ address: '0.0.0.0', port: 2600, url_prefix: '' })
const sslForm = reactive({
  source: 'self-signed' as ZapSslSource,
  cert_id: undefined as number | undefined,
  cert_file: 'conf/zap.crt',
  key_file: 'conf/zap.key',
  cert_content: '',
  key_content: '',
})

const emptyCurrent: ZapSslCurrent = {
  exists: false,
  cert_file: '',
  key_file: '',
  cert_exists: false,
  key_exists: false,
  common_name: '',
  domains: '',
  issuer: '',
  not_before: 0,
  not_after: 0,
  days_left: 0,
  self_signed: false,
  key_match: null,
  error: '',
}
const current = ref<ZapSslCurrent>({ ...emptyCurrent })
const certs = ref<ZapCertOption[]>([])
const configPath = ref('')
const configExists = ref(false)
const configContent = ref('')

const addressOptions = computed(() => [
  { value: '0.0.0.0', label: t('zapCfg.bindAll') },
  { value: '127.0.0.1', label: t('zapCfg.bindLocal') },
  { value: '::', label: t('zapCfg.bindAllV6') },
])

const previewUrl = computed(() => {
  const p = server.url_prefix.trim().replace(/^\/+|\/+$/g, '')
  const { protocol, hostname } = window.location
  return `${protocol}//${hostname}:${server.port}${p ? `/${p}` : ''}/`
})

function fmtDate(ts: number): string {
  if (!ts) return '-'
  return new Date(ts * 1000).toLocaleDateString(getLocale())
}

const IPV4_RE = /^((25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)$/

function validIp(s: string): boolean {
  if (!s) return false
  if (s.includes(':')) return /^[0-9a-fA-F:]+$/.test(s) && s.split('::').length <= 2
  return IPV4_RE.test(s)
}

async function load() {
  loading.value = true
  try {
    const res = await getZapSettings()
    const d = res.data
    server.address = d.server.address || '0.0.0.0'
    server.port = d.server.port || 2600
    server.url_prefix = d.server.url_prefix ?? ''
    sslForm.source = d.ssl.source || 'self-signed'
    sslForm.cert_id = d.ssl.cert_id || undefined
    sslForm.cert_file = d.ssl.current.cert_file
    sslForm.key_file = d.ssl.current.key_file
    current.value = { ...emptyCurrent, ...d.ssl.current }
    certs.value = d.certs ?? []
    configPath.value = d.config_path ?? ''
    configExists.value = !!d.config_exists
    configContent.value = d.config_content ?? ''
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

/** 保存后提示重启：端口 / 证书 / 前缀都在启动时生效 */
async function confirmRestart(tip: string) {
  try {
    await ElMessageBox.confirm(t('zapCfg.willRestart', { tip }), t('zapCfg.tipTitle'), {
      type: 'warning',
      confirmButtonText: t('zapCfg.restartNow'),
      cancelButtonText: t('zapCfg.restartLater'),
    })
  } catch {
    return
  }
  await doRestart()
}

async function doRestart() {
  restarting.value = true
  try {
    await restartZapdService()
    ElMessage.success(t('zapCfg.restartSent'))
  } catch {
    /* handled */
  } finally {
    restarting.value = false
  }
}

async function restartPanel() {
  try {
    await ElMessageBox.confirm(t('zapCfg.restartConfirm'), t('zapCfg.tipTitle'), {
      type: 'warning',
      confirmButtonText: t('zapCfg.restartConfirmBtn'),
    })
  } catch {
    return
  }
  await doRestart()
}

async function saveServer() {
  const address = server.address.trim()
  if (!validIp(address)) {
    ElMessage.warning(t('zapCfg.invalidIp'))
    return
  }
  const port = Number(server.port)
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    ElMessage.warning(t('zapCfg.invalidPort'))
    return
  }
  savingServer.value = true
  try {
    const res = await saveZapSettings({ server: { address, port } })
    ElMessage.success(res.message || t('zapCfg.serverSaved'))
    await confirmRestart(t('zapCfg.tipAddressPort'))
  } catch {
    /* handled */
  } finally {
    savingServer.value = false
  }
}

async function saveSsl() {
  const certFile = sslForm.cert_file.trim()
  const keyFile = sslForm.key_file.trim()
  if (!certFile || !keyFile) {
    ElMessage.warning(t('zapCfg.certPathRequired'))
    return
  }
  if (certFile === keyFile) {
    ElMessage.warning(t('zapCfg.sameFile'))
    return
  }
  const payload: {
    source: ZapSslSource
    cert_file: string
    key_file: string
    cert_id?: number
    cert_content?: string
    key_content?: string
  } = { source: sslForm.source, cert_file: certFile, key_file: keyFile }

  if (sslForm.source === 'library') {
    if (!sslForm.cert_id) {
      ElMessage.warning(t('zapCfg.selectCertRequired'))
      return
    }
    payload.cert_id = sslForm.cert_id
  }
  if (sslForm.source === 'manual') {
    if (!sslForm.cert_content.trim() || !sslForm.key_content.trim()) {
      ElMessage.warning(t('zapCfg.pemRequired'))
      return
    }
    payload.cert_content = sslForm.cert_content
    payload.key_content = sslForm.key_content
  }

  savingSsl.value = true
  try {
    const res = await saveZapSettings({ ssl: payload })
    ElMessage.success(res.message || t('zapCfg.sslSaved'))
    await load()
    await confirmRestart(t('zapCfg.tipCert'))
  } catch {
    /* handled */
  } finally {
    savingSsl.value = false
  }
}

async function regenSelfSigned() {
  try {
    await ElMessageBox.confirm(t('zapCfg.regenConfirm'), t('zapCfg.tipTitle'), {
      type: 'warning',
      confirmButtonText: t('zapCfg.regenConfirmBtn'),
    })
  } catch {
    return
  }
  regening.value = true
  try {
    const res = await regenerateSelfSignedCert()
    ElMessage.success(res.message || t('zapCfg.regenDone'))
    await load()
    await confirmRestart(t('zapCfg.tipCert'))
  } catch {
    /* handled */
  } finally {
    regening.value = false
  }
}

async function savePrefix() {
  const prefix = server.url_prefix.trim().replace(/^\/+|\/+$/g, '')
  if (prefix && !/^[A-Za-z0-9._~\-/]+$/.test(prefix)) {
    ElMessage.warning(t('zapCfg.invalidPrefix'))
    return
  }
  savingPrefix.value = true
  try {
    const res = await saveZapSettings({ server: { url_prefix: prefix } })
    ElMessage.success(res.message || t('zapCfg.prefixSaved'))
    try {
      await ElMessageBox.alert(
        t('zapCfg.prefixUpdatedMsg', { url: previewUrl.value }),
        t('zapCfg.prefixUpdatedTitle'),
        { confirmButtonText: t('zapCfg.gotIt') },
      )
    } catch {
      /* handled */
    }
    await confirmRestart(t('zapCfg.tipPrefix'))
  } catch {
    /* handled */
  } finally {
    savingPrefix.value = false
  }
}

async function copyConfig() {
  try {
    await navigator.clipboard.writeText(configContent.value)
    ElMessage.success(t('zapCfg.configCopied'))
  } catch {
    ElMessage.warning(t('zapCfg.copyFailedManual'))
  }
}

onMounted(load)
</script>

<script lang="ts">
export default { name: 'ZapConfigPanel' }
</script>

<style scoped>
.zap-config {
  padding: 4px;
}
.card-header {
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.card-header .sub {
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.card-header .header-action {
  margin-left: auto;
}
.section-title {
  margin: 18px 0 10px;
  font-size: 14px;
  font-weight: 600;
}
.section-title:first-of-type {
  margin-top: 0;
}
.hint {
  margin-top: 4px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.6;
}
.preview {
  font-family: var(--el-font-family-monospace, monospace);
  color: var(--el-color-primary);
}
.config-view :deep(textarea) {
  font-family: var(--el-font-family-monospace, monospace);
  font-size: 12px;
  line-height: 1.6;
}
</style>
