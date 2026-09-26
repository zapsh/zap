<template>
  <div class="packages-page">
    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <div>
            <div class="page-title">{{ t('packages.title') }}</div>
            <div class="page-sub">{{ t('packages.subtitle') }}</div>
          </div>
          <div class="head-right">
            <el-button :icon="Refresh" circle :disabled="loading" @click="load" />
            <el-button type="primary" @click="openAdd">
              <el-icon><Plus /></el-icon>{{ t('packages.add') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-table :data="filtered" v-loading="loading" stripe>
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column :label="t('packages.name')" min-width="150">
          <template #default="{ row }">
            <span class="pkg-name">{{ row.name }}</span>
            <el-tag v-if="row.owner_id === 0" size="small" type="info" effect="plain">
              {{ t('packages.scopeGlobal') }}
            </el-tag>
            <el-tag v-else size="small" effect="plain">{{ t('packages.scopePrivate') }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.diskQuota')" width="120">
          <template #default="{ row }">
            <span>{{
              row.disk_quota_mb > 0 ? `${row.disk_quota_mb} MB` : t('packages.unlimited')
            }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.maxSites')" width="100">
          <template #default="{ row }">
            <span>{{ row.max_sites > 0 ? row.max_sites : t('packages.unlimited') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.maxDomains')" width="120">
          <template #default="{ row }">
            <span>{{ row.max_domains > 0 ? row.max_domains : t('packages.unlimited') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.maxBandwidth')" width="120">
          <template #default="{ row }">
            <span class="muted">
              {{
                row.max_bandwidth_mb > 0 ? `${row.max_bandwidth_mb} MB` : t('packages.unlimited')
              }}
            </span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.mysqlDbs')" width="100" align="center">
          <template #default="{ row }">
            <span>{{ row.max_mysql_dbs > 0 ? row.max_mysql_dbs : t('packages.unlimited') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.pgsqlDbs')" width="90" align="center">
          <template #default="{ row }">
            <span class="muted">
              {{ row.max_pgsql_dbs > 0 ? row.max_pgsql_dbs : t('packages.unlimited') }}
            </span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.ftpUsers')" width="100" align="center">
          <template #default="{ row }">
            <span class="muted">
              {{ row.max_ftp_users > 0 ? row.max_ftp_users : t('packages.unlimited') }}
            </span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.fpmSpec')" min-width="140" show-overflow-tooltip>
          <template #default="{ row }">
            <el-tag v-if="!row.fpm_spec_ref" size="small" type="info" effect="plain">
              {{ t('users.fpmDefault') }}
            </el-tag>
            <span v-else class="spec-name">{{ row.fpm_spec_ref }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.ssh')" width="90" align="center">
          <template #default="{ row }">
            <el-tag :type="row.allow_ssh ? 'success' : 'info'" size="small" effect="plain">
              {{ row.allow_ssh ? t('packages.allow') : t('packages.deny') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.proxy')" width="110" align="center">
          <template #default="{ row }">
            <el-tag :type="row.allow_proxy ? 'success' : 'info'" size="small" effect="plain">
              {{ row.allow_proxy ? t('packages.allow') : t('packages.deny') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.php')" width="90" align="center">
          <template #default="{ row }">
            <el-tag :type="row.allow_php ? 'success' : 'info'" size="small" effect="plain">
              {{ row.allow_php ? t('packages.allow') : t('packages.deny') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.waf')" width="100" align="center">
          <template #default="{ row }">
            <el-tag :type="row.allow_waf ? 'success' : 'info'" size="small" effect="plain">
              {{ row.allow_waf ? t('packages.allow') : t('packages.deny') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.docker')" width="110" align="center">
          <template #default="{ row }">
            <el-tag :type="row.allow_docker ? 'warning' : 'info'" size="small" effect="plain">
              {{ row.allow_docker ? t('packages.allow') : t('packages.deny') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.usersCount')" width="90" align="center">
          <template #default="{ row }">
            <el-tag v-if="row.users_count > 0" size="small" effect="dark" type="primary">
              {{ row.users_count }}
            </el-tag>
            <span v-else class="muted">0</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.status')" width="90">
          <template #default="{ row }">
            <el-tag :type="row.status === 1 ? 'success' : 'info'" size="small">
              {{ row.status === 1 ? t('common.enable') : t('packages.stop') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('packages.remark')" min-width="160" show-overflow-tooltip>
          <template #default="{ row }">
            <span class="muted">{{ row.remark || '—' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="180" fixed="right">
          <template #default="{ row }">
            <el-button link type="primary" @click="openEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button
              link
              :type="row.status === 1 ? 'warning' : 'success'"
              @click="toggleStatus(row)"
            >
              {{ row.status === 1 ? t('packages.stop') : t('common.enable') }}
            </el-button>
            <el-button link type="danger" @click="handleDelete(row)">
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('packages.empty')" :image-size="80" />
        </template>
      </el-table>
    </el-card>

    <!-- 新增 / 编辑 -->
    <el-dialog
      v-model="dialogVisible"
      :title="editingId ? t('packages.edit') : t('packages.add')"
      width="560px"
      @closed="resetForm"
    >
      <el-form ref="formRef" :model="form" :rules="rules" label-width="110px" @submit.prevent>
        <el-form-item :label="t('packages.name')" prop="name">
          <el-input
            v-model="form.name"
            :placeholder="t('packages.namePlaceholder')"
            maxlength="64"
          />
        </el-form-item>
        <el-form-item :label="t('packages.remark')">
          <el-input
            v-model="form.remark"
            type="textarea"
            :rows="2"
            :placeholder="t('packages.remarkPlaceholder')"
          />
        </el-form-item>

        <el-divider content-position="left">{{ t('packages.resourceLimits') }}</el-divider>

        <el-form-item :label="t('packages.diskQuota')">
          <el-switch
            v-model="unlimitedDisk"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedDisk"
            v-model="form.disk_quota_mb"
            :min="1"
            :max="10485760"
            :step="128"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedDisk" class="form-hint">{{ t('packages.diskUnit') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.maxSites')">
          <el-switch
            v-model="unlimitedSites"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedSites"
            v-model="form.max_sites"
            :min="1"
            :max="100000"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedSites" class="form-hint">{{ t('packages.sitesUnit') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.maxDomains')">
          <el-switch
            v-model="unlimitedDomains"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedDomains"
            v-model="form.max_domains"
            :min="1"
            :max="1000"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedDomains" class="form-hint">{{ t('packages.domainsUnit') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.maxBandwidth')">
          <el-switch
            v-model="unlimitedBw"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedBw"
            v-model="form.max_bandwidth_mb"
            :min="1"
            :max="10485760"
            :step="1024"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedBw" class="form-hint">{{ t('packages.bandwidthUnit') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.mysqlDbs')">
          <el-switch
            v-model="unlimitedMysql"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedMysql"
            v-model="form.max_mysql_dbs"
            :min="1"
            :max="100000"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedMysql" class="form-hint">{{ t('packages.mysqlUnit') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.pgsqlDbs')">
          <el-switch
            v-model="unlimitedPgsql"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedPgsql"
            v-model="form.max_pgsql_dbs"
            :min="1"
            :max="100000"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedPgsql" class="form-hint">{{ t('packages.pgsqlUnit') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.ftpUsers')">
          <el-switch
            v-model="unlimitedFtp"
            :active-text="t('packages.switchUnlimited')"
            :inactive-text="t('packages.switchLimited')"
          />
          <el-input-number
            v-if="!unlimitedFtp"
            v-model="form.max_ftp_users"
            :min="1"
            :max="100000"
            style="margin-left: 12px; width: 160px"
          />
          <span v-if="!unlimitedFtp" class="form-hint">{{ t('packages.ftpUnit') }}</span>
        </el-form-item>

        <el-divider content-position="left">{{ t('packages.capabilities') }}</el-divider>

        <el-form-item :label="t('packages.fpmSpec')">
          <el-select
            v-model="form.fpm_spec_ref"
            :loading="specsLoading"
            :placeholder="t('users.fpmDefault')"
            clearable
            style="width: 100%"
          >
            <el-option v-for="s in specs" :key="s.name" :label="s.name" :value="s.name" />
          </el-select>
          <div class="form-hint">{{ t('packages.fpmSpecHint') }}</div>
        </el-form-item>
        <el-form-item :label="t('packages.ssh')">
          <el-switch v-model="form.allow_ssh" />
          <span class="form-hint">{{ t('packages.sshHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.proxy')">
          <el-switch v-model="form.allow_proxy" />
          <span class="form-hint">{{ t('packages.proxyHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.php')">
          <el-switch v-model="form.allow_php" />
          <span class="form-hint">{{ t('packages.phpHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.waf')">
          <el-switch v-model="form.allow_waf" />
          <span class="form-hint">{{ t('packages.wafHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('packages.docker')">
          <el-switch v-model="form.allow_docker" />
          <span class="form-hint">{{ t('packages.dockerHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('common.status')">
          <el-radio-group v-model="form.status">
            <el-radio :value="1">{{ t('common.enable') }}</el-radio>
            <el-radio :value="0">{{ t('packages.stop') }}</el-radio>
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

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance, FormRules } from 'element-plus'
import { Plus, Refresh } from '@/icons'
import {
  createPackage,
  deletePackage,
  getPackageList,
  updatePackage,
  type PackageItem,
} from '@/api/package'
import { getFpmSpecs, type FpmSpecItem } from '@/api/serverEnv'

const { t } = useI18n()

const list = ref<PackageItem[]>([])
const loading = ref(false)
const saving = ref(false)
const keyword = ref('')

const dialogVisible = ref(false)
const editingId = ref<number | null>(null)
const formRef = ref<FormInstance>()

const form = reactive({
  name: '',
  remark: '',
  disk_quota_mb: 1024,
  max_sites: 5,
  max_domains: 10,
  max_bandwidth_mb: 10240,
  max_mysql_dbs: 10,
  max_pgsql_dbs: 10,
  max_ftp_users: 10,
  fpm_spec_ref: '',
  allow_ssh: false,
  allow_proxy: false,
  // PHP 站点默认开放（建站的主要形态）；容器默认关闭
  allow_php: true,
  allow_waf: false,
  allow_docker: false,
  status: 1,
})
// 「不限」开关：true 时该限制项提交为 0
const unlimitedDisk = ref(true)
const unlimitedSites = ref(true)
const unlimitedDomains = ref(true)
const unlimitedBw = ref(true)
const unlimitedMysql = ref(true)
const unlimitedPgsql = ref(true)
const unlimitedFtp = ref(true)

// computed：切换语言时校验提示跟着变
const rules = computed<FormRules>(() => ({
  name: [{ required: true, message: t('packages.nameRequired'), trigger: 'blur' }],
}))

const specs = ref<FpmSpecItem[]>([])
const specsLoading = ref(false)

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return list.value
  return list.value.filter(
    (i) => i.name.toLowerCase().includes(kw) || (i.remark || '').toLowerCase().includes(kw),
  )
})

async function load() {
  loading.value = true
  try {
    const res = await getPackageList()
    list.value = res.data ?? []
  } catch {
    // 拦截器已提示
  } finally {
    loading.value = false
  }
}

async function loadSpecs() {
  specsLoading.value = true
  try {
    const res = await getFpmSpecs()
    specs.value = res.data ?? []
  } catch {
    // 拦截器已提示
  } finally {
    specsLoading.value = false
  }
}

function resetForm() {
  editingId.value = null
  form.name = ''
  form.remark = ''
  form.disk_quota_mb = 1024
  form.max_sites = 5
  form.max_domains = 10
  form.max_bandwidth_mb = 10240
  form.max_mysql_dbs = 10
  form.max_pgsql_dbs = 10
  form.max_ftp_users = 10
  form.fpm_spec_ref = ''
  form.allow_ssh = false
  form.allow_proxy = false
  form.allow_php = true
  form.allow_waf = false
  form.allow_docker = false
  form.status = 1
  unlimitedDisk.value = true
  unlimitedSites.value = true
  unlimitedDomains.value = true
  unlimitedBw.value = true
  unlimitedMysql.value = true
  unlimitedPgsql.value = true
  unlimitedFtp.value = true
  formRef.value?.clearValidate()
}

function openAdd() {
  resetForm()
  dialogVisible.value = true
}

function openEdit(row: PackageItem) {
  resetForm()
  editingId.value = row.id
  form.name = row.name
  form.remark = row.remark || ''
  unlimitedDisk.value = row.disk_quota_mb <= 0
  form.disk_quota_mb = row.disk_quota_mb > 0 ? row.disk_quota_mb : 1024
  unlimitedSites.value = row.max_sites <= 0
  form.max_sites = row.max_sites > 0 ? row.max_sites : 5
  unlimitedDomains.value = (row.max_domains ?? 0) <= 0
  form.max_domains = (row.max_domains ?? 0) > 0 ? row.max_domains : 10
  unlimitedBw.value = row.max_bandwidth_mb <= 0
  form.max_bandwidth_mb = row.max_bandwidth_mb > 0 ? row.max_bandwidth_mb : 10240
  unlimitedMysql.value = (row.max_mysql_dbs ?? 0) <= 0
  form.max_mysql_dbs = row.max_mysql_dbs > 0 ? row.max_mysql_dbs : 10
  unlimitedPgsql.value = (row.max_pgsql_dbs ?? 0) <= 0
  form.max_pgsql_dbs = row.max_pgsql_dbs > 0 ? row.max_pgsql_dbs : 10
  unlimitedFtp.value = (row.max_ftp_users ?? 0) <= 0
  form.max_ftp_users = row.max_ftp_users > 0 ? row.max_ftp_users : 10
  form.fpm_spec_ref = row.fpm_spec_ref || ''
  form.allow_ssh = !!row.allow_ssh
  form.allow_proxy = !!row.allow_proxy
  form.allow_php = row.allow_php !== false
  form.allow_waf = !!row.allow_waf
  form.allow_docker = !!row.allow_docker
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
    remark: form.remark.trim(),
    disk_quota_mb: unlimitedDisk.value ? 0 : form.disk_quota_mb,
    max_sites: unlimitedSites.value ? 0 : form.max_sites,
    max_domains: unlimitedDomains.value ? 0 : form.max_domains,
    max_bandwidth_mb: unlimitedBw.value ? 0 : form.max_bandwidth_mb,
    max_mysql_dbs: unlimitedMysql.value ? 0 : form.max_mysql_dbs,
    max_pgsql_dbs: unlimitedPgsql.value ? 0 : form.max_pgsql_dbs,
    max_ftp_users: unlimitedFtp.value ? 0 : form.max_ftp_users,
    fpm_spec_ref: form.fpm_spec_ref,
    allow_ssh: form.allow_ssh,
    allow_proxy: form.allow_proxy,
    allow_php: form.allow_php,
    allow_waf: form.allow_waf,
    allow_docker: form.allow_docker,
    status: form.status,
  }
  saving.value = true
  try {
    if (editingId.value) {
      await updatePackage({ id: editingId.value, ...payload })
      ElMessage.success(t('packages.updated'))
    } else {
      await createPackage(payload)
      ElMessage.success(t('packages.created'))
    }
    dialogVisible.value = false
    await load()
  } catch {
    // 拦截器已提示
  } finally {
    saving.value = false
  }
}

async function toggleStatus(row: PackageItem) {
  const next = row.status === 1 ? 0 : 1
  try {
    await updatePackage({ id: row.id, status: next })
    ElMessage.success(next === 1 ? t('packages.enabled') : t('packages.disabled'))
    await load()
  } catch {
    // 拦截器已提示
  }
}

async function handleDelete(row: PackageItem) {
  if (row.users_count > 0) {
    ElMessage.warning(t('packages.inUse', { name: row.name, count: row.users_count }))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('packages.deleteConfirm', { name: row.name }),
      t('common.warning'),
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
    await deletePackage(row.id)
    ElMessage.success(t('packages.deleted'))
    await load()
  } catch {
    // 拦截器已提示
  }
}

onMounted(() => {
  load()
  loadSpecs()
})
</script>

<style scoped>
.packages-page {
  padding: 2px;
}
.card-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.page-title {
  font-size: 16px;
  font-weight: 600;
}
.page-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-top: 4px;
}
.head-right {
  display: flex;
  align-items: center;
  gap: 10px;
}
.pkg-name {
  font-weight: 600;
  margin-right: 6px;
}
.spec-name {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
  font-size: 12px;
}
.muted {
  color: var(--el-text-color-secondary);
}
.form-hint {
  margin-left: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
