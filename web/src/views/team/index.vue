<template>
  <div class="team-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span class="page-title">{{ t('team.title') }}</span>
          <div class="head-right">
            <el-button type="primary" :disabled="isMember" @click="handleAdd">
              <el-icon><Plus /></el-icon>{{ t('team.add') }}
            </el-button>
          </div>
        </div>
      </template>

      <!-- 共享账号说明：成员不单独建系统账号，SSH / 文件 / FPM 都以父账号运行 -->
      <el-alert type="info" :closable="false" class="tip-alert">
        <template #title>
          <div class="tip-title">{{ t('team.subtitle') }}</div>
          <div class="tip-body">{{ t('team.sshTip') }}</div>
        </template>
      </el-alert>

      <el-table :data="tableData" v-loading="loading" stripe>
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="username" :label="t('team.username')" width="140" />
        <el-table-column prop="nickname" :label="t('team.nickname')" width="140" />
        <el-table-column prop="email" :label="t('team.email')" min-width="180" />
        <el-table-column :label="t('team.sharedHome')" min-width="200">
          <template #default="{ row }">
            <el-tooltip
              v-if="row.home_dir"
              :content="t('team.sharedTip', { dir: row.home_dir })"
              placement="top"
            >
              <code class="mono">{{ row.home_dir }}</code>
            </el-tooltip>
            <span v-else class="muted">-</span>
            <div v-if="row.linux_user" class="shared-account">
              {{ t('team.sharedAccount') }}：<code class="mono">{{ row.linux_user }}</code>
            </div>
          </template>
        </el-table-column>
        <el-table-column :label="t('team.denyPerm')" min-width="180">
          <template #default="{ row }">
            <el-tag
              v-for="k in row.perm_deny"
              :key="k"
              size="small"
              type="warning"
              effect="plain"
              class="perm-tag"
            >
              {{ permKeyLabel(k) }}
            </el-tag>
            <span v-if="!row.perm_deny?.length" class="muted">-</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.status')" width="80">
          <template #default="{ row }">
            <el-tag :type="row.status === 1 ? 'success' : 'danger'" size="small">
              {{ row.status === 1 ? t('common.enable') : t('common.disable') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('team.lastLogin')" width="170">
          <template #default="{ row }">
            <span v-if="row.last_login_time">{{ fmtTime(row.last_login_time) }}</span>
            <span v-else class="muted">{{ t('team.neverLogin') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.createdAt')" width="170">
          <template #default="{ row }">{{ fmtTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="200" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link @click="handleEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button
              :type="row.status === 1 ? 'warning' : 'success'"
              link
              @click="handleToggleStatus(row)"
            >
              {{ row.status === 1 ? t('common.disable') : t('common.enable') }}
            </el-button>
            <el-button type="danger" link @click="handleDelete(row)">
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <div class="empty-tip">{{ t('team.subtitle') }}</div>
        </template>
      </el-table>
    </el-card>

    <!-- 新增 / 编辑成员 -->
    <el-dialog
      v-model="dialogVisible"
      :title="dialogType === 'add' ? t('team.add') : t('team.edit')"
      width="560px"
      :close-on-click-modal="false"
    >
      <el-form ref="formRef" :model="form" :rules="rules" label-width="110px">
        <el-form-item :label="t('team.username')" prop="username">
          <el-input
            v-model="form.username"
            :disabled="dialogType === 'edit'"
            autocomplete="off"
          />
        </el-form-item>
        <el-form-item :label="t('team.password')" prop="password">
          <el-input
            v-model="form.password"
            type="password"
            show-password
            autocomplete="new-password"
            :placeholder="dialogType === 'edit' ? t('team.passwordKeepTip') : ''"
          />
        </el-form-item>
        <el-form-item :label="t('team.email')" prop="email">
          <el-input v-model="form.email" />
        </el-form-item>
        <el-form-item :label="t('team.phone')">
          <el-input v-model="form.phone" />
        </el-form-item>
        <el-form-item :label="t('team.nickname')">
          <el-input v-model="form.nickname" :placeholder="t('users.nicknameTip')" />
        </el-form-item>
        <el-form-item :label="t('team.denyPerm')">
          <el-select
            v-model="form.perm_deny"
            multiple
            filterable
            clearable
            collapse-tags
            collapse-tags-tooltip
            :placeholder="t('team.denyPermPlaceholder')"
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
          <div class="form-tip">{{ t('team.inheritTip') }}</div>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="submitting" @click="handleSubmit">
          {{ t('common.save') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance, FormRules } from 'element-plus'
import {
  getTeamList,
  createTeamMember,
  updateTeamMember,
  deleteTeamMember,
  type TeamMemberItem,
} from '@/api/user'
import { getPermissionCatalog, type PermGroupItem } from '@/api/role'
import { permGroupLabel, permKeyLabel } from '@/utils/perm'
import { getLocale } from '@/i18n'
import { useUserStore } from '@/stores/user'

const { t } = useI18n()
const userStore = useUserStore()

/** 当前账号本身是成员时不能建成员（后端也会拦，这里只做体验层收敛） */
const isMember = computed(() => (userStore.userInfo as { user_kind?: number }).user_kind === 1)

// ── 列表 ──
const loading = ref(false)
const tableData = ref<TeamMemberItem[]>([])

async function loadList() {
  loading.value = true
  try {
    const res = await getTeamList()
    tableData.value = (res.data ?? []).map((r) => ({
      ...r,
      perm_deny: r.perm_deny ?? [],
    }))
  } catch {
    // 拦截器已弹窗
  } finally {
    loading.value = false
  }
}

// ── 权限点目录（收紧权限下拉）──
const permCatalog = ref<PermGroupItem[]>([])
async function loadPermCatalog() {
  if (permCatalog.value.length) return
  try {
    const res = await getPermissionCatalog()
    permCatalog.value = res.data?.groups ?? []
  } catch {
    // 拦截器已弹窗
  }
}

// ── 表单 ──
const dialogVisible = ref(false)
const dialogType = ref<'add' | 'edit'>('add')
const submitting = ref(false)
const formRef = ref<FormInstance>()
const editingId = ref<number>(0)

const form = reactive({
  username: '',
  password: '',
  email: '',
  phone: '',
  nickname: '',
  perm_deny: [] as string[],
})

const rules = computed<FormRules>(() => {
  const base: FormRules = {
    email: [
      { required: true, message: t('team.emailRequired'), trigger: 'blur' },
      { type: 'email', message: t('team.emailInvalid'), trigger: 'blur' },
    ],
  }
  if (dialogType.value === 'add') {
    base.username = [
      { required: true, message: t('team.usernameRequired'), trigger: 'blur' },
      { min: 2, max: 50, message: t('team.usernameLength'), trigger: 'blur' },
    ]
    base.password = [
      { required: true, message: t('team.passwordRequired'), trigger: 'blur' },
      { min: 6, message: t('team.passwordLength'), trigger: 'blur' },
    ]
  } else if (form.password) {
    base.password = [{ min: 6, message: t('team.passwordLength'), trigger: 'blur' }]
  }
  return base
})

function resetForm() {
  form.username = ''
  form.password = ''
  form.email = ''
  form.phone = ''
  form.nickname = ''
  form.perm_deny = []
  editingId.value = 0
}

async function handleAdd() {
  if (isMember.value) return
  await loadPermCatalog()
  dialogType.value = 'add'
  resetForm()
  dialogVisible.value = true
}

async function handleEdit(row: TeamMemberItem) {
  await loadPermCatalog()
  dialogType.value = 'edit'
  resetForm()
  editingId.value = row.id
  form.username = row.username
  form.email = row.email
  form.phone = row.phone ?? ''
  form.nickname = row.nickname ?? ''
  form.perm_deny = [...(row.perm_deny ?? [])]
  dialogVisible.value = true
}

async function handleSubmit() {
  if (!formRef.value) return
  try {
    await formRef.value.validate()
  } catch {
    return
  }
  submitting.value = true
  try {
    if (dialogType.value === 'add') {
      await createTeamMember({
        username: form.username,
        password: form.password,
        email: form.email,
        phone: form.phone || undefined,
        nickname: form.nickname || undefined,
        perm_deny: form.perm_deny,
      })
      ElMessage.success(t('team.createSuccess'))
    } else {
      const payload: Record<string, unknown> = {
        id: editingId.value,
        email: form.email,
        phone: form.phone || undefined,
        nickname: form.nickname || undefined,
        perm_deny: form.perm_deny,
      }
      if (form.password) payload.password = form.password
      await updateTeamMember(payload as never)
      ElMessage.success(t('team.updateSuccess'))
    }
    dialogVisible.value = false
    await loadList()
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('error.system'))
  } finally {
    submitting.value = false
  }
}

// ── 启用 / 禁用 ──
async function handleToggleStatus(row: TeamMemberItem) {
  const action = row.status === 1 ? t('common.disable') : t('common.enable')
  try {
    await ElMessageBox.confirm(t('team.toggleConfirm', { action, name: row.username }), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await updateTeamMember({ id: row.id, status: row.status === 1 ? 0 : 1 })
    ElMessage.success(t('team.updateSuccess'))
    await loadList()
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('error.system'))
  }
}

// ── 删除 ──
async function handleDelete(row: TeamMemberItem) {
  try {
    await ElMessageBox.confirm(
      `${t('team.deleteConfirm', { name: row.username })}\n${t('team.deleteTip')}`,
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteTeamMember(row.id)
    ElMessage.success(t('team.deleteSuccess'))
    await loadList()
  } catch (e: unknown) {
    ElMessage.error((e as Error)?.message || t('error.system'))
  }
}

// ── 工具 ──
function fmtTime(ts: number) {
  if (!ts) return '-'
  return new Date(ts * 1000).toLocaleString(getLocale())
}

onMounted(() => {
  loadList()
})
</script>

<style scoped>
.team-container {
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
}

.tip-alert {
  margin-bottom: 16px;
}

.tip-title {
  font-size: 13px;
  line-height: 1.6;
}

.tip-body {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  opacity: 0.85;
}

.mono {
  font-family: 'SFMono-Regular', Consolas, Menlo, monospace;
  font-size: 12px;
  color: var(--el-color-primary);
}

.shared-account {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.muted {
  color: var(--el-text-color-placeholder);
}

.perm-tag {
  margin: 2px 4px 2px 0;
}

.form-tip {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--el-text-color-secondary);
}

.empty-tip {
  padding: 8px 0;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
