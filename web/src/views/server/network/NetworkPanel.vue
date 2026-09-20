<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Delete } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'

interface NetworkInfo {
  hostname: string
  static_hostname: string
  pretty_hostname: string
  icon_name: string
  resolv: {
    nameservers: string[]
    search: string[]
    symlink_target?: string | null
    managed: boolean
  }
}

const { t } = useI18n()

const activeTab = ref('hostname')
const info = ref<NetworkInfo | null>(null)
const loading = ref(false)

// ── Hostname ──────────────────────────────────────────────
const newHostname = ref('')
const savingHostname = ref(false)

// ── Resolver ──────────────────────────────────────────────
const nameservers = ref<string[]>([])
const searchDomains = ref<string[]>([])
const savingResolver = ref(false)

const HOSTNAME_RE = /^[a-zA-Z0-9._-]+$/

function isIP(s: string): boolean {
  // IPv4
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/
  const m = s.match(v4)
  if (m) return m.slice(1).every((n) => Number(n) <= 255)
  // IPv6（宽松校验：含冒号且不含空白）
  return s.includes(':') && !/\s/.test(s)
}

async function load() {
  loading.value = true
  try {
    const res = await http.get<{ code: number; data: NetworkInfo }>('/system/config/network')
    info.value = res.data
    newHostname.value = res.data.hostname || res.data.static_hostname || ''
    nameservers.value = (
      res.data.resolv?.nameservers?.length ? res.data.resolv.nameservers : ['']
    ).map((s) => s)
    searchDomains.value = res.data.resolv?.search?.length ? res.data.resolv.search : ['']
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

async function saveHostname() {
  const name = newHostname.value.trim()
  if (!name) {
    ElMessage.warning(t('serverNetwork.needHostname'))
    return
  }
  if (!HOSTNAME_RE.test(name) || name.length > 253) {
    ElMessage.warning(t('serverNetwork.hostnameInvalid'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('serverNetwork.hostnameConfirm', { name }),
      t('serverNetwork.confirmTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  savingHostname.value = true
  try {
    const res = await http.post<{ code: number; message: string }>(
      '/system/config/network/hostname',
      {
        hostname: name,
      },
    )
    ElMessage.success(res.message ?? t('serverNetwork.hostnameOk'))
    load()
  } catch {
    /* handled */
  } finally {
    savingHostname.value = false
  }
}

function saveResolver() {
  const ns = nameservers.value.map((s) => s.trim()).filter(Boolean)
  if (!ns.length) {
    ElMessage.warning(t('serverNetwork.needNameserver'))
    return
  }
  for (const n of ns) {
    if (!isIP(n)) {
      ElMessage.warning(t('serverNetwork.invalidNameserver', { ip: n }))
      return
    }
  }
  const search = searchDomains.value.map((s) => s.trim()).filter(Boolean)
  ElMessageBox.confirm(t('serverNetwork.resolverConfirm'), t('serverNetwork.confirmTitle'), {
    type: 'warning',
  })
    .then(async () => {
      savingResolver.value = true
      try {
        const res = await http.post<{ code: number; message: string }>(
          '/system/config/network/resolver',
          {
            nameservers: ns,
            search,
          },
        )
        ElMessage.success(res.message ?? t('serverNetwork.resolverOk'))
        load()
      } catch {
        /* handled */
      } finally {
        savingResolver.value = false
      }
    })
    .catch(() => {
      /* canceled */
    })
}

onMounted(load)
</script>

<template>
  <div class="network-container">
    <el-card v-loading="loading">
      <el-tabs v-model="activeTab">
        <!-- Hostname -->
        <el-tab-pane :label="t('serverNetwork.tabHostname')" name="hostname">
          <el-descriptions :column="2" border style="margin-bottom: 20px">
            <el-descriptions-item :label="t('serverNetwork.curHostname')">
              {{ info?.hostname || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('serverNetwork.staticHostname')">
              {{ info?.static_hostname || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('serverNetwork.prettyHostname')">
              {{ info?.pretty_hostname || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('serverNetwork.iconName')">
              {{ info?.icon_name || '-' }}
            </el-descriptions-item>
          </el-descriptions>

          <el-divider content-position="left">{{ t('serverNetwork.changeHostname') }}</el-divider>
          <el-form label-width="100px" style="max-width: 520px" @submit.prevent>
            <el-form-item :label="t('serverNetwork.newHostname')">
              <el-input
                v-model="newHostname"
                :placeholder="t('serverNetwork.hostnamePlaceholder')"
                maxlength="253"
                clearable
              />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingHostname" @click="saveHostname">
                {{ t('serverNetwork.saveHostname') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>

        <!-- Resolver -->
        <el-tab-pane :label="t('serverNetwork.tabResolver')" name="resolver">
          <el-alert
            v-if="info?.resolv?.managed"
            type="warning"
            :closable="false"
            show-icon
            :title="t('serverNetwork.managedAlertTitle')"
            :description="t('serverNetwork.managedAlertDesc')"
            style="margin-bottom: 16px"
          />
          <el-alert
            type="info"
            :closable="false"
            show-icon
            :title="t('serverNetwork.currentDnsTitle')"
            style="margin-bottom: 16px"
          >
            <div v-if="info?.resolv?.symlink_target">
              {{ t('serverNetwork.symlinkTip') }}
              <code style="word-break: break-all">{{ info.resolv.symlink_target }}</code>
            </div>
          </el-alert>

          <el-form label-width="100px" style="max-width: 620px" @submit.prevent>
            <el-form-item :label="t('serverNetwork.nameserver')">
              <div style="width: 100%">
                <div v-for="(ns, i) in nameservers" :key="i" class="row-line">
                  <el-input
                    v-model="nameservers[i]"
                    :placeholder="t('serverNetwork.nameserverPlaceholder')"
                    clearable
                  />
                  <el-button type="danger" plain @click="nameservers.splice(i, 1)">
                    <el-icon><Delete /></el-icon>
                  </el-button>
                </div>
                <el-button type="primary" plain @click="nameservers.push('')">
                  {{ t('serverNetwork.addNameserver') }}
                </el-button>
              </div>
            </el-form-item>
            <el-form-item :label="t('serverNetwork.search')">
              <div style="width: 100%">
                <div v-for="(s, i) in searchDomains" :key="i" class="row-line">
                  <el-input
                    v-model="searchDomains[i]"
                    :placeholder="t('serverNetwork.searchPlaceholder')"
                    clearable
                  />
                  <el-button type="danger" plain @click="searchDomains.splice(i, 1)">
                    <el-icon><Delete /></el-icon>
                  </el-button>
                </div>
                <el-button type="primary" plain @click="searchDomains.push('')">
                  {{ t('serverNetwork.addSearch') }}
                </el-button>
              </div>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingResolver" @click="saveResolver">
                {{ t('serverNetwork.saveResolver') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>
      </el-tabs>
    </el-card>
  </div>
</template>

<style scoped>
.network-container {
  padding: 0;
}
.row-line {
  display: flex;
  gap: 8px;
  margin-bottom: 8px;
}
.row-line .el-input {
  flex: 1;
}
code {
  background: var(--el-fill-color-light);
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 13px;
}
</style>
