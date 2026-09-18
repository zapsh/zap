<template>
  <div class="migrate-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <div>
            <span>{{ t('serverMigrate.title') }}</span>
            <span class="card-sub">{{ t('serverMigrate.sub') }}</span>
          </div>
        </div>
      </template>

      <el-alert
        :title="t('serverMigrate.warningAlert')"
        type="warning"
        :closable="false"
        show-icon
        style="margin-bottom: 16px"
      />

      <!-- 挂载点设置 -->
      <el-form inline :model="form" @submit.prevent>
        <el-form-item :label="t('serverMigrate.srcMount')">
          <el-input v-model="form.src" placeholder="/home" style="width: 180px" />
        </el-form-item>
        <el-form-item :label="t('serverMigrate.destMount')">
          <el-input v-model="form.dest" placeholder="/home2" style="width: 180px" />
          <div class="form-tip" style="margin-left: 8px">{{ t('serverMigrate.destTip') }}</div>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="previewLoading" @click="loadPreview">
            {{ t('serverMigrate.queryUsers') }}
          </el-button>
          <el-button
            type="danger"
            :disabled="!canMigrate"
            :loading="migrating"
            @click="confirmMigrate"
          >
            {{ t('serverMigrate.startMigrate') }}
          </el-button>
        </el-form-item>
      </el-form>

      <!-- 候选用户 -->
      <el-table
        :data="candidates"
        v-loading="previewLoading"
        border
        stripe
        @selection-change="onSelect"
      >
        <el-table-column type="selection" width="46" :selectable="() => !migrating" />
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="username" :label="t('serverMigrate.colUsername')" width="150" />
        <el-table-column
          prop="linux_user"
          :label="t('serverMigrate.colLinuxUser')"
          width="140"
          show-overflow-tooltip
        >
          <template #default="{ row }">{{ row.linux_user || '—' }}</template>
        </el-table-column>
        <el-table-column prop="home_dir" :label="t('serverMigrate.colHomeDir')" min-width="220" />
        <el-table-column :label="t('serverMigrate.colSiteCount')" width="100" align="center">
          <template #default="{ row }">
            <el-tag
              size="small"
              :type="row.site_count > 0 ? 'warning' : 'info'"
              disable-transitions
            >
              {{ row.site_count }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('serverMigrate.colShared')" width="120" align="center">
          <template #default="{ row }">
            <el-tag
              v-if="row.share_count > 1"
              size="small"
              type="primary"
              effect="plain"
              disable-transitions
            >
              {{ t('serverMigrate.sharedTag', { n: row.share_count }) }}
            </el-tag>
            <span v-else class="dim-text">—</span>
          </template>
        </el-table-column>
      </el-table>
      <div class="form-tip" style="margin-top: 8px">{{ t('serverMigrate.sharedTip') }}</div>
      <el-empty
        v-if="!previewLoading && previewLoaded && candidates.length === 0"
        :description="t('serverMigrate.empty')"
        :image-size="70"
      />
    </el-card>

    <!-- 迁移结果 -->
    <el-dialog
      v-model="resultVisible"
      :title="t('serverMigrate.resultTitle')"
      width="820px"
      top="8vh"
    >
      <template v-if="result">
        <el-alert
          :title="
            t('serverMigrate.resultSummary', {
              src: result.src,
              dest: result.dest,
              ok: result.ok.length,
              fail: result.fail.length,
            })
          "
          :type="result.fail.length ? 'warning' : 'success'"
          :closable="false"
          show-icon
          style="margin-bottom: 12px"
        />
        <el-table v-if="result.ok.length" :data="result.ok" size="small" border max-height="260">
          <el-table-column :label="t('serverMigrate.colUsername')" width="180">
            <template #default="{ row }">
              <span>{{ row.username }}</span>
              <el-tag
                v-if="row.reused"
                size="small"
                type="info"
                effect="plain"
                style="margin-left: 6px"
              >
                {{ t('serverMigrate.reused') }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="old_home" :label="t('serverMigrate.colOldHome')" min-width="180" />
          <el-table-column prop="new_home" :label="t('serverMigrate.colNewHome')" min-width="180" />
          <el-table-column :label="t('serverMigrate.colSiteSync')" width="110" align="center">
            <template #default="{ row }">
              <span v-if="row.sites > 0">
                {{ row.sites_synced }}/{{ row.sites }}
                <el-tooltip
                  v-if="row.site_errors.length"
                  :content="row.site_errors.join(t('serverMigrate.listSeparator'))"
                  placement="top"
                >
                  <span class="warn-text">{{ t('serverMigrate.partialFailed') }}</span>
                </el-tooltip>
              </span>
              <span v-else class="dim-text">{{ t('serverMigrate.none') }}</span>
            </template>
          </el-table-column>
        </el-table>
        <el-table
          v-if="result.fail.length"
          :data="result.fail"
          size="small"
          border
          max-height="200"
          style="margin-top: 12px"
        >
          <el-table-column prop="username" :label="t('serverMigrate.colUsername')" width="140" />
          <el-table-column prop="home_dir" :label="t('serverMigrate.colHome')" min-width="200" />
          <el-table-column prop="error" :label="t('serverMigrate.colReason')" min-width="240" />
        </el-table>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { reactive, ref, computed } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { getMigrateUsers, runMigrate } from '@/api/serverMigrate'
import type { MigrateCandidate, MigrateResult } from '@/api/serverMigrate'

const { t } = useI18n()

const form = reactive({ src: '/home', dest: '/home2' })
const candidates = ref<MigrateCandidate[]>([])
const selected = ref<MigrateCandidate[]>([])
const previewLoading = ref(false)
const previewLoaded = ref(false)
const migrating = ref(false)
const resultVisible = ref(false)
const result = ref<MigrateResult | null>(null)

const canMigrate = computed(
  () => !previewLoading.value && !migrating.value && selected.value.length > 0,
)

function onSelect(rows: MigrateCandidate[]) {
  selected.value = rows
}

async function loadPreview() {
  if (!form.src.trim().startsWith('/')) {
    ElMessage.warning(t('serverMigrate.invalidSrc'))
    return
  }
  previewLoading.value = true
  try {
    const res = await getMigrateUsers(form.src.trim())
    candidates.value = res.data.candidates ?? []
    previewLoaded.value = true
    ElMessage.success(t('serverMigrate.loaded', { count: res.data.count, src: res.data.src }))
  } catch {
    /* handled */
  } finally {
    previewLoading.value = false
  }
}

async function confirmMigrate() {
  if (!form.dest.trim().startsWith('/') || form.dest.trim() === form.src.trim()) {
    ElMessage.warning(t('serverMigrate.invalidDest'))
    return
  }
  const names = selected.value.map((u) => u.username).join(t('serverMigrate.nameSeparator'))
  try {
    await ElMessageBox.confirm(
      t('serverMigrate.confirmBody', {
        n: selected.value.length,
        dest: form.dest.trim(),
        names,
      }),
      t('serverMigrate.confirmTitle'),
      { type: 'warning', confirmButtonText: t('serverMigrate.confirmStart') },
    )
  } catch {
    return
  }
  migrating.value = true
  try {
    const res = await runMigrate({
      src: form.src.trim(),
      dest: form.dest.trim(),
      user_ids: selected.value.map((u) => u.id),
    })
    result.value = res.data
    resultVisible.value = true
    ElMessage.success(res.message || t('serverMigrate.migrated'))
    // 迁移后刷新候选（已迁移用户将不再出现在源挂载点下）
    loadPreview()
  } catch {
    /* handled */
  } finally {
    migrating.value = false
  }
}
</script>

<style scoped>
.migrate-container {
  padding: 4px;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.card-header > div {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.card-header span:first-child {
  font-weight: 600;
}
.card-sub {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.form-tip {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.dim-text {
  color: var(--el-text-color-placeholder);
}
.warn-text {
  color: #e6a23c;
  margin-left: 4px;
}
</style>
