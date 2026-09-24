<template>
  <div class="cloud-manager">
    <!-- 左侧：云存储列表（一个用户可配多套） -->
    <div class="cm-sidebar">
      <div class="cm-sidebar-header">
        <span>{{ t('filesCloud.storeListTitle') }}</span>
        <el-button
          :icon="Plus"
          size="small"
          text
          :title="t('filesCloud.addStore')"
          @click="openStoreDialog()"
        />
      </div>
      <el-scrollbar class="cm-store-scroll">
        <div v-loading="storeLoading" class="cm-store-list">
          <div
            v-for="store in stores"
            :key="store.id"
            class="cm-store-item"
            :class="{ 'is-active': store.id === activeStoreId }"
            @click="selectStore(store)"
          >
            <el-icon :size="18" class="cm-store-icon"><Cloud /></el-icon>
            <div class="cm-store-info">
              <div class="cm-store-name" :title="store.name">{{ store.name }}</div>
              <div class="cm-store-meta" :title="`${store.service_label} · ${store.bucket}`">
                {{ store.service_label }} · {{ store.bucket }}
              </div>
            </div>
            <el-dropdown trigger="click" @command="(cmd: string) => onStoreCommand(cmd, store)">
              <el-icon class="cm-store-more" @click.stop><MoreFilled /></el-icon>
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item command="test">
                    {{ t('filesCloud.testConn') }}
                  </el-dropdown-item>
                  <el-dropdown-item command="edit">
                    {{ t('filesCloud.editConfig') }}
                  </el-dropdown-item>
                  <el-dropdown-item command="delete" divided>
                    {{ t('filesCloud.deleteConfig') }}
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>

          <el-empty
            v-if="!storeLoading && stores.length === 0"
            :description="t('filesCloud.noStore')"
            :image-size="64"
          >
            <el-button type="primary" size="small" :icon="Plus" @click="openStoreDialog()">
              {{ t('filesCloud.addStore') }}
            </el-button>
          </el-empty>
        </div>
      </el-scrollbar>
    </div>

    <!-- 右侧：对象浏览 -->
    <div class="cm-main">
      <template v-if="activeStore">
        <div class="cm-toolbar">
          <div class="cm-toolbar-left">
            <!-- 地址栏：桶根用云图标表示，层级用 > 分隔（与本地存储一致） -->
            <el-breadcrumb separator=">" class="cm-crumbs">
              <el-breadcrumb-item>
                <a
                  href="javascript:void(0)"
                  :class="{ 'is-last': !currentPath }"
                  :title="t('filesCloud.bucketRoot')"
                  @click="navigateTo('')"
                >
                  <el-icon :size="14" class="cm-crumb-icon"><Cloud /></el-icon>
                </a>
              </el-breadcrumb-item>
              <el-breadcrumb-item v-for="(seg, idx) in pathSegments" :key="seg.path">
                <a
                  href="javascript:void(0)"
                  :class="{ 'is-last': idx === pathSegments.length - 1 }"
                  :title="seg.path"
                  @click="navigateTo(seg.path)"
                >
                  {{ seg.name }}
                </a>
              </el-breadcrumb-item>
            </el-breadcrumb>
          </div>
          <div class="cm-toolbar-right">
            <el-button size="small" @click="openUploadDialog">
              <el-icon><Upload /></el-icon>
              {{ t('filesCloud.upload') }}
            </el-button>
            <el-button size="small" @click="showMkdirDialog">
              <el-icon><FolderAdd /></el-icon>
              {{ t('filesCloud.newDir') }}
            </el-button>
            <el-button size="small" :loading="fileLoading" @click="loadFiles">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </div>
        </div>

        <div class="cm-sub-toolbar">
          <span class="cm-path-hint" :title="storeLocation">
            {{ storeLocation }}{{ currentPath ? '/' + currentPath : '' }}
          </span>
          <el-checkbox v-model="showHidden" size="small" @change="loadFiles">
            {{ t('filesCloud.showHidden') }}
          </el-checkbox>
        </div>

        <div class="cm-table-wrap">
          <el-table
            :data="entries"
            v-loading="fileLoading"
            stripe
            highlight-current-row
            style="width: 100%"
            @row-dblclick="onRowDblClick"
          >
            <el-table-column :label="t('common.name')" min-width="300">
              <template #default="{ row }">
                <div class="cm-file-name" @click="onNameClick(row)">
                  <el-icon
                    :size="18"
                    :color="
                      row.is_dir ? 'var(--el-color-primary)' : 'var(--el-text-color-secondary)'
                    "
                  >
                    <Folder v-if="row.is_dir" />
                    <Document v-else />
                  </el-icon>
                  <span>{{ row.name }}</span>
                </div>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.size')" width="120" align="right">
              <template #default="{ row }">
                <span v-if="!row.is_dir">{{ formatSize(row.size) }}</span>
                <span v-else class="cm-muted">-</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('filesCloud.modified')" width="180">
              <template #default="{ row }">
                <span class="cm-muted">{{ formatTime(row.modified) }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('filesCloud.actions')" width="180" align="right">
              <template #default="{ row }">
                <el-button
                  v-if="!row.is_dir"
                  link
                  type="primary"
                  size="small"
                  @click="downloadEntry(row)"
                >
                  {{ t('filesCloud.download') }}
                </el-button>
                <el-button link type="primary" size="small" @click="showRenameDialog(row)">
                  {{ t('filesCloud.rename') }}
                </el-button>
                <el-button link type="danger" size="small" @click="doDelete(row)">
                  {{ t('common.delete') }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>

          <el-empty
            v-if="!fileLoading && entries.length === 0"
            :description="listFailed ? t('filesCloud.listError') : t('filesCloud.emptyDir')"
            :image-size="70"
          />
          <div v-if="truncated" class="cm-truncated">
            {{ t('filesCloud.truncated', { n: entries.length }) }}
          </div>
        </div>
      </template>

      <el-empty v-else :description="t('filesCloud.pickStore')" :image-size="90" />
    </div>

    <!--
      上传：两种来源
      - 从本地上传：文件经浏览器 → 服务端 → 云存储（multipart 流式写入）
      - 从服务器选择：文件已在服务器上，后端读盘直接写入云存储，不经浏览器中转
    -->
    <el-dialog
      v-model="uploadVisible"
      :title="t('filesCloud.uploadTitle')"
      width="780px"
      :close-on-click-modal="false"
      :close-on-press-escape="!uploading"
      :show-close="!uploading"
      destroy-on-close
      @closed="resetUploadDialog"
    >
      <div class="cm-upload-target">
        <span class="cm-upload-target-label">{{ t('filesCloud.targetDir') }}</span>
        <span class="mono">{{ storeLocation }}{{ currentPath ? '/' + currentPath : '' }}</span>
      </div>

      <el-tabs v-model="uploadTab">
        <el-tab-pane :label="t('filesCloud.tabLocal')" name="local" lazy>
          <el-upload
            drag
            multiple
            :auto-upload="false"
            :show-file-list="false"
            :on-change="onLocalPick"
            class="cm-upload-drop"
          >
            <el-icon class="cm-upload-drop-icon"><Upload /></el-icon>
            <div class="cm-upload-drop-text">
              {{ t('filesCloud.dropTextPrefix') }}<em>{{ t('filesCloud.dropAction') }}</em>
            </div>
            <div class="cm-upload-drop-tip">{{ t('filesCloud.dropTip') }}</div>
          </el-upload>
        </el-tab-pane>

        <el-tab-pane :label="t('filesCloud.tabServer')" name="server" lazy>
          <div class="cm-server-bar">
            <el-breadcrumb separator=">" class="cm-server-crumbs">
              <el-breadcrumb-item>
                <a href="javascript:void(0)" @click="loadServerDir(serverHome)">
                  {{ t('filesCloud.home') }}
                </a>
              </el-breadcrumb-item>
              <el-breadcrumb-item v-for="seg in serverSegments" :key="seg.path">
                <a href="javascript:void(0)" @click="loadServerDir(seg.path)">{{ seg.name }}</a>
              </el-breadcrumb-item>
            </el-breadcrumb>
            <el-button size="small" :loading="serverLoading" @click="loadServerDir(serverPath)">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </div>

          <el-table
            ref="serverTableRef"
            :data="serverEntries"
            v-loading="serverLoading"
            height="240"
            size="small"
            stripe
            @row-dblclick="onServerRowDblClick"
            @selection-change="onServerSelectionChange"
          >
            <el-table-column type="selection" width="42" :selectable="selectableServerRow" />
            <el-table-column :label="t('common.name')" min-width="240">
              <template #default="{ row }">
                <div class="cm-file-name" @click="onServerNameClick(row)">
                  <el-icon
                    :size="16"
                    :color="
                      row.is_dir ? 'var(--el-color-primary)' : 'var(--el-text-color-secondary)'
                    "
                  >
                    <Folder v-if="row.is_dir" />
                    <Document v-else />
                  </el-icon>
                  <span>{{ row.name }}</span>
                </div>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.size')" width="100" align="right">
              <template #default="{ row }">
                <span v-if="!row.is_dir">{{ formatSize(row.size) }}</span>
                <span v-else class="cm-muted">-</span>
              </template>
            </el-table-column>
          </el-table>

          <div class="cm-server-foot">
            <span class="cm-server-tip">
              {{ t('filesCloud.serverTip', { n: serverChecked.length }) }}
            </span>
            <el-button
              size="small"
              type="primary"
              plain
              :disabled="!serverChecked.length"
              @click="addServerFiles"
            >
              {{ t('filesCloud.addToPending') }}
            </el-button>
          </div>
        </el-tab-pane>
      </el-tabs>

      <!-- 待上传清单：两种来源合并在一起，每个文件标注来源 -->
      <div class="cm-pending">
        <div class="cm-pending-head">
          <span>{{ t('filesCloud.pendingTitle', { n: pendingList.length }) }}</span>
          <el-button
            link
            type="primary"
            size="small"
            :disabled="uploading || !pendingList.length"
            @click="clearPending"
          >
            {{ t('filesCloud.clearAll') }}
          </el-button>
        </div>
        <el-scrollbar max-height="132px">
          <div v-if="!pendingList.length" class="cm-pending-empty">
            {{ t('filesCloud.pendingEmpty') }}
          </div>
          <div v-for="item in pendingList" :key="item.key" class="cm-pending-item">
            <el-icon :size="14" class="cm-pending-icon"><Document /></el-icon>
            <span class="cm-pending-name" :title="item.name">{{ item.name }}</span>
            <span class="cm-pending-src">
              {{
                item.source === 'local' ? t('filesCloud.sourceLocal') : t('filesCloud.sourceServer')
              }}
            </span>
            <span class="cm-pending-size">{{ formatSize(item.size) }}</span>
            <el-button
              link
              type="danger"
              size="small"
              :disabled="uploading"
              @click="removePending(item.key)"
            >
              {{ t('filesCloud.removeItem') }}
            </el-button>
          </div>
        </el-scrollbar>
      </div>

      <div v-if="uploading || uploadSummary" class="cm-upload-foot">
        <el-progress :percentage="uploadPercent" :stroke-width="6" />
        <div class="cm-upload-summary">{{ uploadSummary }}</div>
      </div>

      <template #footer>
        <el-button :disabled="uploading" @click="uploadVisible = false">
          {{ t('common.cancel') }}
        </el-button>
        <el-button
          type="primary"
          :loading="uploading"
          :disabled="!pendingList.length"
          @click="doUpload"
        >
          {{ t('filesCloud.startUpload') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 新建目录 -->
    <el-dialog v-model="mkdirVisible" :title="t('filesCloud.newDir')" width="420px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesCloud.dirName')">
          <el-input
            v-model="mkdirName"
            :placeholder="t('filesCloud.dirNamePlaceholder')"
            @keydown.enter.prevent="onEnterConfirm($event, doMkdir)"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="mkdirVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doMkdir">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 重命名 -->
    <el-dialog v-model="renameVisible" :title="t('filesCloud.rename')" width="420px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesCloud.newName')">
          <el-input
            v-model="renameName"
            :placeholder="t('filesCloud.newNamePlaceholder')"
            @keydown.enter.prevent="onEnterConfirm($event, doRename)"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="renameVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doRename">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 新增 / 编辑云存储 -->
    <el-dialog
      v-model="storeDialogVisible"
      :title="editingId ? t('filesCloud.editStore') : t('filesCloud.addStore')"
      width="640px"
      :close-on-click-modal="false"
    >
      <el-form
        ref="storeFormRef"
        :model="storeForm"
        :rules="storeRules"
        label-width="130px"
        @submit.prevent
      >
        <el-form-item :label="t('common.name')" prop="name">
          <el-input
            v-model="storeForm.name"
            :placeholder="t('filesCloud.namePlaceholder')"
            maxlength="40"
          />
        </el-form-item>

        <el-form-item :label="t('filesCloud.serviceType')" prop="service">
          <el-select v-model="storeForm.service" style="width: 100%" @change="onServiceChange">
            <el-option v-for="p in presets" :key="p.id" :label="p.label" :value="p.id" />
          </el-select>
        </el-form-item>

        <el-form-item label="Endpoint" prop="endpoint" :required="currentPreset?.endpoint_required">
          <el-input
            v-model="storeForm.endpoint"
            :placeholder="currentPreset?.endpoint_hint || 'https://s3.example.com'"
          />
        </el-form-item>

        <el-form-item label="Region" prop="region" :required="currentPreset?.region_required">
          <el-input v-model="storeForm.region" :placeholder="t('filesCloud.regionPlaceholder')" />
        </el-form-item>

        <el-form-item label="Bucket" prop="bucket">
          <el-input v-model="storeForm.bucket" :placeholder="t('filesCloud.bucketPlaceholder')" />
        </el-form-item>

        <el-form-item :label="t('filesCloud.rootLabel')" prop="root">
          <el-input v-model="storeForm.root" :placeholder="t('filesCloud.rootPlaceholder')" />
        </el-form-item>

        <el-form-item :label="t('filesCloud.accessMode')">
          <el-switch v-model="storeForm.virtual_host_style" />
          <span class="cm-form-tip">
            {{
              storeForm.virtual_host_style ? t('filesCloud.vhostStyle') : t('filesCloud.pathStyle')
            }}
          </span>
        </el-form-item>

        <el-divider content-position="left">{{ t('filesCloud.credentials') }}</el-divider>

        <el-form-item label="AccessKey ID" prop="access_key_id">
          <el-input
            v-model="storeForm.access_key_id"
            :placeholder="
              editingId
                ? t('filesCloud.keyKeepWithHint', {
                    hint: storeForm.access_key_hint || t('filesCloud.saved'),
                  })
                : 'AccessKey ID'
            "
            autocomplete="off"
          />
        </el-form-item>

        <el-form-item label="AccessKey Secret" prop="secret_access_key">
          <el-input
            v-model="storeForm.secret_access_key"
            type="password"
            show-password
            :placeholder="editingId ? t('filesCloud.keyKeep') : 'AccessKey Secret'"
            autocomplete="new-password"
          />
        </el-form-item>

        <el-form-item :label="t('filesCloud.securityToken')">
          <el-input
            v-model="storeForm.security_token"
            :placeholder="t('filesCloud.securityTokenPlaceholder')"
          />
        </el-form-item>

        <el-alert type="info" :closable="false" show-icon :title="t('filesCloud.secretNote')" />
      </el-form>
      <template #footer>
        <el-button @click="storeDialogVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="storeSaving" @click="doSaveStore">
          {{ t('common.save') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
/**
 * 云存储面板：左侧多套存储配置，右侧对象浏览 / 上传 / 下载 / 重命名 / 删除。
 *
 * - 前端不碰密钥明文：新增或编辑时提交，接口只回脱敏提示（`LTAI****cdef`）；
 *   编辑时密钥留空 = 沿用原值（后端按「未提交即不变」处理）。
 * - 上传走 el-upload 自定义请求，逐个文件流式写入，工具栏显示进度条。
 * - 对象存储没有真正的目录：目录是「以 / 结尾的路径 + 零字节占位对象」，
 *   这里的路径始终以 `/` 结尾表示目录，删除/重命名都由后端按此规则处理。
 */
import { computed, nextTick, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance, TableInstance, UploadFile } from 'element-plus'
import { useI18n } from 'vue-i18n'

import { Cloud, Document, Folder, FolderAdd, MoreFilled, Plus, Refresh, Upload } from '@/icons'
import {
  cloudDelete,
  cloudMkdir,
  cloudRename,
  cloudUpload,
  cloudUploadLocal,
  deleteCloudStore,
  downloadCloudFile,
  listCloudFiles,
  listCloudStores,
  listLocalFiles,
  saveCloudStore,
  testCloudStore,
  type CloudEntry,
  type CloudServicePreset,
  type CloudStore,
  type CloudStoreInput,
} from '@/api/cloud'
import type { FileEntry } from '@/api/file'

const { t } = useI18n()

// ── state ──────────────────────────────────────────────────

const storeLoading = ref(false)
const stores = ref<CloudStore[]>([])
const presets = ref<CloudServicePreset[]>([])
const activeStoreId = ref('')

const fileLoading = ref(false)
const entries = ref<CloudEntry[]>([])
const currentPath = ref('')
const truncated = ref(false)
/** 上一次列目录是否失败，用于把「空目录」和「读失败」区分开 */
const listFailed = ref(false)
const showHidden = ref(false)

// ── 上传弹窗 ────────────────────────────────────────────────

/** 待上传项：两种来源（本地文件 / 服务器路径）合并成一个清单 */
interface PendingUpload {
  /** 去重键：本地按「名字+大小+修改时间」，服务器按绝对路径 */
  key: string
  name: string
  size: number
  source: 'local' | 'server'
  file?: File
  localPath?: string
}

const uploadVisible = ref(false)
const uploadTab = ref<'local' | 'server'>('local')
const pendingList = ref<PendingUpload[]>([])
const uploading = ref(false)
const uploadPercent = ref(0)
const uploadSummary = ref('')
/** el-upload 内部累积条目的 uid：同一次选择里防止重复入清单 */
const localSeenUids = new Set<number>()

const serverLoading = ref(false)
const serverEntries = ref<FileEntry[]>([])
/** 当前浏览的服务器目录（绝对路径） */
const serverPath = ref('')
/** 当前用户家目录：面包屑根节点 */
const serverHome = ref('')
const serverChecked = ref<FileEntry[]>([])
const serverTableRef = ref<TableInstance>()

const mkdirVisible = ref(false)
const mkdirName = ref('')

const renameVisible = ref(false)
const renameName = ref('')
let renameTarget: CloudEntry | null = null

const storeDialogVisible = ref(false)
const storeSaving = ref(false)
const editingId = ref('')
const storeFormRef = ref<FormInstance>()
const storeForm = reactive({
  name: '',
  service: 'aws_s3',
  endpoint: '',
  region: '',
  bucket: '',
  root: '',
  virtual_host_style: true,
  access_key_id: '',
  secret_access_key: '',
  security_token: '',
  /** 仅用于编辑时的占位提示，不提交 */
  access_key_hint: '',
})

// ── computed ───────────────────────────────────────────────

const activeStore = computed(() => stores.value.find((s) => s.id === activeStoreId.value) || null)

const currentPreset = computed(() => presets.value.find((p) => p.id === storeForm.service) || null)

/** 面包屑：把 `a/b/c` 拆成逐级可点的段落 */
const pathSegments = computed(() => {
  const segs = currentPath.value.split('/').filter(Boolean)
  return segs.map((name, idx) => ({ name, path: segs.slice(0, idx + 1).join('/') }))
})

/** 当前存储的位置提示：服务 · bucket/root（让人一眼知道在看哪个桶） */
const storeLocation = computed(() => {
  const store = activeStore.value
  if (!store) return ''
  const root = store.root ? `/${store.root}` : ''
  return `${store.service_label} · ${store.bucket}${root}`
})

/** 服务器目录面包屑：家目录为根，家目录之外（admin 浏览系统目录）退回根路径展开 */
const serverSegments = computed(() => {
  const home = serverHome.value.replace(/\/+$/, '')
  const current = serverPath.value.replace(/\/+$/, '')
  if (!current || current === home) return []
  const underHome = !!home && current.startsWith(`${home}/`)
  const rel = underHome ? current.slice(home.length + 1) : current.replace(/^\//, '')
  const parts = rel.split('/').filter(Boolean)
  const base = underHome ? home : ''
  return parts.map((name, idx) => ({
    name,
    path: `${base}/${parts.slice(0, idx + 1).join('/')}`,
  }))
})

/** 表单校验：必填项跟随所选服务类型（与后端保存时的校验保持一致） */
const storeRules = computed(() => ({
  name: [{ required: true, message: t('filesCloud.ruleName'), trigger: 'blur' }],
  service: [{ required: true, message: t('filesCloud.ruleService'), trigger: 'change' }],
  bucket: [{ required: true, message: t('filesCloud.ruleBucket'), trigger: 'blur' }],
  endpoint: currentPreset.value?.endpoint_required
    ? [{ required: true, message: t('filesCloud.ruleEndpoint'), trigger: 'blur' }]
    : [],
  region: currentPreset.value?.region_required
    ? [{ required: true, message: t('filesCloud.ruleRegion'), trigger: 'blur' }]
    : [],
  // 编辑时密钥留空表示沿用，不做必填
  access_key_id: editingId.value
    ? []
    : [{ required: true, message: t('filesCloud.ruleAccessKey'), trigger: 'blur' }],
  secret_access_key: editingId.value
    ? []
    : [{ required: true, message: t('filesCloud.ruleSecret'), trigger: 'blur' }],
}))

// ── 存储配置 ────────────────────────────────────────────────

async function loadStores(preferId?: string) {
  storeLoading.value = true
  try {
    const res = await listCloudStores()
    stores.value = res.data?.stores || []
    presets.value = res.data?.services || []

    // 选中项：优先刚保存的 → 当前选中 → 第一个
    const wanted = preferId || activeStoreId.value
    const next = stores.value.find((s) => s.id === wanted) || stores.value[0]
    if (!next) {
      activeStoreId.value = ''
      entries.value = []
      currentPath.value = ''
      return
    }
    if (next.id !== activeStoreId.value) {
      activeStoreId.value = next.id
      currentPath.value = ''
    }
    await loadFiles()
  } catch (e) {
    notifyError(e, t('filesCloud.errLoadStores'))
  } finally {
    storeLoading.value = false
  }
}

function selectStore(store: CloudStore) {
  if (store.id !== activeStoreId.value) {
    activeStoreId.value = store.id
    currentPath.value = ''
  }
  loadFiles()
}

function onStoreCommand(command: string, store: CloudStore) {
  if (command === 'test') return testStore(store)
  if (command === 'edit') return openStoreDialog(store)
  if (command === 'delete') return doDeleteStore(store)
}

async function testStore(store: CloudStore) {
  const notice = ElMessage({ message: t('filesCloud.testing', { name: store.name }), duration: 0 })
  try {
    const res = await testCloudStore(store.id)
    ElMessage.success(res.message || t('filesCloud.testOk'))
  } catch (e) {
    // 失败原因来自后端（鉴权 / endpoint / 桶不存在 / 网络），必须显式提示
    notifyError(e, t('filesCloud.testFailed', { name: store.name }))
  } finally {
    notice.close()
  }
}

function openStoreDialog(store?: CloudStore) {
  editingId.value = store?.id || ''
  storeForm.name = store?.name || ''
  storeForm.service = store?.service || presets.value[0]?.id || 'aws_s3'
  storeForm.endpoint = store?.endpoint || ''
  storeForm.region = store?.region || ''
  storeForm.bucket = store?.bucket || ''
  storeForm.root = store?.root || ''
  storeForm.virtual_host_style =
    store?.virtual_host_style ?? currentPreset.value?.virtual_host_default ?? true
  storeForm.access_key_id = ''
  storeForm.secret_access_key = ''
  storeForm.security_token = ''
  storeForm.access_key_hint = store?.access_key_hint || ''
  storeDialogVisible.value = true
  nextTick(() => storeFormRef.value?.clearValidate())
}

function onServiceChange() {
  // 切换服务类型时按预设重置访问方式（S3 兼容默认 path 样式）
  storeForm.virtual_host_style = currentPreset.value?.virtual_host_default ?? true
}

async function doSaveStore() {
  const form = storeFormRef.value
  if (!form) return
  try {
    await form.validate()
  } catch {
    return // 校验失败，字段已标红
  }

  const payload: CloudStoreInput = {
    id: editingId.value || undefined,
    name: storeForm.name.trim(),
    service: storeForm.service,
    endpoint: storeForm.endpoint.trim(),
    region: storeForm.region.trim(),
    bucket: storeForm.bucket.trim(),
    root: storeForm.root.trim(),
    virtual_host_style: storeForm.virtual_host_style,
    access_key_id: storeForm.access_key_id.trim(),
    secret_access_key: storeForm.secret_access_key.trim(),
    security_token: storeForm.security_token.trim(),
  }

  storeSaving.value = true
  try {
    const res = await saveCloudStore(payload)
    ElMessage.success(editingId.value ? t('filesCloud.storeUpdated') : t('filesCloud.storeCreated'))
    storeDialogVisible.value = false
    await loadStores(res.data?.store?.id)
  } catch (e) {
    notifyError(e, t('filesCloud.errSaveStore'))
  } finally {
    storeSaving.value = false
  }
}

async function doDeleteStore(store: CloudStore) {
  try {
    await ElMessageBox.confirm(
      t('filesCloud.deleteStoreConfirm', { name: store.name }),
      t('filesCloud.deleteStoreTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteCloudStore(store.id)
    ElMessage.success(t('filesCloud.storeDeleted'))
    if (activeStoreId.value === store.id) {
      activeStoreId.value = ''
      entries.value = []
      currentPath.value = ''
    }
    await loadStores()
  } catch (e) {
    notifyError(e, t('filesCloud.errDeleteStore'))
  }
}

// ── 对象浏览 ────────────────────────────────────────────────

async function loadFiles() {
  const store = activeStore.value
  if (!store) return
  fileLoading.value = true
  listFailed.value = false
  try {
    const res = await listCloudFiles(store.id, currentPath.value, showHidden.value)
    entries.value = res.data?.entries || []
    currentPath.value = res.data?.current_path ?? currentPath.value
    truncated.value = !!res.data?.truncated
  } catch (e) {
    entries.value = []
    truncated.value = false
    // 只留一个「当前目录为空」会把「没权限 / 桶不存在」说成「桶是空的」，这里区分开
    listFailed.value = true
    notifyError(e, t('filesCloud.errListDir'))
  } finally {
    fileLoading.value = false
  }
}

function navigateTo(path: string) {
  currentPath.value = path
  loadFiles()
}

function onNameClick(row: CloudEntry) {
  if (row.is_dir) navigateTo(row.path)
}

function onRowDblClick(row: CloudEntry) {
  if (row.is_dir) navigateTo(row.path)
  else downloadEntry(row)
}

async function downloadEntry(row: CloudEntry) {
  try {
    const blob = await downloadCloudFile(activeStoreId.value, row.path)
    const url = window.URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = row.name
    a.click()
    window.URL.revokeObjectURL(url)
  } catch (e) {
    notifyError(e, t('filesCloud.errDownload', { name: row.name }))
  }
}

// ── 上传（从本地上传 / 从服务器选择）────────────────────────

function openUploadDialog() {
  if (!activeStore.value) {
    ElMessage.warning(t('filesCloud.needStore'))
    return
  }
  uploadTab.value = 'local'
  pendingList.value = []
  uploadPercent.value = 0
  uploadSummary.value = ''
  localSeenUids.clear()
  uploadVisible.value = true
  // 顺手把服务器侧的家目录读出来：切到「从服务器选择」页签即可用
  loadServerDir('')
}

function resetUploadDialog() {
  pendingList.value = []
  localSeenUids.clear()
  uploadPercent.value = 0
  uploadSummary.value = ''
  serverEntries.value = []
  serverChecked.value = []
  serverPath.value = ''
}

function clearPending() {
  pendingList.value = []
}

function removePending(key: string) {
  pendingList.value = pendingList.value.filter((item) => item.key !== key)
}

/** 入清单：按 key 去重，重复选择只提示一次 */
function addPending(item: PendingUpload): boolean {
  if (pendingList.value.some((i) => i.key === item.key)) {
    ElMessage.info(t('filesCloud.alreadyPending', { name: item.name }))
    return false
  }
  pendingList.value.push(item)
  return true
}

/** 本地文件由 el-upload 收集（关闭自动上传，选中即入清单） */
function onLocalPick(file: UploadFile) {
  const raw = file.raw
  if (!raw) return
  if (localSeenUids.has(file.uid)) return
  localSeenUids.add(file.uid)
  addPending({
    key: `local:${raw.name}:${raw.size}:${raw.lastModified}`,
    name: raw.name,
    size: raw.size,
    source: 'local',
    file: raw,
  })
}

// ── 服务器文件浏览 ──────────────────────────────────────────

async function loadServerDir(path: string) {
  serverLoading.value = true
  try {
    const res = await listLocalFiles(path)
    const data = res.data
    serverEntries.value = data?.entries || []
    serverPath.value = data?.current_path || ''
    if (data?.home) serverHome.value = data.home
    serverChecked.value = []
    serverTableRef.value?.clearSelection()
  } catch (e) {
    serverEntries.value = []
    notifyError(e, t('filesCloud.errServerDir'))
  } finally {
    serverLoading.value = false
  }
}

/** 目录不参与上传（对象存储没有目录概念），选择列里禁掉 */
function selectableServerRow(row: FileEntry) {
  return !row.is_dir
}

function onServerSelectionChange(rows: FileEntry[]) {
  serverChecked.value = rows
}

function onServerNameClick(row: FileEntry) {
  if (row.is_dir) loadServerDir(row.path)
}

function onServerRowDblClick(row: FileEntry) {
  if (row.is_dir) loadServerDir(row.path)
}

function addServerFiles() {
  let added = 0
  for (const row of serverChecked.value) {
    const ok = addPending({
      key: `server:${row.path}`,
      name: row.name,
      size: row.size,
      source: 'server',
      localPath: row.path,
    })
    if (ok) added++
  }
  serverChecked.value = []
  serverTableRef.value?.clearSelection()
  if (added) ElMessage.success(t('filesCloud.serverAdded', { n: added }))
}

// ── 开始上传 ────────────────────────────────────────────────

/** 并发上限：本地走 multipart、服务器走后端直传，一次全打出去对两边都不友好 */
const UPLOAD_CONCURRENCY = 3

async function doUpload() {
  const store = activeStore.value
  const list = [...pendingList.value]
  if (!store || !list.length) return

  uploading.value = true
  uploadPercent.value = 0
  uploadSummary.value = t('filesCloud.uploadingProgress', { done: 0, total: list.length })

  const queue = [...list]
  const succeeded: string[] = []
  const failed: string[] = []
  let finished = 0

  const worker = async () => {
    for (;;) {
      const item = queue.shift()
      if (!item) return
      try {
        if (item.source === 'local' && item.file) {
          await cloudUpload(store.id, currentPath.value, [item.file])
        } else if (item.localPath) {
          const res = await cloudUploadLocal(store.id, currentPath.value, [item.localPath])
          // 后端逐个文件处理，失败项随响应返回（单个失败不影响其余文件）
          if (res.data?.failed?.length) throw new Error(res.data.failed.join('；'))
        } else {
          throw new Error(t('filesCloud.errNoSource'))
        }
        succeeded.push(item.key)
      } catch (e) {
        failed.push(item.name)
        // 失败原因（越权 / 读不到 / 写入失败）都由这里显式提示
        notifyError(e, t('filesCloud.errUpload', { name: item.name }))
      } finally {
        finished++
        uploadPercent.value = Math.round((finished / list.length) * 100)
        uploadSummary.value = t('filesCloud.uploadingProgress', {
          done: finished,
          total: list.length,
        })
      }
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(UPLOAD_CONCURRENCY, queue.length) }, () => worker()),
  )

  uploading.value = false
  // 只保留失败的项：改好后可直接再点一次「开始上传」重试
  const doneSet = new Set(succeeded)
  pendingList.value = pendingList.value.filter((item) => !doneSet.has(item.key))

  uploadSummary.value = failed.length
    ? t('filesCloud.uploadDoneFailed', {
        ok: succeeded.length,
        failed: failed.length,
      })
    : t('filesCloud.uploadDone', { n: succeeded.length })

  if (!failed.length) {
    ElMessage.success(t('filesCloud.uploadDone', { n: succeeded.length }))
    uploadVisible.value = false
  }
  loadFiles()
}

function showMkdirDialog() {
  mkdirName.value = ''
  mkdirVisible.value = true
}

async function doMkdir() {
  const name = mkdirName.value.trim()
  if (!name) return ElMessage.warning(t('filesCloud.needDirName'))
  if (name.includes('/')) return ElMessage.warning(t('filesCloud.noSlash'))

  const path = joinPath(currentPath.value, name)
  try {
    await cloudMkdir(activeStoreId.value, path)
    ElMessage.success(t('filesCloud.dirCreated'))
    mkdirVisible.value = false
    loadFiles()
  } catch (e) {
    notifyError(e, t('filesCloud.errMkdir', { name }))
  }
}

function showRenameDialog(row: CloudEntry) {
  renameTarget = row
  renameName.value = row.name
  renameVisible.value = true
}

async function doRename() {
  const row = renameTarget
  if (!row) return
  const name = renameName.value.trim()
  if (!name) return ElMessage.warning(t('filesCloud.needNewName'))
  if (name.includes('/')) return ElMessage.warning(t('filesCloud.noSlash'))
  if (name === row.name) {
    renameVisible.value = false
    return
  }

  // 同目录内改名：父路径不变，目录路径保留结尾的 `/`
  const parent = parentOf(row.path)
  const target = joinPath(parent, name)
  try {
    await cloudRename(activeStoreId.value, row.path, row.is_dir ? `${target}/` : target)
    ElMessage.success(t('filesCloud.renamed'))
    renameVisible.value = false
    loadFiles()
  } catch (e) {
    notifyError(e, t('filesCloud.errRename', { name: row.name }))
  }
}

async function doDelete(row: CloudEntry) {
  const tip = row.is_dir
    ? t('filesCloud.deleteDirConfirm', { name: row.name })
    : t('filesCloud.deleteFileConfirm', { name: row.name })
  try {
    await ElMessageBox.confirm(
      tip,
      row.is_dir ? t('filesCloud.deleteDirTitle') : t('filesCloud.deleteFileTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await cloudDelete(activeStoreId.value, row.path)
    ElMessage.success(t('filesCloud.deleted'))
    loadFiles()
  } catch (e) {
    notifyError(e, t('filesCloud.errDelete', { name: row.name }))
  }
}

// ── 错误提示 ────────────────────────────────────────────────

/**
 * 统一提示云存储操作的失败原因。
 *
 * 后端业务失败是 HTTP 200 + `{ code, message }` 返回的，而响应拦截器对业务错误
 * 只 reject 不弹窗（见 `utils/request.ts`），所以每个 catch 都必须自己把 message
 * 显示出来；否则「鉴权失败 / endpoint 或 region 填错 / 桶不存在 / 网络不通」这些
 * 原因会被完全吞掉，界面上只剩下一个空目录，无从排查。
 */
function notifyError(e: unknown, fallback: string) {
  const message = (e as { message?: string } | null)?.message
  // 云存储的报错来自 opendal，通常较长（含状态码与请求上下文），给足阅读时间
  ElMessage({
    message: message || fallback,
    type: 'error',
    duration: 8000,
    showClose: true,
  })
}

// ── utils ──────────────────────────────────────────────────

/** 回车即确认；中文输入法组字中的回车只用于上屏，不提交 */
function onEnterConfirm(event: KeyboardEvent, action: () => void) {
  if (event.isComposing || event.keyCode === 229) return
  action()
}

function joinPath(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name
}

/** 目录路径以 `/` 结尾，取父路径时要先剥掉 */
function parentOf(path: string): string {
  const clean = path.replace(/\/+$/, '')
  const idx = clean.lastIndexOf('/')
  return idx === -1 ? '' : clean.slice(0, idx)
}

function formatSize(bytes: number): string {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let idx = 0
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024
    idx += 1
  }
  return `${value.toFixed(idx === 0 ? 0 : 1)} ${units[idx]}`
}

/** 后端给的是 RFC3339 字符串，这里按本地时区展示 */
function formatTime(value: string): string {
  if (!value) return '-'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  const pad = (n: number) => String(n).padStart(2, '0')
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  )
}

// ── lifecycle ──────────────────────────────────────────────

onMounted(() => {
  loadStores()
})
</script>

<style scoped lang="scss">
.cloud-manager {
  display: flex;
  height: 100%;
  background: var(--el-bg-color);
  border-radius: 4px;
  overflow: hidden;
}

/* ── 左侧存储列表 ─────────────────────────────────────── */

.cm-sidebar {
  width: 240px;
  min-width: 200px;
  border-right: 1px solid var(--el-border-color-lighter);
  display: flex;
  flex-direction: column;
}

.cm-sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  font-size: 13px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.cm-store-scroll {
  flex: 1;
  min-height: 0;
}

.cm-store-list {
  padding: 6px;
  min-height: 120px;
}

.cm-store-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
  transition: background-color 0.2s;

  &:hover {
    background: var(--el-fill-color-light);
  }

  &.is-active {
    background: var(--el-color-primary-light-9);
  }
}

.cm-store-icon {
  color: var(--el-color-primary);
  flex-shrink: 0;
}

.cm-store-info {
  flex: 1;
  min-width: 0;
}

.cm-store-name {
  font-size: 13px;
  color: var(--el-text-color-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.cm-store-meta {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.cm-store-more {
  color: var(--el-text-color-secondary);
  cursor: pointer;
  flex-shrink: 0;

  &:hover {
    color: var(--el-color-primary);
  }
}

/* ── 右侧对象区 ───────────────────────────────────────── */

.cm-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.cm-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.cm-toolbar-left {
  min-width: 0;
  overflow: hidden;

  /* 层级用 > 分隔；每段单独截断，深目录也不会把工具栏挤变形 */
  :deep(.el-breadcrumb__inner) {
    display: inline-flex;
    align-items: center;
    max-width: 220px;
  }

  :deep(.el-breadcrumb__inner a) {
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;

    &.is-last {
      color: var(--el-text-color-primary);
      cursor: default;
    }
  }

  :deep(.el-breadcrumb__separator) {
    margin: 0 6px;
    font-weight: 400;
    color: var(--el-text-color-placeholder);
  }
}

/* 桶根段只有图标：别被基线挤偏 */
.cm-crumb-icon {
  vertical-align: middle;
}

.cm-toolbar-right {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.cm-sub-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 12px;
  background: var(--el-fill-color-lighter);
}

.cm-path-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ── 上传弹窗 ─────────────────────────────────────────── */

.cm-upload-target {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: -4px 0 8px;
  font-size: 13px;
  color: var(--el-text-color-regular);

  &-label {
    flex: none;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--el-fill-color-light);
    color: var(--el-text-color-secondary);
    font-size: 12px;
  }
}

.cm-upload-drop {
  :deep(.el-upload-dragger) {
    padding: 18px;
  }
}

.cm-upload-drop-icon {
  font-size: 34px;
  color: var(--el-text-color-placeholder);
}

.cm-upload-drop-text {
  margin-top: 6px;
  font-size: 13px;
  color: var(--el-text-color-regular);

  em {
    color: var(--el-color-primary);
    font-style: normal;
  }
}

.cm-upload-drop-tip {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.cm-server-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.cm-server-crumbs {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
}

.cm-server-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 8px;
}

.cm-server-tip {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.cm-pending {
  margin-top: 12px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
}

.cm-pending-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  font-size: 13px;
  color: var(--el-text-color-regular);
  background: var(--el-fill-color-lighter);
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.cm-pending-empty {
  padding: 14px 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  text-align: center;
}

.cm-pending-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  font-size: 13px;

  & + & {
    border-top: 1px solid var(--el-border-color-lighter);
  }
}

.cm-pending-icon {
  flex: none;
  color: var(--el-text-color-secondary);
}

.cm-pending-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.cm-pending-src {
  flex: none;
  padding: 0 6px;
  border-radius: 4px;
  background: var(--el-fill-color-light);
  color: var(--el-text-color-secondary);
  font-size: 12px;
}

.cm-pending-size {
  flex: none;
  width: 80px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  text-align: right;
}

.cm-upload-foot {
  margin-top: 12px;
}

.cm-upload-summary {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.cm-table-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 0 12px 12px;
}

.cm-file-name {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}

.cm-muted {
  color: var(--el-text-color-secondary);
}

.cm-truncated {
  padding: 8px 0;
  font-size: 12px;
  color: var(--el-color-warning);
}

.cm-form-tip {
  margin-left: 10px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
