<template>
  <div class="cache-clean">
    <el-card v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('zapCfg.cacheTitle') }}</span>
          <span class="sub">{{ t('zapCfg.cacheSubtitle') }}</span>
          <el-button class="header-action" size="small" :loading="scanning" @click="scan">
            {{ t('zapCfg.cacheRefresh') }}
          </el-button>
        </div>
      </template>

      <el-alert
        type="info"
        :closable="false"
        show-icon
        :title="t('zapCfg.cacheHint')"
        style="margin-bottom: 16px"
      />

      <el-table :data="targets" border size="small" @selection-change="onSelect">
        <el-table-column type="selection" width="46" />
        <el-table-column :label="t('zapCfg.cacheColItem')" min-width="220">
          <template #default="{ row }">
            <div>{{ labelOf(row.id) }}</div>
            <div class="dir">{{ row.dir }}</div>
          </template>
        </el-table-column>
        <el-table-column :label="t('zapCfg.cacheColCount')" width="120" align="center">
          <template #default="{ row }">{{ row.count }}</template>
        </el-table-column>
        <el-table-column :label="t('zapCfg.cacheColSize')" width="140" align="center">
          <template #default="{ row }">{{ fmtSize(row.size) }}</template>
        </el-table-column>
        <el-table-column :label="t('zapCfg.cacheColProtected')" width="140" align="center">
          <template #default="{ row }">
            <el-tag v-if="row.protected > 0" type="warning" size="small">
              {{ row.protected }}
            </el-tag>
            <span v-else>0</span>
          </template>
        </el-table-column>
      </el-table>

      <div class="footer">
        <el-button
          type="primary"
          :loading="cleaning"
          :disabled="selected.length === 0"
          @click="clean"
        >
          {{ t('zapCfg.cacheClean') }}
        </el-button>
        <span class="hint">{{ t('zapCfg.cacheRunningNote') }}</span>
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  cleanZapCache,
  getZapCache,
  type ZapCacheTarget,
} from '@/api/systemZap'

const { t } = useI18n()

const targets = ref<ZapCacheTarget[]>([])
const selected = ref<string[]>([])
const loading = ref(false)
const scanning = ref(false)
const cleaning = ref(false)

/** id → i18n key 映射（新增清理类型时在此登记） */
const LABELS: Record<string, string> = {
  appstore_logs: 'zapCfg.cacheItemLogs',
  appstore_runs: 'zapCfg.cacheItemRuns',
  appstore_cache: 'zapCfg.cacheItemCache',
  user_cron_logs: 'zapCfg.cacheItemCronLogs',
  user_docker_logs: 'zapCfg.cacheItemDockerLogs',
}
function labelOf(id: string): string {
  const k = LABELS[id]
  return k ? t(k) : id
}

function fmtSize(bytes: number): string {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = bytes
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`
}

function onSelect(rows: ZapCacheTarget[]) {
  selected.value = rows.map((r) => r.id)
}

async function scan() {
  scanning.value = true
  loading.value = true
  try {
    const res = await getZapCache()
    targets.value = res.data.targets ?? []
    // 默认全选（运行中项在清理时仍会被跳过）
    selected.value = targets.value.map((x) => x.id)
  } catch {
    /* handled */
  } finally {
    scanning.value = false
    loading.value = false
  }
}

async function clean() {
  if (selected.value.length === 0) {
    ElMessage.warning(t('zapCfg.cacheNoSelection'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('zapCfg.cacheConfirm', { n: selected.value.length }),
      t('zapCfg.tipTitle'),
      {
        type: 'warning',
        confirmButtonText: t('zapCfg.cacheClean'),
        cancelButtonText: t('zapCfg.cacheCleanCancel'),
      },
    )
  } catch {
    return
  }
  cleaning.value = true
  try {
    const res = await cleanZapCache(selected.value)
    const d = res.data
    if (d.failed > 0) {
      ElMessage.warning(
        t('zapCfg.cacheDonePartial', {
          removed: d.total_removed,
          freed: fmtSize(d.total_freed),
          skipped: d.skipped_running,
          failed: d.failed,
        }),
      )
    } else {
      ElMessage.success(
        t('zapCfg.cacheDone', {
          removed: d.total_removed,
          freed: fmtSize(d.total_freed),
          skipped: d.skipped_running,
        }),
      )
    }
    await scan()
  } catch {
    /* handled */
  } finally {
    cleaning.value = false
  }
}

onMounted(scan)
</script>

<script lang="ts">
export default { name: 'CacheCleanPanel' }
</script>

<style scoped>
.cache-clean {
  padding: 4px;
}
.card-header {
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.card-header .sub {
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.card-header .header-action {
  margin-left: auto;
}
.dir {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  word-break: break-all;
}
.footer {
  margin-top: 16px;
  display: flex;
  align-items: center;
  gap: 12px;
}
.hint {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
</style>
