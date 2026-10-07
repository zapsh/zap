<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-drawer
    v-model="visible"
    :title="mode === 'add' ? t('appstore.addRepoTitle') : t('appstore.repoTitle')"
    size="520px"
    destroy-on-close
  >
    <!-- 增加软件源：表单 -->
    <template v-if="mode === 'add'">
      <el-form label-width="80px" @submit.prevent>
        <el-form-item :label="t('appstore.nameLabel')" required>
          <el-input
            v-model="addForm.name"
            :placeholder="t('appstore.namePlaceholder')"
            maxlength="32"
          />
        </el-form-item>
        <el-form-item :label="t('appstore.gitUrlLabel')" required>
          <el-input v-model="addForm.url" placeholder="https://github.com/org/repo.git" />
        </el-form-item>
      </el-form>
      <div class="repo-intro">{{ t('appstore.repoSub') }}</div>
    </template>

    <!-- 软件源列表：更新 / 删除 -->
    <div v-else class="repo-list" v-loading="loading">
      <div class="repo-intro">{{ t('appstore.repoSub') }}</div>
      <div v-for="r in repos" :key="r.id" class="repo-item">
        <div class="repo-item-left">
          <div class="repo-item-name">
            {{ r.name || r.id }}
            <el-tag v-if="r.builtin" size="small" type="primary" effect="plain">{{
              t('appstore.tagBuiltin')
            }}</el-tag>
            <el-tag v-if="!r.exists" size="small" type="danger" effect="plain">{{
              t('appstore.tagDirMissing')
            }}</el-tag>
          </div>
          <div class="repo-item-url">{{ r.url }}</div>
          <div class="repo-item-meta">
            <span>id: {{ r.id }}</span>
            <span v-if="r.version">{{ t('appstore.versionColon') }} {{ r.version }}</span>
            <span v-if="r.commit">commit: {{ r.commit.slice(0, 7) }}</span>
            <span>{{ t('appstore.updatedAtColon', { time: fmtTime(r.updated_at) }) }}</span>
          </div>
        </div>
        <div class="repo-item-actions">
          <el-button
            size="small"
            type="primary"
            plain
            :disabled="!isAdmin"
            @click="handleUpdate(r)"
            >{{ t('appstore.update') }}</el-button
          >
          <el-button
            v-if="!r.builtin"
            size="small"
            type="danger"
            plain
            :disabled="!isAdmin"
            @click="handleRemove(r)"
            >{{ t('common.delete') }}</el-button
          >
        </div>
      </div>
      <el-empty
        v-if="!loading && repos.length === 0"
        :description="t('appstore.noRepo')"
        :image-size="60"
      />
    </div>

    <template #footer>
      <div class="drawer-footer">
        <el-button v-if="mode === 'add'" @click="mode = 'list'">{{ t('common.cancel') }}</el-button>
        <el-button v-else @click="visible = false">{{ t('common.close') }}</el-button>
        <el-button
          v-if="mode === 'add'"
          type="primary"
          :loading="adding"
          @click="handleAdd"
          >{{ t('appstore.addAndPull') }}</el-button
        >
        <el-button
          v-else
          type="primary"
          :icon="Plus"
          :disabled="!isAdmin"
          @click="mode = 'add'"
          >{{ t('appstore.addSource') }}</el-button
        >
      </div>
    </template>
  </el-drawer>
</template>

<script setup lang="ts">
/**
 * 软件园（Git 源）管理抽屉：列表 + 更新 / 删除 / 增加源。
 *
 * 原来这张卡占着应用商店页顶部，现在收到右上角「管理软件园」「增加软件源」两个入口里：
 * 打开时拉一次源列表，操作后 emit changed 让商店页重载包列表，日志交给父级的日志抽屉。
 */
import { computed, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Plus } from '@/icons'
import { useUserStore } from '@/stores/user'
import { addRepo, getRepos, removeRepo, updateRepo, type RepoSource } from '@/api/appstore'

const emit = defineEmits<{
  /** 源增减会影响包列表，父级据此重载 */
  (e: 'changed'): void
  /** 拉取 / 删除是异步任务，把日志交给父级的日志抽屉 */
  (e: 'log', runId: string, title: string): void
}>()

const { t } = useI18n()
const userStore = useUserStore()
const isAdmin = computed(() => userStore.roles.includes('admin'))

defineOptions({ name: 'RepoManageDrawer' })

const visible = ref(false)
/** add = 直接进添加表单（右上角「增加软件源」） */
const mode = ref<'list' | 'add'>('list')
const loading = ref(false)
const adding = ref(false)
const repos = ref<RepoSource[]>([])
const addForm = ref({ name: '', url: '' })

async function loadRepos() {
  loading.value = true
  try {
    const resp = await getRepos()
    repos.value = resp.data.repos || []
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.loadReposFailed'))
  } finally {
    loading.value = false
  }
}

function open(next: 'list' | 'add' = 'list') {
  mode.value = next
  visible.value = true
  loadRepos()
}

async function handleAdd() {
  const name = addForm.value.name.trim()
  const url = addForm.value.url.trim()
  if (!name) {
    ElMessage.warning(t('appstore.nameRequired'))
    return
  }
  if (!url) {
    ElMessage.warning(t('appstore.urlRequired'))
    return
  }
  adding.value = true
  try {
    const resp = await addRepo({ name, url })
    ElMessage.success(t('appstore.addStarted'))
    addForm.value = { name: '', url: '' }
    mode.value = 'list'
    emit('log', resp.data.run_id, t('appstore.addRepoLogTitle'))
    setTimeout(() => {
      loadRepos()
      emit('changed')
    }, 3000)
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.addFailed'))
  } finally {
    adding.value = false
  }
}

async function handleUpdate(r: RepoSource) {
  try {
    const resp = await updateRepo({ id: r.id })
    ElMessage.success(t('appstore.updateStarted'))
    emit('log', resp.data.run_id, t('appstore.updateLogTitle', { name: r.name || r.id }))
    setTimeout(() => {
      loadRepos()
      emit('changed')
    }, 3000)
  } catch (e: any) {
    ElMessage.error(e.message || t('appstore.updateFailed'))
  }
}

async function handleRemove(r: RepoSource) {
  try {
    await ElMessageBox.confirm(
      t('appstore.removeConfirm', { name: r.name || r.id }),
      t('appstore.removeTitle'),
      { type: 'warning' },
    )
    const resp = await removeRepo({ id: r.id })
    ElMessage.success(resp.message || t('appstore.removed'))
    loadRepos()
    emit('changed')
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('appstore.removeFailed'))
  }
}

function fmtTime(ts: number | null | undefined): string {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

defineExpose({ open })
</script>

<style scoped>
.repo-intro {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.6;
  margin-bottom: 12px;
}

.repo-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 80px;
}

.repo-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  background: var(--el-bg-color-page);
}

.repo-item-left {
  min-width: 0;
}

.repo-item-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  display: flex;
  align-items: center;
  gap: 6px;
}

.repo-item-url {
  font-size: 12px;
  color: var(--el-text-color-regular);
  margin-top: 4px;
  word-break: break-all;
}

.repo-item-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.repo-item-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.drawer-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
