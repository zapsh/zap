<template>
  <div class="roles-panel">
    <el-card>
      <template #header>
        <div class="card-header">
          <div class="nav-pills">
            <span class="pill" :class="{ active: statusFilter === 'all' }" @click="statusFilter = 'all'">
              {{ t('common.all') }} <b>{{ rows.length }}</b>
            </span>
            <span class="pill" :class="{ active: statusFilter === 'on' }" @click="statusFilter = 'on'">
              {{ t('common.enable') }} <b>{{ rows.filter((r) => r.status === 1).length }}</b>
            </span>
            <span class="pill" :class="{ active: statusFilter === 'off' }" @click="statusFilter = 'off'">
              {{ t('common.disable') }} <b>{{ rows.filter((r) => r.status !== 1).length }}</b>
            </span>
          </div>
          <div class="head-right">
            <el-input
              v-model="keyword"
              :placeholder="t('common.inputPlaceholder', { field: t('roles.name') })"
              clearable
              style="width: 180px"
            />
            <el-button type="primary" @click="handleAdd">
              <el-icon><Plus /></el-icon>{{ t('roles.add') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-table :data="filteredData" v-loading="loading" stripe>
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="name" :label="t('roles.name')" width="140" />
        <el-table-column :label="t('roles.roleKey')" width="170">
          <template #default="{ row }">
            <span>{{ row.role_key }}</span>
            <el-tag
              v-if="isBuiltinRole(row.role_key)"
              type="warning"
              size="small"
              style="margin-left: 6px"
            >
              {{ t('roles.builtin') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column
          prop="description"
          :label="t('common.description')"
          min-width="180"
          show-overflow-tooltip
        />
        <el-table-column :label="t('common.status')" width="80">
          <template #default="{ row }">
            <el-tag :type="row.status === 1 ? 'success' : 'danger'" size="small">
              {{ row.status === 1 ? t('common.enable') : t('common.disable') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.createdAt')" width="170">
          <template #default="{ row }">{{ fmtTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="240" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link @click="handleEdit(row)">{{
              t('common.edit')
            }}</el-button>
            <el-button type="primary" link @click="handlePermission(row)">
              {{ t('roles.permission') }}
            </el-button>
            <el-button
              v-if="!isBuiltinRole(row.role_key)"
              type="danger"
              link
              @click="handleDelete(row)"
            >
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新增 / 编辑角色 -->
    <el-drawer
      v-model="drawerVisible"
      :size="drawerSize"
      :close-on-click-modal="false"
      @closed="resetForm"
    >
      <template #header>
        <div class="drawer-head">
          <div class="avatar">{{ avatarText }}</div>
          <div class="head-main">
            <div class="head-title">
              {{ dialogType === 'add' ? t('roles.addTitle') : t('roles.editTitle') }}
            </div>
            <div class="head-sub">{{ form.name || form.role_key || t('roles.name') }}</div>
          </div>
        </div>
      </template>

      <el-form ref="formRef" :model="form" :rules="rules" label-width="80px" @submit.prevent>
        <el-form-item :label="t('roles.formName')" prop="name">
          <el-input v-model="form.name" />
        </el-form-item>
        <el-form-item :label="t('roles.formKey')" prop="role_key">
          <el-input v-model="form.role_key" :disabled="dialogType === 'edit'" />
          <div v-if="dialogType === 'edit'" class="form-tip">{{ t('roles.keyImmutable') }}</div>
        </el-form-item>
        <el-form-item :label="t('common.description')">
          <el-input v-model="form.description" type="textarea" :rows="3" />
        </el-form-item>
        <el-form-item :label="t('common.status')">
          <el-switch
            v-model="form.status"
            :active-value="1"
            :inactive-value="0"
            :active-text="t('common.enable')"
            :inactive-text="t('common.disable')"
            inline-prompt
            :disabled="dialogType === 'edit' && isBuiltinRole(form.role_key)"
          />
          <div v-if="dialogType === 'edit' && isBuiltinRole(form.role_key)" class="form-tip">
            {{ t('roles.builtinProtected') }}
          </div>
        </el-form-item>
      </el-form>

      <template #footer>
        <div class="drawer-footer">
          <el-button @click="drawerVisible = false">{{ t('common.cancel') }}</el-button>
          <el-button type="primary" :loading="submitting" @click="submitForm">
            {{ t('common.save') }}
          </el-button>
        </div>
      </template>
    </el-drawer>

    <!-- 权限设置 -->
    <el-drawer
      v-model="permVisible"
      :size="permSize"
      :title="t('roles.permTitle')"
      :close-on-click-modal="false"
    >
      <el-alert type="info" :closable="false" show-icon :title="t('roles.permAlert')" />
      <el-divider content-position="left">{{ t('roles.menuVisibility') }}</el-divider>
      <el-tree
        ref="treeRef"
        :data="permTree"
        :props="treeProps"
        show-checkbox
        node-key="id"
        :default-checked-keys="checkedPerms"
        default-expand-all
        class="menu-tree"
      />

      <el-divider content-position="left">{{ t('roles.actionPerm') }}</el-divider>
      <div class="perm-grid">
        <!-- 分组与动作都按 ns / action 标识符翻译：后端下发中文，前端只管显示 -->
        <div v-for="g in permCatalog" :key="g.ns" class="perm-row">
          <span class="perm-title">{{ permGroupLabel(g.ns) }}</span>
          <el-checkbox-group v-model="checkedActions">
            <el-checkbox v-for="a in g.actions" :key="a.key" :value="a.key">
              {{ permActionLabel(permActionOf(a.key)) }}
            </el-checkbox>
          </el-checkbox-group>
        </div>
      </div>

      <template #footer>
        <div class="drawer-footer">
          <el-button @click="permVisible = false">{{ t('common.cancel') }}</el-button>
          <el-button type="primary" :loading="savingPerms" @click="savePermissions">
            {{ t('common.save') }}
          </el-button>
        </div>
      </template>
    </el-drawer>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance, FormRules } from 'element-plus'
import {
  getRoleList,
  createRole,
  updateRole,
  deleteRole,
  getRolePermissions,
  setRolePermissions,
  getPermissionCatalog,
  type RoleItem,
  type PermGroupItem,
} from '@/api/role'
import { getMenuList } from '@/api/menu'
import { isBuiltinRole } from '@/utils/role'
import { permActionLabel, permActionOf, permGroupLabel } from '@/utils/perm'
import { translateTitle, getLocale } from '@/i18n'

const { t } = useI18n()

const loading = ref(false)
const rows = ref<RoleItem[]>([])

async function loadList() {
  loading.value = true
  try {
    const res = await getRoleList()
    rows.value = res.data ?? []
  } catch {
    /* handled by interceptor */
  } finally {
    loading.value = false
  }
}

// ── 筛选 ───────────────────────────────────────────────────
const statusFilter = ref<'all' | 'on' | 'off'>('all')
const keyword = ref('')

const filteredData = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return rows.value.filter((r) => {
    if (statusFilter.value === 'on' && r.status !== 1) return false
    if (statusFilter.value === 'off' && r.status === 1) return false
    if (!kw) return true
    return [r.name, r.role_key, r.description].some((f) => (f ?? '').toLowerCase().includes(kw))
  })
})

// ── 窄屏抽屉铺满 ───────────────────────────────────────────
const narrow = ref(false)
function syncNarrow() {
  narrow.value = window.innerWidth < 900
}
const drawerSize = computed(() => (narrow.value ? '100%' : '480px'))
const permSize = computed(() => (narrow.value ? '100%' : '640px'))

// ── 表单 ───────────────────────────────────────────────────
const drawerVisible = ref(false)
const dialogType = ref<'add' | 'edit'>('add')
const submitting = ref(false)
const formRef = ref<FormInstance>()
const editingId = ref<number>(0)

interface F {
  name: string
  role_key: string
  description: string
  status: number
}
const form = reactive<F>({ name: '', role_key: '', description: '', status: 1 })
// computed：切换语言时校验提示跟着变
const rules = computed<FormRules<F>>(() => ({
  name: [{ required: true, message: t('roles.nameRequired'), trigger: 'blur' }],
  role_key: [{ required: true, message: t('roles.keyRequired'), trigger: 'blur' }],
}))

/** 抽屉头像：角色名首字母 */
const avatarText = computed(() => {
  const n = (form.name || form.role_key).trim()
  return n ? n[0].toUpperCase() : '+'
})

function resetForm() {
  formRef.value?.resetFields()
}

function handleAdd() {
  dialogType.value = 'add'
  editingId.value = 0
  Object.assign(form, { name: '', role_key: '', description: '', status: 1 })
  drawerVisible.value = true
}

function handleEdit(row: RoleItem) {
  dialogType.value = 'edit'
  editingId.value = row.id
  Object.assign(form, {
    name: row.name,
    role_key: row.role_key,
    description: row.description,
    status: row.status,
  })
  drawerVisible.value = true
}

async function submitForm() {
  const valid = await formRef.value?.validate().catch(() => false)
  if (!valid) return
  submitting.value = true
  try {
    if (dialogType.value === 'add') {
      await createRole({ name: form.name, role_key: form.role_key, description: form.description })
      ElMessage.success(t('common.createSuccess'))
    } else {
      await updateRole({
        id: editingId.value,
        name: form.name,
        description: form.description,
        status: form.status,
      })
      ElMessage.success(t('common.updateSuccess'))
    }
    drawerVisible.value = false
    loadList()
  } catch {
    /* handled by interceptor */
  } finally {
    submitting.value = false
  }
}

async function handleDelete(row: RoleItem) {
  try {
    await ElMessageBox.confirm(t('roles.deleteConfirm', { name: row.name }), t('common.warning'), {
      type: 'warning',
      confirmButtonText: t('users.confirmDeleteBtn'),
    })
  } catch {
    return
  }
  try {
    await deleteRole(row.id)
    ElMessage.success(t('common.deleteSuccess'))
    loadList()
  } catch {
    /* handled */
  }
}

// ── 权限 ───────────────────────────────────────────────────
const permVisible = ref(false)
const savingPerms = ref(false)
const treeRef = ref()
const permTree = ref<any[]>([])
const checkedPerms = ref<number[]>([])
/** 动作级权限点（{ns}:view / {ns}:edit） */
const permCatalog = ref<PermGroupItem[]>([])
const checkedActions = ref<string[]>([])
let permRoleId = 0

async function loadCatalog() {
  if (permCatalog.value.length) return
  const res = await getPermissionCatalog()
  permCatalog.value = res.data?.groups ?? []
}

// 菜单节点文本在 meta.title（显示名，后端下发中文），回退到 name（路由名）
// translateTitle：中文原文 → 语言包 key，使菜单树跟随语言切换
const treeProps = {
  children: 'children',
  label: (data: any) => translateTitle(data?.meta?.title || data?.name || ''),
}

async function handlePermission(row: RoleItem) {
  permRoleId = row.id
  try {
    const [menusRes, permsRes] = await Promise.all([
      getMenuList(),
      getRolePermissions(row.id),
      loadCatalog(),
    ])
    permTree.value = menusRes.data ?? []
    checkedPerms.value = permsRes.data?.menu_ids ?? []
    checkedActions.value = permsRes.data?.permissions ?? []
    permVisible.value = true
    // drawer 非销毁式，第二次打开需手动同步勾选状态
    await nextTick()
    treeRef.value?.setCheckedKeys(checkedPerms.value)
  } catch {
    /* handled */
  }
}

async function savePermissions() {
  savingPerms.value = true
  try {
    const keys = treeRef.value?.getCheckedKeys() ?? []
    const half = treeRef.value?.getHalfCheckedKeys() ?? []
    await setRolePermissions(permRoleId, [...keys, ...half], checkedActions.value)
    ElMessage.success(t('roles.permSaveSuccess'))
    permVisible.value = false
  } catch {
    /* handled */
  } finally {
    savingPerms.value = false
  }
}

function fmtTime(ts: number) {
  return ts ? new Date(ts * 1000).toLocaleString(getLocale()) : '-'
}

onMounted(() => {
  syncNarrow()
  window.addEventListener('resize', syncNarrow)
  loadList()
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', syncNarrow)
})

/** 供父级（access/index）在切回本面板时刷新 */
defineExpose({ reload: loadList })
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.head-right {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.nav-pills {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px;
  border-radius: 8px;
  background: var(--el-fill-color-light);
}

.nav-pills .pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 10px;
  border-radius: 6px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  transition:
    background-color 0.15s,
    color 0.15s;
}

.nav-pills .pill:hover {
  background: var(--el-fill-color);
}

.nav-pills .pill.active {
  background: var(--el-color-primary);
  color: #fff;
}

.drawer-head {
  display: flex;
  align-items: center;
  gap: 12px;
}

.drawer-head .avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 38px;
  height: 38px;
  flex-shrink: 0;
  border-radius: 50%;
  font-size: 16px;
  font-weight: 600;
  color: #fff;
  background: var(--el-color-primary);
}

.head-main {
  min-width: 0;
}

.head-title {
  font-size: 15px;
  font-weight: 600;
}

.head-sub {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.drawer-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.form-tip {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.6;
}

.perm-grid {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 320px;
  overflow-y: auto;
}

.perm-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.perm-title {
  width: 120px;
  flex-shrink: 0;
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.menu-tree {
  width: 100%;
  max-height: 280px;
  padding: 4px;
  overflow: auto;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: var(--el-border-radius-base);
}
</style>
