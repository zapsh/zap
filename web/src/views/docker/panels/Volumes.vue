<template>
  <div class="docker-pane">
    <div class="pane-toolbar">
      <div class="pane-toolbar__left">
        <el-input
          v-model="keyword"
          :prefix-icon="Search"
          :placeholder="t('docker.common.search')"
          clearable
          :style="{ width: '240px' }"
        />
        <template v-if="selection.length">
          <el-divider direction="vertical" />
          <span class="pane-selected">{{ t('docker.common.selected', { n: selection.length }) }}</span>
          <el-button size="small" type="danger" plain @click="bulkRemove">
            {{ t('docker.common.remove') }}
          </el-button>
        </template>
      </div>
      <div class="pane-toolbar__right">
        <el-button size="small" type="primary" :icon="Plus" @click="createVisible = true">
          {{ t('docker.volume.create') }}
        </el-button>
        <el-button size="small" :icon="Delete" @click="prune">{{ t('docker.volume.prune') }}</el-button>
        <el-button size="small" :icon="Refresh" @click="emit('refresh')">{{ t('docker.refresh') }}</el-button>
      </div>
    </div>

    <el-table
      v-loading="loading"
      :data="filtered"
      row-key="Name"
      size="small"
      @selection-change="(rows: DockerVolume[]) => (selection = rows)"
    >
      <el-table-column type="selection" width="42" />

      <el-table-column :label="t('docker.volume.name')" min-width="200">
        <template #default="{ row }">
          <span class="cell-name mono">{{ row.Name }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.driver')" width="120">
        <template #default="{ row }">{{ row.Driver || '—' }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.volume.mountpoint')" min-width="280" show-overflow-tooltip>
        <template #default="{ row }">
          <span class="mono">{{ row.Mountpoint || '—' }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.scope')" width="120">
        <template #default="{ row }">{{ row.Scope || '—' }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.common.actions')" width="120" fixed="right">
        <template #default="{ row }">
          <el-button link type="danger" @click="remove(row)">{{ t('docker.common.remove') }}</el-button>
        </template>
      </el-table-column>

      <template #empty>
        <el-empty :description="t('docker.common.empty')" :image-size="60" />
      </template>
    </el-table>

    <el-dialog v-model="createVisible" :title="t('docker.volume.createTitle')" width="460px">
      <el-input v-model="form.name" :placeholder="t('docker.volume.namePlaceholder')" @keyup.enter="doCreate" />
      <template #footer>
        <el-button @click="createVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="doCreate">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete, Plus, Refresh, Search } from '@/icons'
import { listVolumes, volumeAction, type DockerVolume } from '@/api/docker'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()

const loading = ref(false)
const rows = ref<DockerVolume[]>([])
const selection = ref<DockerVolume[]>([])
const keyword = ref('')

const createVisible = ref(false)
const saving = ref(false)
const form = reactive({ name: '' })

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return rows.value
  return rows.value.filter((r) => `${r.Name} ${r.Driver} ${r.Mountpoint}`.toLowerCase().includes(kw))
})

async function remove(row: DockerVolume) {
  try {
    await ElMessageBox.confirm(
      t('docker.common.removeConfirm', { name: row.Name }),
      t('docker.common.remove'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await volumeAction(row.Name, 'remove')
    ElMessage.success(t('docker.common.deleted'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

async function bulkRemove() {
  if (!selection.value.length) {
    ElMessage.warning(t('docker.common.noSelection'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('docker.common.bulkRemoveConfirm', { n: selection.value.length }),
      t('docker.common.remove'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  let failed = 0
  for (const row of selection.value) {
    try {
      await volumeAction(row.Name, 'remove')
    } catch {
      failed += 1
    }
  }
  if (failed) ElMessage.warning(t('docker.common.partialFailed', { n: failed }))
  else ElMessage.success(t('docker.common.deleted'))
  selection.value = []
  await load()
}

async function prune() {
  try {
    await ElMessageBox.confirm(t('docker.common.pruneConfirm'), t('docker.volume.prune'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await volumeAction('', 'prune')
    ElMessage.success(t('docker.common.pruned'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

async function doCreate() {
  const name = form.name.trim()
  if (!name) return
  saving.value = true
  try {
    await volumeAction(name, 'create')
    ElMessage.success(t('common.saveSuccess'))
    form.name = ''
    createVisible.value = false
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  } finally {
    saving.value = false
  }
}

async function load() {
  loading.value = true
  try {
    const resp = await listVolumes()
    rows.value = resp.data.items ?? []
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.loadFailed'))
  } finally {
    loading.value = false
  }
}

watch(() => props.refreshToken, load)
watch(() => rows.value.length, (n) => emit('count', n), { immediate: true })
onMounted(load)
</script>

<style scoped>
.pane-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.pane-toolbar__left,
.pane-toolbar__right {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.pane-selected {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.cell-name {
  font-weight: 600;
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}
</style>
