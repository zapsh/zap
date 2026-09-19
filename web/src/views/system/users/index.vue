<template>
  <div class="users-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span class="page-title">{{ pageTitle }}</span>
          <!-- 搜索 + 新增：统一放在卡片右上角 -->
          <div class="head-right">
            <el-input
              v-model="searchForm.username"
              :placeholder="t('common.inputPlaceholder', { field: t('users.username') })"
              clearable
              style="width: 180px"
              @keyup.enter="handleSearch"
              @clear="handleSearch"
            />
            <el-button @click="handleSearch">{{ t('common.search') }}</el-button>
            <el-button @click="resetSearch">{{ t('common.reset') }}</el-button>
            <el-button type="primary" @click="handleAdd">
              <el-icon><Plus /></el-icon>{{ isAdmin ? t('users.addUser') : t('users.addReseller') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-table
        ref="tableRef"
        :data="filteredData"
        v-loading="loading"
        stripe
        :row-class-name="rowClassName"
        @row-click="onRowClick"
        @expand-change="onExpandChange"
      >
        <!-- 行展开：用户详情（运行实体 / 用量 / 权限）与名下站点列表。
             本列只承载展开内容，自带箭头由 CSS 隐藏，改用用户名列里的折叠按钮控制 -->
        <el-table-column type="expand" width="1">
          <template #default="{ row }">
            <div class="user-detail">
              <div class="detail-grid">
                <!-- 运行实体：家目录 / 系统账号 / FPM 规格 -->
                <div class="detail-card">
                  <div class="card-title">{{ t('users.detailRuntime') }}</div>
                  <div class="kv">
                    <span class="k">{{ t('users.homeDir') }}</span>
                    <code v-if="row.home_dir" class="v">{{ row.home_dir }}</code>
                    <span v-else class="muted">{{ t('users.notSet') }}</span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.linuxUser') }}</span>
                    <code v-if="row.linux_user" class="v">{{ row.linux_user }}</code>
                    <span v-else class="muted">{{ t('users.emptyValue') }}</span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.fpmSpec') }}</span>
                    <span class="v">
                      <el-tag
                        v-if="fpmSpecKind(row) === 'default'"
                        size="small"
                        type="info"
                        effect="plain"
                      >
                        {{ t('users.fpmDefault') }}
                      </el-tag>
                      <el-tag
                        v-else-if="fpmSpecKind(row) === 'inherit'"
                        size="small"
                        type="success"
                      >
                        {{ t('users.fpmInheritTag') }}
                      </el-tag>
                      <el-tag v-else-if="fpmSpecKind(row) === 'custom'" size="small" type="warning">
                        {{ t('users.fpmCustomJson') }}
                      </el-tag>
                      <span v-else class="spec-name">{{ row.fpm_spec_ref }}</span>
                    </span>
                  </div>
                </div>

                <!-- 资源用量：家目录磁盘 / 站点合计 / 本月流量 -->
                <div class="detail-card">
                  <div class="card-title">{{ t('users.detailUsage') }}</div>
                  <div class="kv">
                    <span class="k">{{ t('users.homeDisk') }}</span>
                    <span class="v strong">{{ fmtSize(row.disk_used_bytes) }}</span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.siteDiskTotal') }}</span>
                    <span class="v">{{ fmtSize(sitesDiskTotal(row)) }}</span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.bandwidth') }}</span>
                    <span class="v">
                      {{ fmtSize(row.bandwidth_used_bytes) }}
                      <span v-if="row.bandwidth_period" class="muted">
                        （{{ row.bandwidth_period }}）
                      </span>
                    </span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.statAt') }}</span>
                    <span class="v muted">{{ fmtTime(row.disk_stat_at) }}</span>
                  </div>
                </div>

                <!-- 账号信息：归属 / 登录 / 时间 -->
                <div class="detail-card">
                  <div class="card-title">{{ t('users.detailAccount') }}</div>
                  <div class="kv">
                    <span class="k">{{ t('users.owner') }}</span>
                    <span class="v">
                      <el-tag v-if="!row.owner_id" size="small" type="info">
                        {{ t('users.ownerSystemTag') }}
                      </el-tag>
                      <el-tag v-else size="small">{{ ownerName(row.owner_id) }}</el-tag>
                    </span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.lastLogin') }}</span>
                    <span class="v">
                      {{
                        row.last_login_time ? fmtTime(row.last_login_time) : t('users.emptyValue')
                      }}
                      <span v-if="row.last_login_ip" class="muted">
                        · {{ row.last_login_ip }}
                      </span>
                    </span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('common.createdAt') }}</span>
                    <span class="v">{{ fmtTime(row.created_at) }}</span>
                  </div>
                  <div class="kv">
                    <span class="k">{{ t('users.package') }}</span>
                    <span class="v">
                      <el-tag v-if="row.package_name" size="small" type="primary" effect="plain">
                        {{ row.package_name }}
                      </el-tag>
                      <span v-else class="muted">{{ t('users.emptyValue') }}</span>
                    </span>
                  </div>
                </div>
              </div>

              <!-- 名下站点：与站点列表同构的精简表格 -->
              <div class="site-block">
                <div class="block-head">
                  <span class="block-title">{{ t('users.siteList') }}</span>
                  <el-tag size="small" effect="plain">
                    {{ t('users.siteCount', { count: row.sites?.length ?? 0 }) }}
                  </el-tag>
                  <el-button link type="primary" @click="goSiteManage">
                    {{ t('users.goSiteManage') }}
                  </el-button>
                </div>
                <el-table :data="row.sites ?? []" size="small" stripe>
                  <el-table-column prop="name" :label="t('site.colName')" min-width="140" />
                  <el-table-column :label="t('common.status')" width="80">
                    <template #default="{ row: s }">
                      <el-tag size="small" :type="s.status === 1 ? 'success' : 'danger'">
                        {{ s.status === 1 ? t('common.enable') : t('common.disable') }}
                      </el-tag>
                    </template>
                  </el-table-column>
                  <el-table-column :label="t('users.phpChannel')" min-width="140">
                    <template #default="{ row: s }">
                      <code v-if="s.php_instance" class="v">{{ s.php_instance }}</code>
                      <span v-else class="muted">{{ t('users.emptyValue') }}</span>
                    </template>
                  </el-table-column>
                  <el-table-column :label="t('users.diskUsage')" width="110">
                    <template #default="{ row: s }">{{ fmtSize(s.disk_used_bytes) }}</template>
                  </el-table-column>
                  <el-table-column
                    prop="remark"
                    :label="t('users.remark')"
                    min-width="140"
                    show-overflow-tooltip
                  >
                    <template #default="{ row: s }">
                      <span class="muted">{{ s.remark || t('users.emptyValue') }}</span>
                    </template>
                  </el-table-column>
                  <el-table-column :label="t('common.createdAt')" width="170">
                    <template #default="{ row: s }">{{ fmtTime(s.created_at) }}</template>
                  </el-table-column>
                  <template #empty>
                    <span class="muted">{{ t('users.noSite') }}</span>
                  </template>
                </el-table>
              </div>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column :label="t('users.username')" min-width="170">
          <template #default="{ row }">
            <div class="name-cell">
              <!-- 折叠/展开按钮合并进用户名列（展开列本身已隐藏） -->
              <span
                class="expander"
                :class="{ 'is-open': isExpanded(row) }"
                :title="isExpanded(row) ? t('common.collapse') : t('common.expand')"
                @click.stop="toggleExpand(row)"
              >
                <el-icon><ArrowRight /></el-icon>
              </span>
              <span class="user-name">{{ row.username }}</span>
              <el-tag
                v-if="row.user_kind === 1"
                size="small"
                type="warning"
                effect="plain"
                class="kind-tag"
              >
                {{ t('users.kindMember') }}
              </el-tag>
              <el-tag v-if="row.read_only" size="small" type="info" effect="plain">
                {{ t('users.readOnly') }}
              </el-tag>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="nickname" :label="t('users.nickname')" width="120" />
        <el-table-column prop="email" :label="t('users.email')" min-width="180" />
        <el-table-column :label="t('users.roles')" width="120">
          <template #default="{ row }">
            <el-tag v-for="r in row.roles" :key="r" size="small" style="margin-right: 4px">
              {{ roleLabel(r) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('users.diskUsage')" width="170">
          <template #default="{ row }">
            <el-tooltip :content="diskTip(row)" placement="top">
              <span class="disk-cell">
                {{ fmtSize(row.disk_used_bytes) }}
                <el-tag v-if="!row.disk_stat_at" size="small" type="info" effect="plain">
                  {{ t('users.notCollected') }}
                </el-tag>
              </span>
            </el-tooltip>
          </template>
        </el-table-column>
        <el-table-column :label="t('users.sites')" width="110">
          <template #default="{ row }">
            <el-tag size="small" effect="plain" :type="row.sites?.length ? 'primary' : 'info'">
              {{ row.sites?.length ?? 0 }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('users.package')" width="140">
          <template #default="{ row }">
            <el-tag v-if="row.package_name" size="small" type="primary" effect="plain">
              {{ row.package_name }}
            </el-tag>
            <span v-else class="muted">{{ t('users.emptyValue') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.status')" width="80">
          <template #default="{ row }">
            <el-tag :type="row.status === 1 ? 'success' : 'danger'" size="small">
              {{ row.status === 1 ? t('common.enable') : t('common.disable') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="220" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link :disabled="isRootLocked(row)" @click="handleEdit(row)">{{
              t('common.edit')
            }}</el-button>
            <el-button
              :type="row.status === 1 ? 'warning' : 'success'"
              link
              :disabled="row.id === ROOT_USER_ID"
              @click="handleToggleStatus(row)"
            >
              {{ row.status === 1 ? t('common.disable') : t('common.enable') }}
            </el-button>
            <el-tooltip
              :disabled="row.id !== ROOT_USER_ID"
              :content="t('users.rootProtected')"
              placement="top"
            >
              <span>
                <el-button
                  type="danger"
                  link
                  :disabled="row.id === ROOT_USER_ID"
                  @click="handleDelete(row)"
                >
                  {{ t('common.delete') }}
                </el-button>
              </span>
            </el-tooltip>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新增 / 编辑 对话框 -->
    <el-dialog
      v-model="dialogVisible"
      :title="
        dialogType === 'add'
          ? isAdmin
            ? t('users.addUser')
            : t('users.addReseller')
          : t('common.edit')
      "
      width="480px"
      @closed="resetForm"
    >
      <el-form ref="formRef" :model="form" :rules="rules" label-width="70px" @submit.prevent>
        <el-form-item :label="t('users.username')" prop="username">
          <el-input v-model="form.username" :disabled="dialogType === 'edit'" />
        </el-form-item>
        <el-form-item :label="t('users.nickname')" prop="nickname">
          <el-input v-model="form.nickname" :placeholder="t('users.nicknameTip')" clearable />
        </el-form-item>
        <el-form-item :label="t('users.email')" prop="email">
          <el-input v-model="form.email" />
        </el-form-item>
        <el-form-item v-if="dialogType === 'add'" :label="t('users.password')" prop="password">
          <el-input v-model="form.password" type="password" show-password />
        </el-form-item>
        <el-form-item v-if="isAdmin" :label="t('users.roles')" prop="roles">
          <el-select v-model="form.roles" :disabled="editingId === ROOT_USER_ID">
            <el-option
              v-for="opt in roleOptions()"
              :key="opt.value"
              :label="opt.label"
              :value="opt.value"
            />
          </el-select>
        </el-form-item>
        <el-form-item v-if="isAdmin" :label="t('users.extraPerm')">
          <el-select
            v-model="form.permissions"
            multiple
            filterable
            clearable
            collapse-tags
            collapse-tags-tooltip
            :placeholder="t('users.extraPermPlaceholder')"
            style="width: 100%"
          >
            <!-- 权限点文案按 ns / action 标识符翻译，见 utils/perm.ts -->
            <el-option-group v-for="g in permCatalog" :key="g.ns" :label="permGroupLabel(g.ns)">
              <el-option
                v-for="a in g.actions"
                :key="a.key"
                :label="permKeyLabel(a.key)"
                :value="a.key"
              />
            </el-option-group>
          </el-select>
          <div class="form-tip">{{ t('users.extraPermTip') }}</div>
        </el-form-item>
        <!-- 用户类型：成员共享归属用户的家目录与 Linux 系统账号，权限默认继承父账号 -->
        <el-form-item v-if="dialogType === 'add'" :label="t('users.kind')">
          <el-radio-group v-model="form.user_kind">
            <el-radio :value="0">{{ t('users.kindCustomer') }}</el-radio>
            <el-radio :value="1">{{ t('users.kindMember') }}</el-radio>
          </el-radio-group>
          <div v-if="form.user_kind === 1" class="form-tip">
            {{
              t('users.kindMemberTip', {
                owner: form.owner_id ? ownerName(form.owner_id) : t('users.ownerSystem'),
              })
            }}
          </div>
        </el-form-item>
        <el-form-item v-if="form.user_kind === 1" :label="t('users.denyPerm')">
          <el-select
            v-model="form.perm_deny"
            multiple
            filterable
            clearable
            collapse-tags
            collapse-tags-tooltip
            :placeholder="t('users.denyPermPlaceholder')"
            style="width: 100%"
          >
            <el-option-group v-for="g in permCatalog" :key="g.ns" :label="permGroupLabel(g.ns)">
              <el-option
                v-for="a in g.actions"
                :key="a.key"
                :label="permKeyLabel(a.key)"
                :value="a.key"
              />
            </el-option-group>
          </el-select>
          <div class="form-tip">{{ t('users.denyPermTip') }}</div>
        </el-form-item>
        <!-- 只读：共享可见但不可改（后端收敛为只保留 {ns}:view 权限点） -->
        <el-form-item v-if="form.user_kind === 1" :label="t('users.readOnly')">
          <el-switch v-model="form.read_only" />
          <div class="form-tip">{{ t('users.readOnlyTip') }}</div>
        </el-form-item>
        <el-form-item v-if="isAdmin && dialogType === 'add'" :label="t('users.owner')">
          <el-select v-model="form.owner_id" @change="onOwnerChange">
            <el-option :label="t('users.ownerSystem')" :value="0" />
            <el-option v-for="r in resellerList" :key="r.id" :label="r.username" :value="r.id" />
          </el-select>
        </el-form-item>
        <!-- 成员共享父账号的运行实体：套餐 / FPM 规格一律跟随父账号 -->
        <el-form-item
          v-if="(isAdmin || isReseller) && form.user_kind !== 1"
          :label="t('users.fpmSpec')"
        >
          <el-select
            v-model="fpmMode"
            :loading="fpmLoading"
            :placeholder="t('users.fpmDefault')"
            style="width: 100%"
          >
            <el-option
              v-if="inheritOwnerName"
              :value="'inherit'"
              :label="t('users.fpmInheritOption', { owner: inheritOwnerName })"
            />
            <el-option
              v-for="opt in fpmOptions"
              :key="opt.value"
              :value="opt.value"
              :label="opt.label"
            />
            <el-option v-if="isAdmin" :value="'custom'" :label="t('users.fpmCustomOption')" />
          </el-select>
          <div class="form-tip">{{ fpmModeTip() }}</div>
          <el-input
            v-if="fpmMode === 'custom'"
            v-model="fpmCustomJson"
            type="textarea"
            :rows="4"
            spellcheck="false"
            style="margin-top: 8px"
            :placeholder="t('users.fpmCustomPlaceholder')"
          />
          <el-alert
            v-else-if="fpmMode === '__keep__'"
            :title="t('users.fpmKeepTip', { json: keepJsonPreview })"
            type="info"
            :closable="false"
            show-icon
            style="margin-top: 8px"
          />
        </el-form-item>
        <el-form-item v-if="form.user_kind !== 1" :label="t('users.package')">
          <el-select
            v-model="form.package_id"
            :loading="pkgLoading"
            :placeholder="t('users.packageNone')"
            style="width: 100%"
          >
            <el-option :label="t('users.packageNoneOption')" :value="0" />
            <el-option
              v-for="p in packageOptions"
              :key="p.value"
              :label="p.label"
              :value="p.value"
            />
          </el-select>
          <div class="form-tip">{{ packageTip() }}</div>
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
        <el-button type="primary" :loading="submitting" @click="submitForm">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ArrowRight, Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance, FormRules } from 'element-plus'
import {
  getUserList,
  createUser,
  updateUser,
  deleteUser,
  getResellerList,
  type UserListItem,
  type ResellerItem,
  type CreateUserPayload,
  type UpdateUserPayload,
} from '@/api/user'
import { roleLabel, roleOptions } from '@/utils/role'
import { useUserStore } from '@/stores/user'
import { getFpmSpecs, type FpmSpecItem } from '@/api/serverEnv'
import { getPackageList, type PackageItem } from '@/api/package'
import { getPermissionCatalog, type PermGroupItem } from '@/api/role'
import { permGroupLabel, permKeyLabel } from '@/utils/perm'
import { getLocale } from '@/i18n'

const { t } = useI18n()

/** 权限点目录：附加权限 / 成员收紧权限下拉用（admin 与 reseller 加载） */
const permCatalog = ref<PermGroupItem[]>([])

const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))
/** 内置初始管理员 ID：不可删除、不可禁用，仅本人可编辑 */
const ROOT_USER_ID = 1
/** 内置管理员行是否被当前登录者锁定（非本人不可编辑） */
const isRootLocked = (row: UserListItem) =>
  row.id === ROOT_USER_ID && userStore.userInfo.id !== ROOT_USER_ID
const isReseller = computed(() => userStore.roles.includes('reseller'))
const pageTitle = computed(() => t(isReseller.value ? 'users.titleReseller' : 'users.title'))

// ── 搜索 ───────────────────────────────────────────────────
const searchForm = reactive({ username: '' })

// ── 表格 ───────────────────────────────────────────────────
const loading = ref(false)
const tableData = ref<UserListItem[]>([])
const resellerList = ref<ResellerItem[]>([])

async function loadList() {
  loading.value = true
  try {
    const res = await getUserList({ username: searchForm.username || undefined })
    tableData.value = res.data ?? []
  } catch {
    // 拦截器已弹窗
  } finally {
    loading.value = false
  }
}

/**
 * 列表过滤：后端 /system/user/list 暂不支持关键字过滤，这里按用户名 / 昵称 / 邮箱本地过滤，
 * 用户输入即时生效（点「搜索」或回车会顺带刷新一次列表）。
 */
const filteredData = computed(() => {
  const kw = searchForm.username.trim().toLowerCase()
  if (!kw) return tableData.value
  return tableData.value.filter((u) =>
    [u.username, u.nickname, u.email].some((f) => (f ?? '').toLowerCase().includes(kw)),
  )
})

// ── 行展开：当前展开行高亮；站点入口跳到站点管理 ────────────
const router = useRouter()
const tableRef = ref<{ toggleRowExpansion: (row: UserListItem, expanded?: boolean) => void }>()
const expandedIds = ref<Set<number>>(new Set())

function onExpandChange(_row: UserListItem, rows: UserListItem[]) {
  expandedIds.value = new Set(rows.map((r) => r.id))
}

/** 单击整行展开/收起详情：展开列与操作列（右侧固定）交给自身处理 */
function onRowClick(row: UserListItem, column?: { type?: string; fixed?: string | boolean }) {
  if (column?.type === 'expand' || column?.fixed === 'right' || column?.fixed === true) return
  tableRef.value?.toggleRowExpansion(row)
}

function isExpanded(row: UserListItem) {
  return expandedIds.value.has(row.id)
}

function toggleExpand(row: UserListItem) {
  tableRef.value?.toggleRowExpansion(row)
}

function rowClassName({ row }: { row: UserListItem }) {
  return expandedIds.value.has(row.id) ? 'row-expanded' : ''
}

function goSiteManage() {
  router.push('/site/index')
}

async function loadResellers() {
  if (!isAdmin.value) return
  try {
    const res = await getResellerList()
    resellerList.value = res.data ?? []
  } catch {
    // 拦截器已弹窗
  }
}

function ownerName(ownerId: number) {
  const r = resellerList.value.find((x) => x.id === ownerId)
  return r ? r.username : `#${ownerId}`
}

// ── PHP-FPM 规格模板选择 ────────────────────────────────────

/** 规格模板列表（后端：admin 全量；reseller 仅自己名下 + 全局） */
const specs = ref<FpmSpecItem[]>([])
const fpmLoading = ref(false)

async function loadSpecs() {
  fpmLoading.value = true
  try {
    const res = await getFpmSpecs()
    specs.value = res.data ?? []
  } catch {
    // 拦截器已弹窗
  } finally {
    fpmLoading.value = false
  }
}

/**
 * 当前归属者用户名（决定可选的"名下模板"与"继承"目标）。
 * - add：admin 按归属下拉；reseller 为本人
 * - edit：按被编辑用户的 owner_id（reseller 场景 owner 为本人）
 * 系统直属（owner_id=0 且 admin）返回空串 → 只有全局通用模板可用，无"继承"。
 */
function targetResellerName(): string {
  if (dialogType.value === 'edit') {
    const row = editingId.value ? tableData.value.find((r) => r.id === editingId.value) : undefined
    if (!row) return ''
    if (!isAdmin.value) return userStore.name
    if (!row.owner_id) return ''
    const n = ownerName(row.owner_id)
    return n.startsWith('#') ? '' : n
  }
  // add
  if (!isAdmin.value) return userStore.name
  const owner = form.owner_id
  if (!owner) return ''
  const n = ownerName(owner)
  return n.startsWith('#') ? '' : n
}

/** 目标归属者名下模板（无前缀的全局模板始终可见可选） */
const targetTemplates = computed(() => {
  const owner = targetResellerName()
  return specs.value.filter((s) => !s.owner || (owner && s.owner === owner))
})

const inheritOwnerName = computed(() => targetResellerName())

/** 下拉选项：面板默认 → 可用模板（全局 + 名下） */
const fpmOptions = computed(() => [
  { value: '', label: t('users.fpmDefaultOption') },
  ...targetTemplates.value.map((s) => ({
    value: s.name,
    label: s.owner
      ? t('users.fpmOwnerTemplate', { name: s.name, owner: s.owner })
      : t('users.fpmGlobalTemplate', { name: s.name }),
  })),
])

/** 编辑时旧自定义 JSON 的保留哨兵（不向后端提交，保持原值） */
const KEEP_CUSTOM = '__keep__'
/** 自定义 JSON（高级模式） */
const CUSTOM = 'custom'

/** 用户 FPM 规格引用选择：''=默认 / inherit=继承 reseller / 模板名 / __keep__ / custom */
const fpmMode = ref('')
/** 自定义 JSON 模式下的文本 */
const fpmCustomJson = ref('')

function fpmModeTip(): string {
  if (fpmMode.value === 'inherit') {
    return t('users.fpmTipInherit', { owner: inheritOwnerName.value })
  }
  if (fpmMode.value === 'custom') {
    return t('users.fpmTipCustom')
  }
  if (fpmMode.value === KEEP_CUSTOM) {
    return t('users.fpmTipKeep')
  }
  if (fpmMode.value) {
    return t('users.fpmTipTemplate')
  }
  return t('users.fpmTipDefault')
}

/** 「保留原自定义规格」提示里的 JSON 预览（超长截断，避免撑破 alert） */
const keepJsonPreview = computed(() => {
  const raw = fpmCustomJson.value
  return raw.length > 120 ? `${raw.slice(0, 120)}…` : raw
})

/** 编辑回显：根据 row 的 fpm_pool / fpm_spec_ref 计算下拉初始值 */
function fpmEditInitial(row: UserListItem): string {
  const ref = row.fpm_spec_ref ?? ''
  if (ref) return ref // '' / inherit / 模板名
  if (row.fpm_pool && row.fpm_pool.trim()) return KEEP_CUSTOM
  return ''
}

/**
 * 行 FPM 规格的**类型标识**（不是展示文案）。
 *
 * 用标识而非中文串做比较：文案要随语言变，比较不能依赖它。
 */
function fpmSpecKind(row: UserListItem): 'default' | 'inherit' | 'custom' | 'template' {
  const ref = row.fpm_spec_ref ?? ''
  if (ref === 'inherit') return 'inherit'
  if (ref) return 'template'
  if (row.fpm_pool && row.fpm_pool.trim()) return 'custom'
  return 'default'
}

// ── 套餐（Packages）选择 ────────────────────────────────────

/** 套餐列表（后端已按角色过滤：admin 全量；reseller 全局 + 自己名下） */
const packages = ref<PackageItem[]>([])
const pkgLoading = ref(false)

async function loadPackages() {
  pkgLoading.value = true
  try {
    const res = await getPackageList()
    // 停用的套餐不参与下拉选择
    packages.value = (res.data ?? []).filter((p) => p.status === 1)
  } catch {
    // 拦截器已弹窗
  } finally {
    pkgLoading.value = false
  }
}

/** 套餐摘要：磁盘 / 站点 / SSH */
function describePackage(p: PackageItem): string {
  const parts = [
    p.disk_quota_mb > 0
      ? t('users.packageDisk', { mb: p.disk_quota_mb })
      : t('users.packageDiskUnlimited'),
    p.max_sites > 0
      ? t('users.packageSites', { count: p.max_sites })
      : t('users.packageSitesUnlimited'),
    p.allow_ssh ? t('users.packageSshAllow') : t('users.packageSshDeny'),
  ]
  if (p.max_bandwidth_mb > 0) parts.push(t('users.packageBandwidth', { mb: p.max_bandwidth_mb }))
  return parts.join(' · ')
}

const packageOptions = computed(() =>
  packages.value.map((p) => ({ value: p.id, label: `${p.name}（${describePackage(p)}）` })),
)

/** 当前所选套餐的提示文案 */
function packageTip(): string {
  if (!form.package_id) return t('users.packageTipNone')
  const p = packages.value.find((x) => x.id === form.package_id)
  if (!p) return t('users.packageTipMissing')
  return t('users.packageTipInherit', {
    summary: `${describePackage(p)}${p.fpm_spec_ref ? ` · FPM ${p.fpm_spec_ref}` : ''}`,
  })
}

/** 校验自定义 FPM JSON（空 = 不允许，自定义模式必须填对象） */
function fpmCustomValid(raw: string): boolean {
  const v = raw.trim()
  if (!v) return false
  try {
    const obj = JSON.parse(v)
    return typeof obj === 'object' && obj !== null && !Array.isArray(obj)
  } catch {
    return false
  }
}

function onOwnerChange() {
  // 归属切换后：若原选择为「继承」且新归属无 reseller，则回到面板默认
  if (fpmMode.value === 'inherit' && !targetResellerName()) {
    fpmMode.value = ''
  }
}

function handleSearch() {
  loadList()
}

function resetSearch() {
  searchForm.username = ''
  loadList()
}

// ── 对话框 ─────────────────────────────────────────────────
const dialogVisible = ref(false)
const dialogType = ref<'add' | 'edit'>('add')
const submitting = ref(false)
const formRef = ref<FormInstance>()
const editingId = ref<number | null>(null)

interface FormData {
  username: string
  nickname: string
  email: string
  password: string
  roles: string
  owner_id: number
  status: number
  /** 用户 PHP-FPM pool 规格（JSON 字符串，空 = 面板默认） */
  fpm_pool: string
  /** 套餐 id；0 = 不绑定套餐 */
  package_id: number
  /** 个人附加权限点：在角色权限之外单独授予（只做加法） */
  permissions: string[]
  /** 用户类型：0=客户（独立家目录与系统账号）/ 1=成员（共享归属用户的家目录与系统账号） */
  user_kind: number
  /** 父账号对该成员收紧（取消）的权限点；仅成员生效 */
  perm_deny: string[]
  /** 只读账号：开启后只能查看，不能做任何修改 */
  read_only: boolean
}

const defaultForm = (): FormData => ({
  username: '',
  nickname: '',
  email: '',
  password: '',
  roles: 'user',
  owner_id: 0,
  status: 1,
  fpm_pool: '',
  package_id: 0,
  permissions: [],
  user_kind: 0,
  perm_deny: [],
  read_only: false,
})

const form = reactive<FormData>(defaultForm())

// computed：切换语言时校验提示跟着变（普通对象只在 setup 时求值一次）
const rules = computed<FormRules<FormData>>(() => ({
  username: [
    { required: true, message: t('users.usernameRequired'), trigger: 'blur' },
    { min: 2, max: 50, message: t('users.usernameLength'), trigger: 'blur' },
  ],
  nickname: [{ max: 50, message: t('users.usernameLength'), trigger: 'blur' }],
  email: [
    { required: true, message: t('users.emailRequired'), trigger: 'blur' },
    { type: 'email', message: t('users.emailInvalid'), trigger: 'blur' },
  ],
  password: [
    { required: true, message: t('users.passwordRequired'), trigger: 'blur' },
    { min: 6, message: t('users.passwordLength'), trigger: 'blur' },
  ],
}))

function handleAdd() {
  dialogType.value = 'add'
  editingId.value = null
  Object.assign(form, defaultForm())
  fpmMode.value = ''
  fpmCustomJson.value = ''
  dialogVisible.value = true
}

function handleEdit(row: UserListItem) {
  if (isRootLocked(row)) {
    ElMessage.warning(t('users.rootProtected'))
    return
  }
  dialogType.value = 'edit'
  editingId.value = row.id
  Object.assign(form, {
    username: row.username,
    nickname: row.nickname,
    email: row.email,
    password: '',
    roles: row.roles?.[0] ?? 'user',
    owner_id: row.owner_id ?? 0,
    status: row.status,
    fpm_pool: row.fpm_pool ?? '',
    package_id: row.package_id ?? 0,
    permissions: (row.permissions ?? []).filter(Boolean),
    user_kind: row.user_kind ?? 0,
    perm_deny: (row.perm_deny ?? []).filter(Boolean),
    read_only: !!row.read_only,
  })
  fpmMode.value = fpmEditInitial(row)
  fpmCustomJson.value = row.fpm_pool && row.fpm_pool.trim() ? row.fpm_pool : ''
  dialogVisible.value = true
}

/**
 * 依据当前下拉选择构造 FPM 提交字段。
 * - __keep__：保留旧自定义 JSON（不提交）
 * - custom：提交 fpm_pool（后端自动清空模板引用）
 * - 其它（'' / inherit / 模板名）：提交 fpm_spec_ref（后端自动清空旧自定义 JSON）
 * 返回 null 表示校验失败（已提示）。
 */
function resolveFpmPayload(): Record<string, string> | null {
  const m = fpmMode.value
  if (m === KEEP_CUSTOM) return {}
  if (m === CUSTOM) {
    if (!isAdmin.value) return {}
    if (!fpmCustomValid(fpmCustomJson.value)) {
      ElMessage.warning(t('users.fpmInvalidJson'))
      return null
    }
    return { fpm_pool: fpmCustomJson.value.trim() }
  }
  return { fpm_spec_ref: m }
}

function resetForm() {
  formRef.value?.resetFields()
}

async function submitForm() {
  const valid = await formRef.value?.validate().catch(() => false)
  if (!valid) return

  const fpmPayload = resolveFpmPayload()
  if (fpmPayload === null) return

  submitting.value = true
  try {
    if (dialogType.value === 'add') {
      const payload: CreateUserPayload = {
        username: form.username,
        password: form.password,
        email: form.email,
        nickname: form.nickname.trim() || form.username,
      }
      if (isAdmin.value) {
        payload.roles = form.roles
        payload.owner_id = form.owner_id || 0
        payload.permissions = form.permissions
      }
      // 成员（子账号）：共享归属用户的家目录与系统账号，套餐 / FPM 一律跟随父账号
      if (form.user_kind === 1) {
        payload.user_kind = 1
        payload.perm_deny = form.perm_deny
        payload.read_only = form.read_only
      } else {
        payload.package_id = form.package_id || 0
        if (fpmPayload.fpm_spec_ref !== undefined) {
          payload.fpm_spec_ref = fpmPayload.fpm_spec_ref
        } else if (fpmPayload.fpm_pool !== undefined) {
          payload.fpm_pool = fpmPayload.fpm_pool
        }
      }
      const res = await createUser(payload)
      ElMessage.success(
        res.data?.home_dir
          ? t('users.createSuccessHome', { dir: res.data.home_dir })
          : t('common.createSuccess'),
      )
    } else {
      const payload: UpdateUserPayload = {
        id: editingId.value!,
        email: form.email,
        nickname: form.nickname.trim() || form.username,
        status: form.status,
      }
      // 内置管理员角色不可变更：编辑本人时不下发 roles / permissions
      if (isAdmin.value && editingId.value !== ROOT_USER_ID) {
        payload.roles = form.roles
        payload.permissions = form.permissions
      }
      // 成员只下发收紧清单（套餐 / FPM 由父账号决定）
      if (form.user_kind === 1) {
        payload.perm_deny = form.perm_deny
        payload.read_only = form.read_only
      } else {
        payload.package_id = form.package_id || 0
        if (fpmPayload.fpm_spec_ref !== undefined) {
          payload.fpm_spec_ref = fpmPayload.fpm_spec_ref
        }
        if (fpmPayload.fpm_pool !== undefined) {
          payload.fpm_pool = fpmPayload.fpm_pool
        }
      }
      await updateUser(payload)
      ElMessage.success(t('common.updateSuccess'))
    }
    dialogVisible.value = false
    loadList()
  } catch (e: unknown) {
    // 业务错误（如「邮箱已存在」）由拦截器 reject，这里统一提示
    ElMessage.error((e as Error)?.message || t('error.system'))
  } finally {
    submitting.value = false
  }
}

// ── 状态切换 ───────────────────────────────────────────────
async function handleToggleStatus(row: UserListItem) {
  if (row.id === ROOT_USER_ID) {
    ElMessage.warning(t('users.rootProtected'))
    return
  }
  const newStatus = row.status === 1 ? 0 : 1
  const action = newStatus === 1 ? t('common.enable') : t('common.disable')
  try {
    await ElMessageBox.confirm(
      t('users.toggleConfirm', { action, name: row.username }),
      t('common.tip'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await updateUser({ id: row.id, status: newStatus })
    row.status = newStatus
    ElMessage.success(t('users.toggleSuccess', { action }))
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('error.system'))
  }
}

// ── 删除 ───────────────────────────────────────────────────
async function handleDelete(row: UserListItem) {
  if (row.id === ROOT_USER_ID) {
    ElMessage.warning(t('users.rootProtected'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('users.deleteConfirm', { name: row.username }),
      t('common.warning'),
      { type: 'warning', confirmButtonText: t('users.confirmDeleteBtn') },
    )
  } catch {
    return
  }
  try {
    await deleteUser(row.id)
    ElMessage.success(t('common.deleteSuccess'))
    loadList()
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('error.system'))
  }
}

// ── 工具 ───────────────────────────────────────────────────
/** 时间按当前界面语言格式化（切换语言后列表会重渲染） */
function fmtTime(ts: number) {
  if (!ts) return '-'
  return new Date(ts * 1000).toLocaleString(getLocale())
}

/** 字节数按 1024 进制换算成可读文本（与磁盘 / 流量统计口径一致） */
function fmtSize(bytes: number): string {
  const n = Number(bytes) || 0
  if (n <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), units.length - 1)
  const v = n / 1024 ** i
  // 末位单位（B）不带小数，其余保留 1～2 位够读即可
  return `${v.toFixed(i === 0 ? 0 : v >= 100 ? 0 : 1)} ${units[i]}`
}

/** 磁盘用量列的悬浮提示：精确字节 + 采集时间 */
function diskTip(row: UserListItem): string {
  const size = fmtSize(row.disk_used_bytes)
  if (!row.disk_stat_at) return t('users.diskTipNotCollected', { size })
  return t('users.diskTip', { size, bytes: row.disk_used_bytes, time: fmtTime(row.disk_stat_at) })
}

/** 名下站点磁盘用量合计（与「站点详细」逐行对齐） */
function sitesDiskTotal(row: UserListItem): number {
  return (row.sites ?? []).reduce((sum, s) => sum + (Number(s.disk_used_bytes) || 0), 0)
}

async function loadPermCatalog() {
  // 权限点目录对任意登录用户开放（后端 Required::User）：reseller 建成员时也要选收紧项
  if ((!isAdmin.value && !isReseller.value) || permCatalog.value.length) return
  try {
    const res = await getPermissionCatalog()
    permCatalog.value = res.data?.groups ?? []
  } catch {
    /* handled by interceptor */
  }
}

onMounted(() => {
  loadList()
  loadResellers()
  loadSpecs()
  loadPackages()
  loadPermCatalog()
})
</script>

<style scoped>
.users-container {
  padding: 20px;
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.page-title {
  font-size: 16px;
  font-weight: 600;
}

.head-right {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

/* ── 行展开（与站点列表同一套交互）───────────────────────── */
/* 展开列自带的箭头隐藏，改用用户名列里的折叠按钮 */
:deep(.el-table__expand-icon) {
  display: none;
}

:deep(.el-table__expanded-cell) {
  background: transparent;
  padding: 0 8px 8px !important;
}

:deep(.el-table__row.row-expanded) > td.el-table__cell {
  background: var(--el-color-primary-light-9);
}

.name-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.user-name {
  font-weight: 500;
}

.kind-tag {
  flex-shrink: 0;
}

.expander {
  display: inline-flex;
  align-items: center;
  cursor: pointer;
  color: var(--el-text-color-secondary);
  transition: transform 0.2s;
}

.expander:hover {
  color: var(--el-color-primary);
}

.expander.is-open {
  transform: rotate(90deg);
}

.disk-cell {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

/* 展开详情：概览卡 + 名下站点 */
.user-detail {
  padding: 8px 4px 4px;
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: 12px;
  margin-bottom: 14px;
}

.detail-card {
  background: var(--el-fill-color-blank);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  padding: 10px 12px;
}

.card-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 8px;
}

.kv {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 3px 0;
  font-size: 12px;
}

.kv .k {
  flex: 0 0 82px;
  color: var(--el-text-color-secondary);
}

.kv .v {
  min-width: 0;
  word-break: break-all;
}

.kv .v.strong {
  font-weight: 600;
}

code.v {
  font-family: 'SFMono-Regular', Consolas, Menlo, monospace;
  font-size: 12px;
  color: var(--el-color-primary);
  background: var(--el-fill-color-light);
  border-radius: 4px;
  padding: 1px 6px;
}

.site-block {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  padding: 10px 12px 12px;
}

.block-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}

.block-title {
  font-size: 13px;
  font-weight: 600;
}

.form-tip {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.6;
}

.muted {
  color: var(--el-text-color-placeholder);
}
</style>
