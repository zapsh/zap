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
      </div>
      <div class="pane-toolbar__right">
        <el-button size="small" type="primary" :icon="Plus" @click="createVisible = true">
          {{ t('docker.network.create') }}
        </el-button>
        <el-button size="small" :icon="Delete" @click="prune">{{ t('docker.network.prune') }}</el-button>
        <el-button size="small" :icon="Refresh" @click="emit('refresh')">{{ t('docker.refresh') }}</el-button>
      </div>
    </div>

    <el-table v-loading="loading" :data="filtered" row-key="ID" size="small">
      <el-table-column :label="t('docker.network.name')" min-width="200">
        <template #default="{ row }">
          <span class="cell-name">{{ row.Name }}</span>
          <div class="cell-sub mono">{{ shortId(row.ID) }}</div>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.driver')" width="120">
        <template #default="{ row }">{{ row.Driver || '—' }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.common.scope')" width="120">
        <template #default="{ row }">{{ row.Scope || '—' }}</template>
      </el-table-column>

      <el-table-column :label="t('docker.network.ipv6')" width="100">
        <template #default="{ row }">
          <el-tag v-if="row.IPv6" size="small" type="success" effect="plain">IPv6</el-tag>
          <span v-else>—</span>
        </template>
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

    <el-dialog v-model="createVisible" :title="t('docker.network.createTitle')" width="460px">
      <el-form label-width="90px">
        <el-form-item :label="t('docker.common.name')">
          <el-input v-model="form.name" :placeholder="t('docker.volume.namePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('docker.network.driverLabel')">
          <el-select v-model="form.driver" :style="{ width: '100%' }">
            <el-option v-for="d in drivers" :key="d" :label="d" :value="d" />
          </el-select>
        </el-form-item>
      </el-form>
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
import { listNetworks, networkAction, type DockerNetwork } from '@/api/docker'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()

const drivers = ['bridge', 'host', 'overlay', 'macvlan', 'none']

const loading = ref(false)
const rows = ref<DockerNetwork[]>([])
const keyword = ref('')

const createVisible = ref(false)
const saving = ref(false)
const form = reactive({ name: '', driver: 'bridge' })

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return rows.value
  return rows.value.filter((r) => `${r.ID} ${r.Name} ${r.Driver}`.toLowerCase().includes(kw))
})

function shortId(id: string): string {
  return id?.slice(0, 12) || ''
}

async function remove(row: DockerNetwork) {
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
    await networkAction(row.Name, 'remove')
    ElMessage.success(t('docker.common.deleted'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

async function prune() {
  try {
    await ElMessageBox.confirm(t('docker.common.pruneConfirm'), t('docker.network.prune'), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await networkAction('', 'prune')
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
    await networkAction(name, 'create', form.driver)
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
    const resp = await listNetworks()
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
