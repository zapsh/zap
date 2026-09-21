<template>
  <div class="db-page">
    <!-- 页头 -->
    <div class="page-head">
      <div class="head-left">
        <h2 class="page-title">{{ t('database.title') }}</h2>
        <div class="page-sub">
          {{ t('database.summary', { count: dbList.length, size: formatSize(totalSize) }) }}
          <template v-if="prefix">
            {{ t('database.prefixFilter', { prefix }) }}
          </template>
          <template v-else>{{ t('database.adminView') }}</template>
        </div>
      </div>
      <div class="head-actions">
        <el-button :icon="Link" @click="openPhpMyAdmin">phpMyAdmin</el-button>
        <el-button :icon="User" @click="openUsers">{{ t('database.btnUsers') }}</el-button>
        <el-button :icon="Connection" @click="openRemote">{{ t('database.btnRemote') }}</el-button>
        <el-button :icon="Refresh" :loading="loadingList" @click="reload">{{
          t('common.refresh')
        }}</el-button>
      </div>
    </div>

    <!-- 数据库列表（当前页直接展示） -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-header">
          <span class="card-title">{{ t('database.listTitle') }}</span>
          <div class="header-right">
            <el-checkbox
              v-model="lightMode"
              :disabled="loadingList"
              :title="t('database.fastModeTip')"
              @change="loadList"
            >
              {{ t('database.fastMode') }}
            </el-checkbox>
            <el-button type="primary" :icon="Plus" @click="openCreate">{{
              t('database.newDb')
            }}</el-button>
          </div>
        </div>
      </template>

      <el-table
        v-loading="loadingList"
        :data="dbList"
        border
        size="small"
        :empty-text="t('database.emptyDb')"
      >
        <el-table-column
          prop="name"
          :label="t('database.colDbName')"
          min-width="220"
          show-overflow-tooltip
        >
          <template #default="{ row }">
            <span class="db-name">{{ row.name }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="charset" :label="t('database.colCharset')" width="110" />
        <el-table-column :label="t('database.colSize')" width="110" align="right">
          <template #default="{ row }">{{ formatSize(row.size) }}</template>
        </el-table-column>
        <el-table-column prop="users" :label="t('database.colUsers')" width="90" align="center" />
        <el-table-column prop="tables" :label="t('database.colTables')" width="90" align="center" />
        <el-table-column :label="t('common.operation')" width="90" fixed="right" align="center">
          <template #default="{ row }">
            <el-button link type="danger" @click="handleDrop(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新建数据库：弹窗表单（列表页「新建数据库」按钮触发，见 openCreate） -->
    <el-dialog
      v-model="createVisible"
      :title="t('database.newDb')"
      width="620px"
      :close-on-click-modal="false"
      @closed="resetCreate"
    >
      <div class="dialog-tip">
        {{ t('database.createHint1') }}
        <template v-if="prefix">{{ t('database.createHint2', { prefix }) }}</template>
      </div>

      <el-form
        ref="createFormRef"
        :model="createForm"
        :rules="createRules"
        label-width="110px"
        class="create-form"
        @submit.prevent
      >
        <el-form-item :label="t('database.colDbName')" prop="name">
          <div class="field-block">
            <el-input
              v-model="createForm.name"
              maxlength="64"
              placeholder="my_app"
              style="max-width: 420px"
            >
              <template v-if="prefix" #prepend>{{ prefix }}</template>
            </el-input>
            <div class="field-tip">
              {{ t('database.fullDbNameLabel') }}<code>{{ fullDbName || '-' }}</code>
            </div>
          </div>
        </el-form-item>

        <el-form-item :label="t('database.charset')">
          <el-select v-model="createForm.charset" style="width: 200px">
            <el-option :label="t('database.charsetRecommend')" value="utf8mb4" />
            <el-option label="utf8mb3" value="utf8mb3" />
            <el-option label="latin1" value="latin1" />
          </el-select>
        </el-form-item>

        <el-form-item :label="t('database.advancedMode')">
          <el-switch v-model="advanced" />
          <span class="field-tip inline">{{ t('database.advancedHint') }}</span>
        </el-form-item>

        <template v-if="advanced">
          <el-form-item :label="t('database.dbUser')">
            <div class="field-block">
              <el-input
                v-model="createForm.user"
                maxlength="64"
                :placeholder="userPlaceholder"
                style="max-width: 420px"
              >
                <template v-if="prefix" #prepend>{{ prefix }}</template>
              </el-input>
              <div class="field-tip">
                {{ t('database.fullUserNameLabel') }}<code>{{ fullUserName }}</code>
              </div>
            </div>
          </el-form-item>
          <el-form-item :label="t('database.password')">
            <el-input
              v-model="createForm.password"
              type="password"
              show-password
              :placeholder="t('database.passwordPlaceholder')"
              style="max-width: 420px"
            />
          </el-form-item>
          <el-form-item :label="t('database.allowHost')">
            <el-input v-model="createForm.host" placeholder="localhost" style="max-width: 220px" />
          </el-form-item>
        </template>
      </el-form>

      <template #footer>
        <el-button @click="createVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :icon="Plus" :loading="creating" @click="handleCreate">
          {{ t('database.createDb') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 服务器信息 -->
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-header">
          <span class="card-title">{{ t('database.serverInfo') }}</span>
          <el-tag :type="status?.ok ? 'success' : 'danger'" size="small">
            {{ status?.ok ? t('database.running') : t('database.unavailable') }}
          </el-tag>
        </div>
      </template>
      <el-descriptions :column="2" border size="small">
        <el-descriptions-item :label="t('database.version')">{{
          status?.version || '-'
        }}</el-descriptions-item>
        <el-descriptions-item :label="t('database.connAddr')">
          <code>{{ status?.addr || '-' }}</code>
          <span class="muted">{{ t('database.adminAccount', { user: status?.user || '-' }) }}</span>
        </el-descriptions-item>
        <el-descriptions-item label="Socket">{{ status?.socket || '-' }}</el-descriptions-item>
        <el-descriptions-item :label="t('database.visibleScope')">
          <el-tag v-if="!prefix" size="small" type="warning">{{ t('database.scopeAll') }}</el-tag>
          <el-tag v-else size="small">{{ t('database.scopePrefix', { prefix }) }}</el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="t('database.sqlMode')" :span="2">
          {{ status?.sql_mode || '-' }}
        </el-descriptions-item>
      </el-descriptions>
    </el-card>

    <!-- 创建成功：凭据只展示一次 -->
    <el-dialog v-model="credVisible" :title="t('database.credCreated')" width="560px">
      <el-alert
        type="warning"
        :closable="false"
        show-icon
        :title="t('database.credAlert')"
        style="margin-bottom: 12px"
      />
      <el-descriptions :column="1" border size="small">
        <el-descriptions-item :label="t('database.colDbName')">{{
          cred.name
        }}</el-descriptions-item>
        <el-descriptions-item :label="t('database.userName')"
          >{{ cred.user }}@{{ cred.host }}</el-descriptions-item
        >
        <el-descriptions-item :label="t('database.password')"
          ><code>{{ cred.password }}</code></el-descriptions-item
        >
      </el-descriptions>
      <template #footer>
        <el-button @click="copyCred">{{ t('database.copyCred') }}</el-button>
        <el-button type="primary" @click="credVisible = false">{{ t('database.done') }}</el-button>
      </template>
    </el-dialog>

    <!-- 数据库用户 -->
    <el-drawer v-model="usersVisible" :title="t('database.btnUsers')" size="60%">
      <el-alert
        type="info"
        :closable="false"
        show-icon
        :title="t('database.usersAlert')"
        style="margin-bottom: 12px"
      />

      <el-form :model="userForm" inline class="remote-form" @submit.prevent>
        <el-form-item :label="t('database.userName')">
          <el-input
            v-model="userForm.user"
            :placeholder="userNamePlaceholder"
            style="width: 180px"
          />
        </el-form-item>
        <el-form-item :label="t('database.password')">
          <el-input
            v-model="userForm.password"
            type="password"
            show-password
            :placeholder="t('database.pwdMin')"
            style="width: 180px"
          />
        </el-form-item>
        <el-form-item :label="t('database.allowHost')">
          <el-input v-model="userForm.host" placeholder="localhost" style="width: 140px" />
        </el-form-item>
        <el-form-item :label="t('database.grantDb')">
          <el-select
            v-model="userForm.schema"
            clearable
            filterable
            :placeholder="t('database.notSelected')"
            style="width: 180px"
          >
            <el-option v-for="d in dbList" :key="d.name" :label="d.name" :value="d.name" />
          </el-select>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :icon="Plus" :loading="creatingUser" @click="handleCreateUser">
            {{ t('database.createUser') }}
          </el-button>
        </el-form-item>
      </el-form>

      <el-table v-loading="loadingUsers" :data="userList" border size="small">
        <el-table-column prop="user" :label="t('database.colUser')" min-width="140">
          <template #default="{ row }">
            <span class="db-name">{{ row.user }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="host" :label="t('database.allowHost')" width="160" />
        <el-table-column
          prop="grants"
          :label="t('database.colGrant')"
          min-width="260"
          show-overflow-tooltip
        />
        <el-table-column :label="t('common.operation')" width="90" fixed="right">
          <template #default="{ row }">
            <el-button link type="danger" @click="handleDropUser(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
        <template #empty>{{ t('database.emptyUsers') }}</template>
      </el-table>
    </el-drawer>

    <!-- 远程访问 -->
    <el-drawer v-model="remoteVisible" :title="t('database.remoteTitle')" size="65%">
      <el-alert
        type="warning"
        :closable="false"
        show-icon
        :title="t('database.remoteAlert')"
        style="margin-bottom: 12px"
      />

      <el-form :model="remoteForm" inline class="remote-form" @submit.prevent>
        <el-form-item :label="t('database.user')">
          <el-select
            v-model="remoteForm.user"
            filterable
            allow-create
            default-first-option
            :placeholder="t('database.selectOrInput')"
            style="width: 180px"
          >
            <el-option
              v-for="u in userList"
              :key="`${u.user}@${u.host}`"
              :label="u.user"
              :value="u.user"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('database.db')">
          <el-select
            v-model="remoteForm.schema"
            :placeholder="t('database.selectDb')"
            style="width: 160px"
          >
            <el-option v-for="d in dbList" :key="d.name" :label="d.name" :value="d.name" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('database.remoteHost')">
          <el-input
            v-model="remoteForm.host"
            :placeholder="t('database.remoteHostPlaceholder')"
            style="width: 160px"
          />
        </el-form-item>
        <el-form-item :label="t('database.password')">
          <el-input
            v-model="remoteForm.password"
            type="password"
            show-password
            :placeholder="t('database.pwdWhenNewUser')"
            style="width: 160px"
          />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="granting" @click="handleGrant">{{
            t('database.grant')
          }}</el-button>
        </el-form-item>
      </el-form>

      <el-table v-loading="loadingRemote" :data="remoteList" border size="small">
        <el-table-column prop="user" :label="t('database.user')" min-width="140" />
        <el-table-column prop="host" :label="t('database.allowHost')" width="160" />
        <el-table-column
          prop="grants"
          :label="t('database.colGrant')"
          min-width="200"
          show-overflow-tooltip
        />
        <el-table-column :label="t('common.operation')" width="90" fixed="right">
          <template #default="{ row }">
            <el-button link type="danger" @click="handleRevoke(row)">{{
              t('database.revoke')
            }}</el-button>
          </template>
        </el-table-column>
        <template #empty>{{ t('database.emptyRemote') }}</template>
      </el-table>
    </el-drawer>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance, FormRules } from 'element-plus'
import { Connection, Link, Plus, Refresh, User } from '@/icons'
import { databaseApi, type DbItem, type DbStatus, type DbUser } from '@/api/database'

const { t } = useI18n()

const status = ref<DbStatus | null>(null)
const dbList = ref<DbItem[]>([])
const userList = ref<DbUser[]>([])
const remoteList = ref<DbUser[]>([])
/** 非管理员可见的库名前缀（形如 `user_`）；管理员为 null */
const prefix = ref<string | null>(null)
/** 快速模式：跳过大小时长统计 */
const lightMode = ref(false)

const loadingStatus = ref(false)
const loadingList = ref(false)
const loadingRemote = ref(false)
const loadingUsers = ref(false)

const remoteVisible = ref(false)
const usersVisible = ref(false)
/** 新建数据库弹窗 */
const createVisible = ref(false)

const totalSize = computed(() => dbList.value.reduce((sum, d) => sum + (d.size || 0), 0))
const formatSize = (bytes: number) => {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)
  return `${(bytes / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`
}

// ── 数据加载 ────────────────────────────────────────────────
async function loadStatus() {
  loadingStatus.value = true
  try {
    status.value = await databaseApi.status()
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.statusFailed'))
  } finally {
    loadingStatus.value = false
  }
}

async function loadList() {
  loadingList.value = true
  try {
    const res = await databaseApi.list(lightMode.value)
    dbList.value = res.list || []
    prefix.value = res.prefix ?? null
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.listFailed'))
  } finally {
    loadingList.value = false
  }
}

async function loadUsers() {
  loadingUsers.value = true
  try {
    userList.value = (await databaseApi.users()).list || []
  } catch {
    userList.value = []
  } finally {
    loadingUsers.value = false
  }
}

async function loadRemote() {
  loadingRemote.value = true
  try {
    remoteList.value = (await databaseApi.remoteList()).list || []
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.remoteFailed'))
  } finally {
    loadingRemote.value = false
  }
}

async function reload() {
  await Promise.all([loadStatus(), loadList(), loadUsers()])
}

onMounted(() => {
  reload()
})

// ── 页头操作 ────────────────────────────────────────────────
function openPhpMyAdmin() {
  window.open('/webapps/phpmyadmin/', '_blank')
}

function openRemote() {
  remoteVisible.value = true
  loadList()
  loadUsers()
  loadRemote()
}

/** 打开「新建数据库」弹窗：清掉上一次的输入与校验残留 */
function openCreate() {
  resetCreate()
  createVisible.value = true
  nextTick(() => createFormRef.value?.clearValidate())
}

// ── 新建数据库 ──────────────────────────────────────────────
const createFormRef = ref<FormInstance>()
const creating = ref(false)
/** 高级模式：自定义用户名 / 密码 / 允许主机 */
const advanced = ref(false)

const createForm = reactive({
  name: '',
  charset: 'utf8mb4',
  user: '',
  password: '',
  host: 'localhost',
})

const nameRule = computed(() => ({
  pattern: /^[A-Za-z0-9_-]{1,64}$/,
  message: t('database.nameRule'),
  trigger: 'blur' as const,
}))
const createRules = computed<FormRules>(() => ({
  name: [{ required: true, message: t('database.nameRequired'), trigger: 'blur' }, nameRule.value],
}))

/** 数据库名（含前缀） */
const fullDbName = computed(() => `${prefix.value || ''}${createForm.name.trim()}`)
/** 数据库用户名（含前缀）：未自定义时与库名同名 */
const fullUserName = computed(() => {
  const base =
    advanced.value && createForm.user.trim() ? createForm.user.trim() : createForm.name.trim()
  return `${prefix.value || ''}${base}`
})
const userPlaceholder = computed(() => createForm.name.trim() || t('database.userSameAsDb'))

function resetCreate() {
  createForm.name = ''
  createForm.user = ''
  createForm.password = ''
  createForm.host = 'localhost'
  advanced.value = false
  createFormRef.value?.clearValidate()
}

/** 创建结果（含一次性明文密码） */
const cred = reactive({ name: '', user: '', host: '', password: '' })
const credVisible = ref(false)

async function handleCreate() {
  const valid = await createFormRef.value?.validate().catch(() => false)
  if (!valid) return

  creating.value = true
  try {
    const res = await databaseApi.create({
      name: createForm.name.trim(),
      charset: createForm.charset,
      // 默认「库 + 同名用户 + 授权」一条龙（用户名与库名同前缀）
      create_user: true,
      user: advanced.value && createForm.user.trim() ? createForm.user.trim() : undefined,
      password: advanced.value && createForm.password ? createForm.password : undefined,
      host: advanced.value ? createForm.host.trim() || 'localhost' : 'localhost',
    })
    cred.name = res.name
    cred.user = res.user || ''
    cred.host = res.host || 'localhost'
    cred.password = res.password || ''
    // 先收创建弹窗再弹凭据：否则两个弹窗会叠在一起
    createVisible.value = false
    credVisible.value = true
    ElMessage.success(t('database.created', { name: res.name }))
    resetCreate()
    await Promise.all([loadList(), loadUsers()])
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.createFailed'))
  } finally {
    creating.value = false
  }
}

async function copyCred() {
  const text = t('database.credText', {
    name: cred.name,
    user: cred.user,
    host: cred.host,
    password: cred.password,
  })
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success(t('database.copiedCred'))
  } catch {
    ElMessage.warning(t('database.copyFailedManual'))
  }
}

// ── 删除 / 远程授权 ─────────────────────────────────────────
async function handleDrop(row: DbItem) {
  try {
    await ElMessageBox.confirm(
      t('database.dropConfirm', { name: row.name }),
      t('database.confirmDelete'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await databaseApi.drop({ name: row.name })
    ElMessage.success(t('database.deleted'))
    await Promise.all([loadList(), loadUsers()])
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.deleteFailed'))
  }
}

const remoteForm = reactive({ user: '', schema: '', host: '', password: '' })
const granting = ref(false)

async function handleGrant() {
  if (!remoteForm.user.trim() || !remoteForm.schema || !remoteForm.host.trim()) {
    ElMessage.warning(t('database.grantFillAll'))
    return
  }
  granting.value = true
  try {
    await databaseApi.remoteGrant({
      user: remoteForm.user.trim(),
      schema: remoteForm.schema,
      host: remoteForm.host.trim(),
      password: remoteForm.password || undefined,
    })
    ElMessage.success(t('database.grantOk'))
    remoteForm.password = ''
    loadRemote()
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.grantFailed'))
  } finally {
    granting.value = false
  }
}

async function handleRevoke(row: DbUser) {
  try {
    await ElMessageBox.confirm(
      t('database.revokeConfirm', { user: row.user, host: row.host }),
      t('database.confirmRevoke'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await databaseApi.remoteRevoke({ user: row.user, host: row.host })
    ElMessage.success(t('database.revoked'))
    loadRemote()
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.revokeFailed'))
  }
}

// ── 数据库用户 ─────────────────────────────────────────────
const creatingUser = ref(false)
const userForm = reactive({ user: '', password: '', host: 'localhost', schema: '' })
const userNamePlaceholder = computed(() =>
  prefix.value ? t('database.userPrefixHint', { prefix: prefix.value }) : t('database.userExample'),
)

function openUsers() {
  usersVisible.value = true
  loadList()
  loadUsers()
}

async function handleCreateUser() {
  if (!userForm.user.trim()) {
    ElMessage.warning(t('database.userNameRequired'))
    return
  }
  if (userForm.password.length < 8) {
    ElMessage.warning(t('database.pwdTooShort'))
    return
  }
  creatingUser.value = true
  try {
    const base = userForm.user.trim()
    await databaseApi.createUser({
      user: base,
      password: userForm.password,
      host: userForm.host.trim() || 'localhost',
      schema: userForm.schema || undefined,
    })
    ElMessage.success(t('database.userCreated', { user: `${prefix.value || ''}${base}` }))
    userForm.user = ''
    userForm.password = ''
    await loadUsers()
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.createUserFailed'))
  } finally {
    creatingUser.value = false
  }
}

async function handleDropUser(row: DbUser) {
  try {
    await ElMessageBox.confirm(
      t('database.dropUserConfirm', { user: row.user, host: row.host }),
      t('database.confirmDelete'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await databaseApi.dropUser({ user: row.user, host: row.host })
    ElMessage.success(t('database.deleted'))
    await loadUsers()
  } catch (e: any) {
    ElMessage.error(e?.message || t('database.deleteFailed'))
  }
}
</script>

<style scoped>
.db-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 16px;
}
.page-head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.page-title {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}
.page-sub {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.head-actions {
  display: flex;
  gap: 8px;
}
.card-header {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.card-title {
  font-size: 15px;
  font-weight: 600;
}
/* 弹窗顶部的说明行（同 element 的次级文字色） */
.dialog-tip {
  margin-bottom: 14px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--el-text-color-secondary);
}
.header-right {
  display: flex;
  align-items: center;
  gap: 12px;
}
.db-name {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
.create-form {
  max-width: 760px;
}
.field-block {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
}
.field-tip {
  font-size: 12px;
  line-height: 1.6;
  color: var(--el-text-color-secondary);
}
.field-tip.inline {
  margin-left: 10px;
}
code {
  padding: 0 4px;
  border-radius: 3px;
  background: var(--el-fill-color-light);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
.remote-form {
  margin-bottom: 12px;
}
.muted {
  margin-left: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
