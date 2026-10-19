<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="trash-pane">
    <el-card shadow="never" class="trash-card">
      <template #header>
        <div class="trash-head">
          <span class="page-title">{{ t('filesTrash.title') }}</span>
          <div class="trash-head__actions">
            <el-button
              size="small"
              :icon="Refresh"
              :loading="loading"
              @click="load"
            >
              {{ t('common.refresh') }}
            </el-button>
            <el-button
              size="small"
              type="danger"
              plain
              :icon="Delete"
              :disabled="!items.length || loading"
              @click="purgeAll"
            >
              {{ t('filesTrash.purgeAll') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-alert
        class="trash-tip"
        :title="t('filesTrash.tip')"
        type="info"
        :closable="false"
        show-icon
      />

      <el-table v-loading="loading" :data="items" empty-text=" " style="width: 100%; margin-top: 12px">
        <el-table-column :label="t('filesTrash.colName')" min-width="160">
          <template #default="{ row }">
            <el-icon v-if="row.is_dir" class="row-icon"><Folder /></el-icon>
            <el-icon v-else class="row-icon"><Document /></el-icon>
            <span class="mono">{{ row.name }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('filesTrash.colPath')" prop="original" min-width="220">
          <template #default="{ row }">
            <span class="mono path">{{ row.original }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('filesTrash.colType')" width="100">
          <template #default="{ row }">
            {{ row.is_dir ? t('filesTrash.typeDir') : t('filesTrash.typeFile') }}
          </template>
        </el-table-column>
        <el-table-column :label="t('filesTrash.colDeletedAt')" width="180">
          <template #default="{ row }">
            {{ formatDeletedAt(row.deleted_at) }}
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="200" fixed="right">
          <template #default="{ row }">
            <el-button size="small" type="primary" plain :loading="busy === row.id" @click="restore(row)">
              {{ t('filesTrash.restore') }}
            </el-button>
            <el-button size="small" type="danger" plain :loading="busy === row.id" @click="purge(row)">
              {{ t('filesTrash.purge') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('filesTrash.empty')" :image-size="80" />
        </template>
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onActivated } from 'vue'
import { useI18n } from 'vue-i18n'
import { Delete, Document, Folder, Refresh } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import { listTrash, purgeTrash, restoreTrash, type TrashItem } from '@/api/file'

const { t } = useI18n()

/// 后端 deleted_at 是秒级时间戳，格式化成「YYYY-MM-DD HH:mm:ss」（本地时区）。
function formatDeletedAt(sec: number | string): string {
  const n = Number(sec)
  if (!Number.isFinite(n) || n <= 0) return '-'
  const d = new Date(n * 1000)
  const p = (x: number) => String(x).padStart(2, '0')
  return (
    `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ` +
    `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
  )
}

const items = ref<TrashItem[]>([])
const loading = ref(false)
const busy = ref('') // 正在操作的条目 id（恢复/彻底删除时禁用整行按钮）

async function load() {
  loading.value = true
  try {
    const res = await listTrash()
    items.value = res.data?.list ?? []
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesTrash.loadFailed'))
  } finally {
    loading.value = false
  }
}

onActivated(load)

async function restore(row: TrashItem) {
  try {
    await ElMessageBox.confirm(
      t('filesTrash.restoreConfirm', { path: row.original }),
      t('filesTrash.restore'),
      { type: 'info', confirmButtonText: t('filesTrash.restore') },
    )
  } catch {
    return
  }
  busy.value = row.id
  try {
    await restoreTrash(row.id)
    ElMessage.success(t('filesTrash.restoreOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesTrash.restoreFailed'))
  } finally {
    busy.value = ''
  }
}

async function purge(row: TrashItem) {
  try {
    await ElMessageBox.confirm(
      t('filesTrash.purgeConfirm', { name: row.name }),
      t('filesTrash.purge'),
      { type: 'warning', confirmButtonText: t('common.confirm') },
    )
  } catch {
    return
  }
  busy.value = row.id
  try {
    await purgeTrash(row.id)
    ElMessage.success(t('filesTrash.purgeOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesTrash.purgeFailed'))
  } finally {
    busy.value = ''
  }
}

async function purgeAll() {
  try {
    await ElMessageBox.confirm(
      t('filesTrash.purgeAllConfirm'),
      t('filesTrash.purgeAll'),
      { type: 'warning', confirmButtonText: t('common.confirm') },
    )
  } catch {
    return
  }
  loading.value = true
  try {
    for (const it of items.value) {
      await purgeTrash(it.id)
    }
    ElMessage.success(t('filesTrash.purgeAllOk'))
    await load()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesTrash.purgeFailed'))
  } finally {
    loading.value = false
  }
}
</script>

<style scoped lang="scss">
.trash-pane {
  height: 100%;
}
.trash-card {
  height: 100%;
}
.trash-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.trash-head__actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
.trash-tip {
  margin-bottom: 4px;
}
.row-icon {
  vertical-align: middle;
  margin-right: 4px;
}
.mono {
  font-family: var(--el-font-family-mono, monospace);
}
.path {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
</style>
