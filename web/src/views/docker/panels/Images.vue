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
        <template v-if="isAdmin && selection.length">
          <el-divider direction="vertical" />
          <span class="pane-selected">{{ t('docker.common.selected', { n: selection.length }) }}</span>
          <el-button size="small" type="danger" plain @click="bulkRemove">
            {{ t('docker.common.remove') }}
          </el-button>
        </template>
      </div>
      <div class="pane-toolbar__right">
        <el-button size="small" type="primary" :icon="Build" @click="buildVisible = true">
          {{ t('docker.build.title') }}
        </el-button>
        <template v-if="isAdmin">
          <el-button size="small" :icon="Download" @click="pullVisible = true">
            {{ t('docker.image.pull') }}
          </el-button>
          <el-button size="small" :icon="Delete" @click="prune">{{ t('docker.image.prune') }}</el-button>
        </template>
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
            <!-- 镜像名是详情页入口：列表里最常用的就是这个入口 -->
            <el-link class="cell-name" type="primary" :underline="false" @click="openDetail(row)">
              {{ repoText(row) }}
            </el-link>
            <el-tag v-if="isDangling(row)" size="small" type="info" effect="plain" round>
              {{ t('docker.image.dangling') }}
            </el-tag>
          </div>
          <div class="cell-sub mono">
            <el-link class="cell-id" :underline="false" @click="openDetail(row)">
              {{ shortId(row.ID) }}
            </el-link>
            <el-tooltip :content="t('docker.image.copyId')" placement="top">
              <el-button
                link
                size="small"
                class="copy-btn"
                :icon="Copy"
                @click.stop="copyId(row)"
              />
            </el-tooltip>
          </div>
        </template>
      </el-table-column>

      <!-- 标签独立成列：`repo:tag` 里 tag 容易被忽略，这里单独拎出来 -->
      <el-table-column :label="t('docker.image.tag')" width="130">
        <template #default="{ row }">
          <el-tag v-if="tagText(row)" size="small" effect="plain" type="success">
            {{ tagText(row) }}
          </el-tag>
          <span v-else class="cell-muted">—</span>
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

      <el-table-column :label="t('docker.common.actions')" width="160" fixed="right">
        <template #default="{ row }">
          <div class="row-ops">
            <!-- 行内快启：等价 docker run -d，端口/容器名在弹窗里补（仅限管理员） -->
            <el-tooltip v-if="isAdmin" :content="t('docker.image.run')" placement="top">
              <el-button link type="success" :icon="Play" @click="openRun(row)" />
            </el-tooltip>
            <el-tooltip v-if="isAdmin" :content="t('docker.common.remove')" placement="top">
              <el-button link type="danger" :icon="Delete" @click="remove(row)" />
            </el-tooltip>
            <el-dropdown trigger="click" @command="(cmd: string) => onRowCommand(cmd)">
              <el-button link :icon="MoreFilled" />
              <template #dropdown>
                <el-dropdown-menu>
                  <!-- 后续 Docker 侧的更多动作往这里加 -->
                  <el-dropdown-item command="backup-home" :icon="Home">
                    {{ t('docker.image.backupHome') }}
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>
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

    <!-- 行内快启：只问最少的信息，端口留空则由 daemon 随机分配 -->
    <el-dialog v-model="runVisible" :title="t('docker.image.runTitle')" width="520px">
      <el-form label-position="top" size="default">
        <el-form-item :label="t('docker.image.runImage')">
          <el-input :model-value="runForm.image" disabled />
        </el-form-item>
        <el-form-item :label="t('docker.image.runName')">
          <el-input v-model="runForm.name" :placeholder="t('docker.image.runNamePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('docker.image.runPorts')">
          <div class="port-editor">
            <div v-for="(p, i) in runForm.ports" :key="i" class="port-row">
              <el-input v-model="runForm.ports[i]" :placeholder="t('docker.image.runPortsPlaceholder')" />
              <el-button link type="danger" :icon="Delete" @click="runForm.ports.splice(i, 1)" />
            </div>
            <el-button size="small" :icon="Plus" @click="runForm.ports.push('')">
              {{ t('docker.image.runAddPort') }}
            </el-button>
          </div>
        </el-form-item>
        <el-form-item :label="t('docker.image.runRestart')">
          <el-select v-model="runForm.restart" :style="{ width: '200px' }">
            <el-option :label="t('docker.image.restartDefault')" value="" />
            <el-option label="no" value="no" />
            <el-option label="always" value="always" />
            <el-option label="unless-stopped" value="unless-stopped" />
            <el-option label="on-failure" value="on-failure" />
          </el-select>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="runVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :icon="Play" :loading="starting" @click="doRun">
          {{ t('docker.image.run') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 镜像详情与构建抽屉 -->
    <ImageDetailDrawer v-model="detailVisible" :image-id="detailId" />
    <ImageBuildDrawer v-model="buildVisible" @built="load" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Build, Copy, Delete, Download, Home, MoreFilled, Play, Plus, Refresh, Search } from '@/icons'
import { containerRun, imageAction, listImages, type DockerImage } from '@/api/docker'
import { backupHome } from '@/api/user'
import { getRuns, type RunItem } from '@/api/appstore'
import { useUserStore } from '@/stores/user'
import ImageDetailDrawer from '../components/ImageDetailDrawer.vue'
import ImageBuildDrawer from '../components/ImageBuildDrawer.vue'

const props = defineProps<{ refreshToken: number }>()
const emit = defineEmits<{ count: [number]; refresh: [] }>()

const { t } = useI18n()
const userStore = useUserStore()

/**
 * 拉取 / 删除 / 清理 / 启容器都是管理员专属接口（见后端 `access.rs`），
 * 这里先按角色隐藏，避免给普通用户摆一排必然 403 的按钮；
 * 构建则是「拿到 docker:build 即可用」，所以对所有人可见，由接口判定。
 */
const isAdmin = computed(() => userStore.roles.includes('admin'))

const loading = ref(false)
const rows = ref<DockerImage[]>([])
const selection = ref<DockerImage[]>([])
const keyword = ref('')
const danglingFilter = ref('all')

const pullVisible = ref(false)
const pulling = ref(false)
const reference = ref('')

// ── 详情抽屉 / 构建抽屉 ────────────────────────────────────
const detailVisible = ref(false)
const detailId = ref('')
const buildVisible = ref(false)

function openDetail(row: DockerImage) {
  detailId.value = row.ID
  detailVisible.value = true
}

// ── 行内快启 ───────────────────────────────────────────────
const runVisible = ref(false)
const starting = ref(false)
const runForm = reactive({ image: '', name: '', ports: [] as string[], restart: '' })

function openRun(row: DockerImage) {
  runForm.image = fullRef(row)
  // 用仓库名兜出一个合法容器名：daemon 对容器名的字符集有要求，库/config/部分/栈名都不行
  runForm.name = String(row.Repository || '')
    .split('/')
    .pop()!
    .replace(/[^a-zA-Z0-9_.-]/g, '-')
    .replace(/^[^a-zA-Z0-9]+/, '')
    .slice(0, 63)
  runForm.ports = []
  runForm.restart = ''
  runVisible.value = true
}

async function doRun() {
  starting.value = true
  try {
    const resp = await containerRun({
      image: runForm.image,
      name: runForm.name.trim(),
      ports: runForm.ports.map((p) => p.trim()).filter(Boolean),
      restart: runForm.restart,
    })
    ElMessage.success(t('docker.image.runStarted', { name: resp.data?.name || runForm.image }))
    runVisible.value = false
    emit('refresh')
  } catch (e: any) {
    ElMessage.error(e?.message || t('docker.common.actionFailed'))
  } finally {
    starting.value = false
  }
}

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
  return row.Repository
}

/** 标签单独显示；`<none>`（悬空镜像）视为空 */
function tagText(row: DockerImage): string {
  return row.Tag && row.Tag !== '<none>' ? row.Tag : ''
}

function shortId(id: string): string {
  return id?.replace('sha256:', '').slice(0, 12) || ''
}

/** 完整引用 `repo:tag`，删除确认等文案用它，避免两个同名不同标签的镜像混淆 */
function fullRef(row: DockerImage): string {
  const tag = tagText(row)
  return tag ? `${repoText(row)}:${tag}` : repoText(row)
}

async function copyId(row: DockerImage) {
  const id = row.ID?.replace('sha256:', '') || ''
  try {
    await navigator.clipboard.writeText(id)
    ElMessage.success(t('common.copySuccess'))
  } catch {
    ElMessage.error(t('docker.image.copyFailed'))
  }
}

// ── 行菜单 ────────────────────────────────────────────────
const backing = ref(false)

function onRowCommand(cmd: string) {
  if (cmd === 'backup-home') void backupMyHome()
}

/**
 * 备份当前账号家目录 → `{home}/backups/home_backup_<时间戳>.tar.gz`。
 *
 * 打包可能持续很久（几十 GB 的家目录），后端丢后台任务跑，
 * 这里只负责提示 + 轮询结果（后端zz CronRun 走 tar，不占内存也不需要前端等待）。
 */
async function backupMyHome() {
  if (backing.value) return
  try {
    await ElMessageBox.confirm(t('docker.image.backupAsk'), t('docker.image.backupHome'), {
      type: 'warning',
    })
  } catch {
    return
  }
  backing.value = true
  try {
    const res = await backupHome()
    ElMessage.success(t('docker.image.backupStarted'))
    trackRun(res.data.run_id, res.data.path)
  } catch (e: any) {
    ElMessage.error(e?.message || t('docker.common.actionFailed'))
  } finally {
    backing.value = false
  }
}

/** 轮询备份任务直到终态（逃离 panel 时清理 timer） */
const runPolls = new Map<string, number>()

function trackRun(runId: string, path: string) {
  if (runPolls.has(runId)) return
  const timer = window.setInterval(async () => {
    try {
      const resp = await getRuns({ page: 1, page_size: 50 })
      const item: RunItem | undefined = (resp.data?.items || []).find(
        (r: RunItem) => r.run_id === runId,
      )
      if (!item || item.status === 'running') return
      window.clearInterval(timer)
      runPolls.delete(runId)
      if (item.status === 'success') {
        ElMessage.success(t('docker.image.backupSuccess', { path }))
      } else {
        ElMessage.error(t('docker.image.backupFailed', { code: item.exit_code }))
      }
    } catch {
      // 后端抖动，下一轮重试
    }
  }, 2000)
  runPolls.set(runId, timer)
}

onBeforeUnmount(() => {
  for (const timer of runPolls.values()) window.clearInterval(timer)
  runPolls.clear()
})

async function remove(row: DockerImage) {
  try {
    await ElMessageBox.confirm(
      t('docker.common.removeConfirm', { name: fullRef(row) }),
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

/* 镜像 ID：比名称弱一档，但同样可点进详情 */
.cell-id.el-link {
  font-size: 11px;
  font-family: Menlo, Monaco, 'Courier New', monospace;
}

.port-editor {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
}

.port-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.cell-sub {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--el-text-color-secondary);
}

.cell-muted {
  color: var(--el-text-color-placeholder);
}

.copy-btn {
  height: auto;
  padding: 0 2px;
}

.row-ops {
  display: flex;
  align-items: center;
  gap: 4px;
}

.mono {
  font-family: Menlo, Monaco, 'Courier New', monospace;
  font-size: 12px;
}
</style>
