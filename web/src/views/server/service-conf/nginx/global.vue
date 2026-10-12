<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div v-loading="loading" class="nginx-global">
    <el-alert
      v-if="spec"
      type="info"
      show-icon
      :closable="false"
      :title="t('statusNginx.globalGenTitle')"
      :description="seeded ? t('statusNginx.globalSeededTip') : t('statusNginx.globalGenTip')"
      class="mb-3"
    />

    <el-tabs v-model="tab" type="border-card">
      <!-- ── 全局 / events ── -->
      <el-tab-pane :label="t('statusNginx.tabGlobal')" name="main">
        <el-form label-width="150px" class="row-form">
          <DirectiveRow v-for="f in MAIN_FIELDS" :key="f.key" :field="f" :model="general" />
        </el-form>
        <el-divider content-position="left">load_module</el-divider>
        <div v-for="(m, i) in spec.load_modules || []" :key="i" class="list-row">
          <el-input v-model="spec.load_modules![i]" class="grow mono" />
          <el-button link type="danger" :icon="Delete" @click="rm(spec.load_modules!, i)" />
        </div>
        <el-button size="small" :icon="Plus" @click="spec.load_modules!.push('')">
          {{ t('statusNginx.addModule') }}
        </el-button>
      </el-tab-pane>

      <el-tab-pane label="events" name="events">
        <el-form label-width="150px" class="row-form">
          <DirectiveRow v-for="f in EVENT_FIELDS" :key="f.key" :field="f" :model="events" />
        </el-form>
      </el-tab-pane>

      <!-- ── http 分组 ── -->
      <el-tab-pane :label="t('statusNginx.tabHttpBasic')" name="basic">
        <el-form label-width="230px" class="row-form">
          <DirectiveRow v-for="f in BASIC_FIELDS" :key="f.key" :field="f" :model="http.basic" />
        </el-form>
      </el-tab-pane>

      <el-tab-pane :label="t('statusNginx.tabGzip')" name="gzip">
        <el-form label-width="200px" class="row-form">
          <DirectiveRow v-for="f in GZIP_FIELDS" :key="f.key" :field="f" :model="http.gzip" />
        </el-form>
      </el-tab-pane>

      <el-tab-pane :label="t('statusNginx.tabTls')" name="tls">
        <el-form label-width="230px" class="row-form">
          <DirectiveRow v-for="f in TLS_FIELDS" :key="f.key" :field="f" :model="http.tls" />
        </el-form>
      </el-tab-pane>

      <el-tab-pane :label="t('statusNginx.tabLog')" name="log">
        <el-form label-width="200px" class="row-form">
          <DirectiveRow v-for="f in LOG_FIELDS" :key="f.key" :field="f" :model="http.logging" />
        </el-form>
      </el-tab-pane>

      <el-tab-pane :label="t('statusNginx.tabProxy')" name="proxy">
        <el-form label-width="230px" class="row-form">
          <DirectiveRow v-for="f in PROXY_FIELDS" :key="f.key" :field="f" :model="http.proxy" />
        </el-form>
      </el-tab-pane>

      <!-- ── 代理缓存（多 zone）── -->
      <el-tab-pane :label="t('statusNginx.tabCache')" name="cache">
        <div class="zone-toolbar">
          <span class="tip">{{ t('statusNginx.cacheTip') }}</span>
          <el-button size="small" :icon="Plus" @click="addZone">{{ t('statusNginx.addZone') }}</el-button>
        </div>
        <el-card v-for="(z, idx) in cacheZoneList" :key="idx" shadow="never" class="zone-card">
          <template #header>
            <div class="zone-head">
              <span class="mono">proxy_cache_path · {{ z.name || `#${idx + 1}` }}</span>
              <el-button link type="danger" :icon="Delete" @click="rm(http.proxy_cache, idx)" />
            </div>
          </template>
          <el-form label-width="190px" class="row-form">
            <el-form-item label="name">
              <el-input v-model="z.name" class="mono" />
            </el-form-item>
            <el-form-item label="path">
              <el-input v-model="z.path" class="mono" />
            </el-form-item>
            <el-form-item label="levels">
              <el-input v-model="z.levels" class="mono" />
            </el-form-item>
            <el-form-item label="keys_zone">
              <el-input v-model="z.keys_zone_size" class="mono" />
            </el-form-item>
            <el-form-item label="max_size">
              <el-input v-model="z.max_size" class="mono" />
            </el-form-item>
            <el-form-item label="inactive">
              <el-input v-model="z.inactive" class="mono" />
            </el-form-item>
            <el-form-item label="use_temp_path">
              <TriBool v-model="z.use_temp_path" />
            </el-form-item>
            <el-form-item label="loader_files">
              <el-input-number
                :model-value="Number(z.loader_files || 0)"
                :min="0"
                controls-position="right"
                @update:model-value="(v: number) => (z.loader_files = v)"
              />
            </el-form-item>
            <el-form-item label="loader_sleep">
              <el-input v-model="z.loader_sleep" class="mono" />
            </el-form-item>
            <el-form-item label="loader_threshold">
              <el-input v-model="z.loader_threshold" class="mono" />
            </el-form-item>
            <el-form-item label="manager_files">
              <el-input-number
                :model-value="Number(z.manager_files || 0)"
                :min="0"
                controls-position="right"
                @update:model-value="(v: number) => (z.manager_files = v)"
              />
            </el-form-item>
            <el-form-item label="extra">
              <el-input v-model="z.extra" class="mono" placeholder="loader_files=100 ..." />
            </el-form-item>
          </el-form>
        </el-card>
        <el-empty v-if="!http.proxy_cache.length" :description="t('statusNginx.noZone')" :image-size="70" />

        <el-divider content-position="left">ngx cache purge</el-divider>
        <el-form label-width="190px" class="row-form">
          <el-form-item :label="t('statusNginx.purgeEnable')">
            <el-switch v-model="http.cache_purge.enabled" />
          </el-form-item>
          <el-form-item :label="t('statusNginx.purgeIps')">
            <el-select v-model="http.cache_purge.allowed_ips" multiple filterable allow-create default-first-option class="grow mono">
              <el-option v-for="ip in http.cache_purge.allowed_ips" :key="ip" :label="ip" :value="ip" />
            </el-select>
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- ── 安全 / ModSecurity ── -->
      <el-tab-pane :label="t('statusNginx.tabSecurity')" name="sec">
        <el-form label-width="230px" class="row-form">
          <el-form-item label="modsecurity">
            <TriBool v-model="http.security.modsecurity" />
          </el-form-item>
          <el-form-item label="modsecurity_rules_file">
            <el-input v-model="http.security.modsecurity_rules_file" class="mono" />
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- ── 包含 / 自定义 ── -->
      <el-tab-pane :label="t('statusNginx.tabInclude')" name="inc">
        <el-form label-width="230px" class="row-form">
          <el-form-item :label="t('statusNginx.confKeep')">
            <span class="mono keep">{{ CONF_D }}</span>
          </el-form-item>
          <el-form-item :label="t('statusNginx.topIncludes')">
            <el-select
              v-model="spec.top_includes"
              multiple
              filterable
              allow-create
              default-first-option
              class="grow mono"
            >
              <el-option v-for="p in spec.top_includes || []" :key="p" :label="p" :value="p" />
            </el-select>
          </el-form-item>
        </el-form>
        <el-divider content-position="left">{{ t('statusNginx.customRaw') }}</el-divider>
        <el-input v-model="http.custom_raw" type="textarea" :rows="6" class="mono" />
        <el-divider content-position="left">{{ t('statusNginx.preserved') }}</el-divider>
        <template v-if="preserved.length">
          <el-tag v-for="(l, i) in preserved" :key="i" type="warning" class="preserved-tag mono">{{ l }}</el-tag>
          <div class="tip mt-2">{{ t('statusNginx.preservedTip') }}</div>
        </template>
        <el-empty v-else :description="t('statusNginx.noPreserved')" :image-size="60" />
      </el-tab-pane>
    </el-tabs>

    <!-- ── 操作栏 ── -->
    <div class="actions">
      <el-button :loading="previewing" @click="doPreview">{{ t('statusNginx.preview') }}</el-button>
      <el-button type="primary" :loading="applying" @click="doApply">{{ t('statusNginx.apply') }}</el-button>
      <el-button @click="doAdopt">{{ t('statusNginx.adopt') }}</el-button>
      <el-button @click="loadRevisions">{{ t('statusNginx.history') }}</el-button>
    </div>

    <!-- ── 预览抽屉 ── -->
    <el-drawer v-model="showPreview" :title="t('statusNginx.previewTitle')" size="60%">
      <el-alert
        v-if="previewData?.warnings?.length"
        type="warning"
        show-icon
        :closable="false"
        :title="t('statusNginx.conflictTitle')"
        class="mb-3"
      >
        <ul class="warn-list">
          <li v-for="(w, i) in previewData.warnings" :key="i" class="mono">{{ w }}</li>
        </ul>
      </el-alert>
      <el-tabs v-model="pvTab">
        <el-tab-pane :label="t('statusNginx.diffTab')" name="diff">
          <pre class="diff-box mono">{{ previewData?.diff }}</pre>
        </el-tab-pane>
        <el-tab-pane :label="t('statusNginx.contentTab')" name="content">
          <pre class="diff-box mono">{{ previewData?.content }}</pre>
        </el-tab-pane>
      </el-tabs>
    </el-drawer>

    <!-- ── 历史抽屉 ── -->
    <el-drawer v-model="showHistory" :title="t('statusNginx.history')" size="45%">
      <el-table :data="revisions" size="small">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="actor" :label="t('statusNginx.actor')" width="120" />
        <el-table-column :label="t('statusNginx.time')">
          <template #default="{ row }">{{ fmtTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column width="150">
          <template #default="{ row }">
            <el-button link size="small" @click="viewRev(row.id)">{{ t('statusNginx.view') }}</el-button>
            <el-button link size="small" type="primary" @click="doRollback(row)">
              {{ t('statusNginx.rollback') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-drawer>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete, Plus } from '@/icons'
import {
  adoptNginxGlobal,
  applyNginxGlobal,
  getNginxGlobalRevision,
  getNginxGlobalSpec,
  listNginxGlobalRevisions,
  previewNginxGlobal,
  putNginxGlobalSpec,
  rollbackNginxGlobal,
  type NginxGlobalPreviewData,
  type NginxGlobalRevision,
  type NginxGlobalSpec,
} from '@/api/serverNginx'
import DirectiveRow from './DirectiveRow.vue'
import TriBool from './TriBool.vue'

const { t } = useI18n()
const CONF_D = 'conf.d/*.conf'

// ── 字段描述：label 直接用 nginx 指令名（语言中立），保持与渲染层一一对应 ──
interface Field {
  key: string
  label: string
  type: 'text' | 'num' | 'bool' | 'area'
  tip?: string
}

/** 一条共享缓存区（对应 `proxy_cache_path`） */
interface CacheZone {
  name: string
  path: string
  levels: string
  keys_zone_size: string
  max_size: string
  inactive: string
  use_temp_path?: boolean | null
  loader_files?: number | null
  loader_sleep?: string
  loader_threshold?: string
  manager_files?: number | null
  manager_sleep?: string
  manager_threshold?: string
  extra?: string
}

const MAIN_FIELDS: Field[] = [
  { key: 'user', label: 'user', type: 'text', tip: '运行 worker 的账号' },
  { key: 'worker_processes', label: 'worker_processes', type: 'text', tip: 'auto 或 CPU 核数' },
  { key: 'worker_priority', label: 'worker_priority', type: 'num', tip: '-20 ~ 19' },
  { key: 'worker_rlimit_nofile', label: 'worker_rlimit_nofile', type: 'num' },
  { key: 'worker_shutdown_timeout', label: 'worker_shutdown_timeout', type: 'text' },
  { key: 'timer_resolution', label: 'timer_resolution', type: 'text' },
  { key: 'pid', label: 'pid', type: 'text' },
  { key: 'error_log', label: 'error_log', type: 'text', tip: '路径 + 级别，如 /var/log/nginx/error.log warn' },
  { key: 'pcre_jit', label: 'pcre_jit', type: 'bool' },
]

const EVENT_FIELDS: Field[] = [
  { key: 'worker_connections', label: 'worker_connections', type: 'num' },
  { key: 'multi_accept', label: 'multi_accept', type: 'bool' },
  { key: 'accept_mutex', label: 'accept_mutex', type: 'bool' },
  { key: 'accept_mutex_delay', label: 'accept_mutex_delay', type: 'text' },
  { key: 'use_method', label: 'use', type: 'text', tip: 'epoll / kqueue，留空让 nginx 自选' },
]

const BASIC_FIELDS: Field[] = [
  { key: 'sendfile', label: 'sendfile', type: 'bool' },
  { key: 'sendfile_max_chunk', label: 'sendfile_max_chunk', type: 'text' },
  { key: 'tcp_nopush', label: 'tcp_nopush', type: 'bool' },
  { key: 'tcp_nodelay', label: 'tcp_nodelay', type: 'bool' },
  { key: 'keepalive_timeout', label: 'keepalive_timeout', type: 'text', tip: '可带第二值：65 65' },
  { key: 'keepalive_requests', label: 'keepalive_requests', type: 'num' },
  { key: 'send_timeout', label: 'send_timeout', type: 'text' },
  { key: 'client_header_timeout', label: 'client_header_timeout', type: 'text' },
  { key: 'client_body_timeout', label: 'client_body_timeout', type: 'text' },
  { key: 'reset_timedout_connection', label: 'reset_timedout_connection', type: 'bool' },
  { key: 'client_max_body_size', label: 'client_max_body_size', type: 'text', tip: '上传大小上限，如 100m' },
  { key: 'client_body_buffer_size', label: 'client_body_buffer_size', type: 'text' },
  { key: 'large_client_header_buffers', label: 'large_client_header_buffers', type: 'text' },
  { key: 'types_hash_max_size', label: 'types_hash_max_size', type: 'num' },
  { key: 'server_names_hash_max_size', label: 'server_names_hash_max_size', type: 'num' },
  { key: 'server_names_hash_bucket_size', label: 'server_names_hash_bucket_size', type: 'num' },
  { key: 'server_tokens', label: 'server_tokens', type: 'bool', tip: '关闭可隐藏版本号' },
  { key: 'default_type', label: 'default_type', type: 'text' },
]

const GZIP_FIELDS: Field[] = [
  { key: 'enable', label: 'gzip', type: 'bool' },
  { key: 'comp_level', label: 'gzip_comp_level', type: 'num', tip: '1-9' },
  { key: 'min_length', label: 'gzip_min_length', type: 'num' },
  { key: 'types', label: 'gzip_types', type: 'area' },
  { key: 'vary', label: 'gzip_vary', type: 'bool' },
  { key: 'proxied', label: 'gzip_proxied', type: 'text' },
  { key: 'disable', label: 'gzip_disable', type: 'text' },
  { key: 'buffers', label: 'gzip_buffers', type: 'text' },
]

const TLS_FIELDS: Field[] = [
  { key: 'protocols', label: 'ssl_protocols', type: 'text' },
  { key: 'ciphers', label: 'ssl_ciphers', type: 'area' },
  { key: 'prefer_server_ciphers', label: 'ssl_prefer_server_ciphers', type: 'bool' },
  { key: 'session_cache', label: 'ssl_session_cache', type: 'text' },
  { key: 'session_timeout', label: 'ssl_session_timeout', type: 'text' },
  { key: 'session_tickets', label: 'ssl_session_tickets', type: 'bool' },
  { key: 'buffer_size', label: 'ssl_buffer_size', type: 'text' },
  { key: 'ecdh_curve', label: 'ssl_ecdh_curve', type: 'text' },
  { key: 'dhparam', label: 'ssl_dhparam', type: 'text' },
  { key: 'stapling', label: 'ssl_stapling', type: 'bool', tip: 'OCSP stapling' },
  { key: 'stapling_verify', label: 'ssl_stapling_verify', type: 'bool' },
  { key: 'trusted_certificate', label: 'ssl_trusted_certificate', type: 'text' },
  { key: 'resolver', label: 'resolver', type: 'text', tip: 'stapling 必需，如 8.8.8.8 valid=300s' },
  { key: 'resolver_timeout', label: 'resolver_timeout', type: 'text' },
]

const LOG_FIELDS: Field[] = [
  { key: 'access_log', label: 'access_log', type: 'text', tip: 'off 关闭；可带 buffer/flush 参数' },
  { key: 'error_log', label: 'error_log', type: 'text' },
  { key: 'log_format_main', label: 'log_format main', type: 'area' },
  { key: 'log_format_json', label: 'log_format json', type: 'area' },
  { key: 'open_log_file_cache', label: 'open_log_file_cache', type: 'text' },
]

const PROXY_FIELDS: Field[] = [
  { key: 'connect_timeout', label: 'proxy_connect_timeout', type: 'text' },
  { key: 'read_timeout', label: 'proxy_read_timeout', type: 'text' },
  { key: 'send_timeout', label: 'proxy_send_timeout', type: 'text' },
  { key: 'buffers', label: 'proxy_buffers', type: 'text' },
  { key: 'buffer_size', label: 'proxy_buffer_size', type: 'text' },
  { key: 'busy_buffers_size', label: 'proxy_busy_buffers_size', type: 'text' },
  { key: 'max_temp_file_size', label: 'proxy_max_temp_file_size', type: 'text' },
  { key: 'temp_file_write_size', label: 'proxy_temp_file_write_size', type: 'text' },
  { key: 'request_buffering', label: 'proxy_request_buffering', type: 'bool' },
  { key: 'buffering', label: 'proxy_buffering', type: 'bool' },
  { key: 'http_version', label: 'proxy_http_version', type: 'text' },
  { key: 'next_upstream', label: 'proxy_next_upstream', type: 'text' },
]

const loading = ref(false)
const previewing = ref(false)
const applying = ref(false)
const tab = ref('main')
const pvTab = ref('diff')
const showPreview = ref(false)
const showHistory = ref(false)
const previewData = ref<NginxGlobalPreviewData | null>(null)
const revisions = ref<NginxGlobalRevision[]>([])
const seeded = ref(false)

// 初始值也必须过 ensureShape：el-tabs 首帧就渲染全部 pane，
// 若 http.security / proxy_cache 等缺壳，会在接口返回前直接抛 TypeError
const spec = ref<NginxGlobalSpec>(
  ensureShape({
    ver: 1,
    general: {},
    events: {},
    http: {},
    load_modules: [],
    top_includes: [],
  }),
)

const http = computed(() => spec.value.http as Record<string, any>)
// ensureShape 已保证对象壳，模板里直接当对象用
const general = computed(() => spec.value.general as Record<string, any>)
const events = computed(() => spec.value.events as Record<string, any>)
/** 缓存区列表（强类型迭代，避免 v-for 索引参与类型推导） */
const cacheZoneList = computed(() => (http.value.proxy_cache || []) as CacheZone[])
const preserved = computed(() => [
  ...(spec.value.preserve_top_raw || []),
  ...(spec.value.preserve_http_raw || []),
])

function ensureShape(s: NginxGlobalSpec) {
  // 后端返回的是 Option 结构，前端需要对象壳以免 v-model 崩
  s.general ??= {}
  s.events ??= {}
  const h = (s.http ??= {}) as Record<string, any>
  h.basic ??= {}
  h.gzip ??= {}
  h.tls ??= {}
  h.logging ??= {}
  h.proxy ??= {}
  h.proxy_cache ??= []
  h.cache_purge ??= { enabled: false, allowed_ips: [] }
  h.security ??= {}
  h.custom_raw ??= ''
  s.load_modules ??= []
  s.top_includes ??= []
  return s
}

async function load() {
  loading.value = true
  try {
    const res = await getNginxGlobalSpec()
    spec.value = ensureShape(res.data.spec)
    seeded.value = res.data.seeded
  } catch {
    // nginx 未安装 / 探测失败时不阻断本页渲染，退回空模型
    spec.value = ensureShape({ ver: 1 })
  } finally {
    loading.value = false
  }
}

function rm(arr: unknown[], i: number) {
  arr.splice(i, 1)
}

function addZone() {
  http.value.proxy_cache.push({
    name: '',
    path: '',
    levels: '1:2',
    keys_zone_size: '32m',
    max_size: '1g',
    inactive: '60m',
    use_temp_path: false,
    extra: '',
  })
}

async function doPreview() {
  previewing.value = true
  try {
    const res = await previewNginxGlobal(spec.value)
    previewData.value = res.data
    pvTab.value = res.data.warnings?.length ? 'diff' : 'diff'
    showPreview.value = true
  } finally {
    previewing.value = false
  }
}

async function doApply() {
  // 先预览拿冲突告警，有重复指令时让用户确认
  const pv = await previewNginxGlobal(spec.value)
  if (pv.data.warnings?.length) {
    try {
      await ElMessageBox.confirm(
        `${t('statusNginx.conflictWarn')}\n\n${pv.data.warnings.join('\n')}`,
        t('common.tip'),
        { type: 'warning', dangerouslyUseHTMLString: false },
      )
    } catch {
      return
    }
  }
  applying.value = true
  try {
    const res = await applyNginxGlobal(spec.value)
    ElMessage.success(res.data.reloaded ? t('statusNginx.applyOkReloaded') : t('statusNginx.applyOk'))
    previewData.value = null
  } finally {
    applying.value = false
  }
}

async function doAdopt() {
  try {
    await ElMessageBox.confirm(t('statusNginx.adoptConfirm'), t('common.tip'), { type: 'warning' })
  } catch {
    return
  }
  const res = await adoptNginxGlobal()
  spec.value = ensureShape(res.data.spec)
  ElMessage.success(t('statusNginx.adoptOk'))
}

async function loadRevisions() {
  const res = await listNginxGlobalRevisions()
  revisions.value = res.data.items
  showHistory.value = true
}

async function viewRev(id: number) {
  const res = await getNginxGlobalRevision(id)
  previewData.value = { content: res.data.content, diff: '', warnings: [], conf_file: '' }
  pvTab.value = 'content'
  showPreview.value = true
}

async function doRollback(row: NginxGlobalRevision) {
  try {
    await ElMessageBox.confirm(t('statusNginx.rollbackConfirm'), t('common.tip'), { type: 'warning' })
  } catch {
    return
  }
  await rollbackNginxGlobal(row.id)
  ElMessage.success(t('statusNginx.rollbackOk'))
  showHistory.value = false
}

async function saveDraft() {
  await putNginxGlobalSpec(spec.value)
}

function fmtTime(ts: number) {
  return new Date(ts * 1000).toLocaleString()
}

defineExpose({ saveDraft })

onMounted(load)
</script>

<style scoped>
.nginx-global {
  padding: 4px;
}
.row-form {
  max-width: 760px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
  font-size: 12px;
}
.grow {
  flex: 1;
}
.mb-3 {
  margin-bottom: 12px;
}
.mt-2 {
  margin-top: 8px;
}
.list-row {
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 760px;
  margin-bottom: 6px;
}
.zone-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  max-width: 760px;
  margin-bottom: 8px;
}
.zone-card {
  max-width: 760px;
  margin-bottom: 10px;
}
.zone-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.keep {
  color: var(--el-color-primary);
}
.preserved-tag {
  display: block;
  width: fit-content;
  max-width: 100%;
  margin-bottom: 4px;
  white-space: pre-wrap;
}
.actions {
  display: flex;
  gap: 10px;
  margin-top: 14px;
}
.diff-box {
  max-height: 60vh;
  overflow: auto;
  padding: 10px;
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  background: var(--el-fill-color-light);
  border-radius: 4px;
  white-space: pre-wrap;
  word-break: break-all;
}
.warn-list {
  margin: 4px 0 0;
  padding-left: 18px;
}
</style>
