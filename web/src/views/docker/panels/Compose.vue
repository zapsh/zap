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
        <el-button size="small" :icon="Refresh" @click="emit('refresh')">{{ t('docker.refresh') }}</el-button>
      </div>
    </div>

    <el-alert
      v-if="env && !env.compose"
      type="info"
      :closable="false"
      class="compose-tip"
      :title="t('docker.env.composeOff')"
    />

    <el-table v-loading="loading" :data="filtered" row-key="Name" size="small">
      <el-table-column :label="t('docker.compose.name')" min-width="180">
        <template #default="{ row }">
          <span class="cell-name">{{ row.Name }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.compose.status')" width="160">
        <template #default="{ row }">
          <el-tag size="small" :type="row.Status?.startsWith('running') ? 'success' : 'info'" effect="dark">
            {{ row.Status || '—' }}
          </el-tag>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.compose.config')" min-width="280" show-overflow-tooltip>
        <template #default="{ row }">
          <span class="mono">{{ row.ConfigFiles || '—' }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('docker.common.actions')" width="280" fixed="right">
        <template #default="{ row }">
          <el-button link type="success" @click="act(row, 'up')">{{ t('docker.compose.up') }}</el-button>
          <el-button link type="warning" @click="act(row, 'stop')">{{ t('docker.compose.stop') }}</el-button>
          <el-button link type="primary" @click="act(row, 'restart')">
            {{ t('docker.compose.restart') }}
          </el-button>
          <el-button link type="primary" @click="act(row, 'pull')">{{ t('docker.compose.pull') }}</el-button>
          <el-button link type="danger" @click="remove(row)">{{ t('docker.compose.down') }}</el-button>
        </template>
      </el-table-column>

      <template #empty>
        <el-empty :description="t('docker.common.empty')" :image-size="60" />
      </template>
    </el-table>
  </div>
</template>

<script setup lang="ts">
import { computed, inject, onMounted, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Search } from '@/icons'
import { composeAction, listComposeProjects, type DockerComposeProject, type DockerEnvStatus } from '@/api/docker'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()

/** 由父页面注入的 Docker 环境信息，用于提示 compose 插件是否可用 */
const env = inject<Ref<DockerEnvStatus | null>>('docker-env', ref(null))

const loading = ref(false)
const rows = ref<DockerComposeProject[]>([])
const keyword = ref('')

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return rows.value
  return rows.value.filter((r) => `${r.Name} ${r.Status} ${r.ConfigFiles}`.toLowerCase().includes(kw))
})

async function act(row: DockerComposeProject, action: string) {
  try {
    await composeAction(row.Name, action)
    ElMessage.success(action === 'pull' ? t('docker.compose.pulled') : t('docker.compose.upDone'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

async function remove(row: DockerComposeProject) {
  try {
    await ElMessageBox.confirm(
      t('docker.compose.downConfirm', { name: row.Name }),
      t('docker.compose.down'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await composeAction(row.Name, 'down')
    ElMessage.success(t('docker.compose.downDone'))
    await load()
  } catch (e: any) {
    ElMessage.error(e.message || t('docker.common.actionFailed'))
  }
}

async function load() {
  loading.value = true
  try {
    const resp = await listComposeProjects()
    rows.value = resp.data.items ?? []
  } catch (e: any) {
    // compose 插件不可用时 docker compose ls 必然失败，这里静默成空列表，
    // 由上方的 alert 说明原因，避免每次开 tab 都弹错误。
    rows.value = []
    if (env.value?.compose) {
      ElMessage.error(e.message || t('docker.common.loadFailed'))
    }
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

.compose-tip {
  margin-bottom: 12px;
}

.cell-name {
  font-weight: 600;
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}
</style>
