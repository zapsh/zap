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
        <el-select v-model="danglingFilter" :style="{ width: '150px' }">
          <el-option :label="t('docker.image.filterAll')" value="all" />
          <el-option :label="t('docker.image.filterUsed')" value="used" />
          <el-option :label="t('docker.image.filterDangling')" value="dangling" />
        </el-select>
        <template v-if="selection.length">
          <el-divider direction="vertical" />
          <span class="pane-selected">{{ t('docker.common.selected', { n: selection.length }) }}</span>
          <el-button size="small" type="danger" plain @click="bulkRemove">
            {{ t('docker.common.remove') }}
          </el-button>
        </template>
      </div>
      <div class="pane-toolbar__right">
        <el-button size="small" type="primary" :icon="Download" @click="pullVisible = true">
          {{ t('docker.image.pull') }}
        </el-button>
        <el-button size="small" :icon="Delete" @click="prune">{{ t('docker.image.prune') }}</el-button>
        <el-button size="small" :icon="Refresh" @click="emit('refresh')">{{ t('docker.refresh') }}</el-button>
      </div>
    </div>

    <el-table
      v-loading="loading"
      :data="filtered"
      row-key="ID"
      size="small"
      @selection-change="(rows: DockerImage[]) => (selection = rows)"
    >
      <el-table-column type="selection" width="42" />

      <el-table-column :label="t('docker.image.name')" min-width="240">
        <template #default="{ row }">
          <div class="cell-primary">
            <span class="cell-name">{{ repoText(row) }}</span>
            <el-tag v-if="isDangling(row)" size="small" type="info" effect="plain" round>
              {{ t('docker.image.dangling') }}
            </el-tag>
          </div>
          <div class="cell-sub mono">{{ shortId(row.ID) }}</div>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.size')" width="110">
        <template #default="{ row }">{{ row.Size }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.image.containers')" width="100">
        <template #default="{ row }">
          <el-tag v-if="Number(row.Containers) > 0" size="small" effect="light">
            {{ row.Containers }}
          </el-tag>
          <span v-else>—</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.created')" width="180">
        <template #default="{ row }">{{ row.CreatedAt || '—' }}</template>
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

    <el-dialog v-model="pullVisible" :title="t('docker.image.pull')" width="480px">
      <el-input
        v-model="reference"
        :placeholder="t('docker.image.pullPlaceholder')"
        @keyup.enter="doPull"
      />
      <template #footer>
        <el-button @click="pullVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="pulling" @click="doPull">{{ t('docker.image.pull') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete, Download, Refresh, Search } from '@/icons'
import { imageAction, listImages, type DockerImage } from '@/api/docker'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()

const loading = ref(false)
const rows = ref<DockerImage[]>([])
const selection = ref<DockerImage[]>([])
const keyword = ref('')
const danglingFilter = ref('all')

const pullVisible = ref(false)
const pulling = ref(false)
const reference = ref('')

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return rows.value.filter((r) => {
    const dangling = isDangling(r)
    if (danglingFilter.value === 'dangling' && !dangling) return false
    if (danglingFilter.value === 'used' && dangling) return false
    if (!kw) return true
    return `${r.ID} ${r.Repository} ${r.Tag}`.toLowerCase().includes(kw)
  })
})

function isDangling(row: DockerImage): boolean {
  return !row.Repository || row.Repository === '<none>' || row.Tag === '<none>'
}

function repoText(row: DockerImage): string {
  if (isDangling(row)) return shortId(row.ID)
  return `${row.Repository}:${row.Tag}`
}

function shortId(id: string): string {
  return id?.replace('sha256:', '').slice(0, 12) || ''
}

async function remove(row: DockerImage) {
  try {
    await ElMessageBox.confirm(
      t('docker.common.removeConfirm', { name: repoText(row) }),
      t('docker.common.remove'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await imageAction(row.ID, 'remove')
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
      await imageAction(row.ID, 'remove')
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
    await ElMessageBox.confirm(t('docker.common.pruneConfirm'), t('docker.image.prune'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await imageAction('', 'prune')
    ElMessage.success(t('docker.common.pruned'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

async function doPull() {
  const ref_ = reference.value.trim()
  if (!ref_) return
  pulling.value = true
  try {
    await imageAction(ref_, 'pull')
    ElMessage.success(t('docker.image.pulled'))
    pullVisible.value = false
    reference.value = ''
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  } finally {
    pulling.value = false
  }
}

async function load() {
  loading.value = true
  try {
    const resp = await listImages()
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

.cell-primary {
  display: flex;
  align-items: center;
  gap: 6px;
}

.cell-name {
  font-weight: 600;
}

.cell-sub {
  font-size: 11px;
  color: var(--el-text-color-secondary);
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}
</style>
