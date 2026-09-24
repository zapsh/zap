<template>
  <!--
    非模态浮窗：Teleport 到 body（文件管理自身 overflow:hidden，且浮窗要能盖住整个视口）。
    生命周期由父组件（文件管理）控制：父级 v-if 决定挂载/卸载，本组件只管显示、最小化与关闭意图。
  -->
  <Teleport to="body">
    <!-- 最小化态：缩成底部小图标，点图标还原，点 × 关闭 -->
    <div
      v-if="dockShown"
      class="few-dock"
      :title="currentPath || t('fileEditor.title')"
      @click="restore"
    >
      <el-icon class="few-dock-icon"><Edit /></el-icon>
      <span class="few-dock-name">{{ fileName || t('fileEditor.title') }}</span>
      <span v-if="dirty" class="few-dock-dot" :title="t('fileEditor.unsaved')" />
      <el-icon class="few-dock-close" @click.stop="requestClose"><Close /></el-icon>
    </div>

    <div
      v-else-if="shown"
      class="few-window"
      :class="{ 'is-loading': loading }"
      :style="winStyle"
      @mousedown="bringToFront"
    >
      <!-- 顶部工具栏：标题 + 保存/重新加载/最小化/关闭，整条可拖动（按钮区除外） -->
      <div class="few-header" @mousedown="startDrag">
        <div class="few-header-left">
          <el-icon class="few-header-icon"><Edit /></el-icon>
          <span class="few-title">{{ fileName || t('fileEditor.title') }}</span>
          <span v-if="dirty" class="few-dirty-dot" :title="t('fileEditor.unsaved')" />
        </div>
        <div class="few-header-right" @mousedown.stop>
          <el-button
            size="small"
            type="primary"
            :disabled="!currentPath || saving"
            :loading="saving"
            @click="saveFile()"
          >
            <el-icon><Save /></el-icon>
            {{ t('common.save') }}
          </el-button>
          <el-button size="small" :disabled="!currentPath || loading" @click="reloadFile">
            <el-icon><Refresh /></el-icon>
            {{ t('fileEditor.reload') }}
          </el-button>
          <el-button size="small" text :title="t('fileEditor.minimize')" @click="minimized = true">
            <el-icon><Minimize /></el-icon>
          </el-button>
          <el-button size="small" text :title="t('common.close')" @click="requestClose">
            <el-icon><Close /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 中间：左侧文件树 + 右侧编辑器 -->
      <div class="few-body">
        <div class="few-sidebar">
          <div class="few-sidebar-header">
            <span>{{ t('filesLocal.tree') }}</span>
            <el-button
              size="small"
              text
              :title="t('common.refresh')"
              :loading="treeLoading"
              @click="refreshTree"
            >
              <el-icon><Refresh /></el-icon>
            </el-button>
          </div>
          <el-scrollbar class="few-tree-scroll">
            <el-tree
              :data="treeData"
              :props="treeProps"
              :load="loadTreeNode"
              node-key="path"
              lazy
              highlight-current
              :current-node-key="currentPath"
              @node-click="onTreeNodeClick"
            >
              <template #default="{ node, data }">
                <span class="few-tree-node">
                  <el-icon :size="15">
                    <Home v-if="data.icon === 'home'" />
                    <HardDrive v-else-if="data.icon === 'root'" />
                    <FolderOpened v-else-if="node.expanded && data.is_dir" />
                    <Document v-else-if="!data.is_dir" />
                    <Folder v-else />
                  </el-icon>
                  <span class="few-tree-label" :title="data.path">{{ node.label }}</span>
                </span>
              </template>
            </el-tree>
          </el-scrollbar>
        </div>

        <div class="few-editor-area">
          <CodeEditor
            v-if="currentPath"
            v-model="content"
            class="few-editor"
            :path="currentPath"
            :lang="editorLang"
            @cursor="onCursor"
          />
          <div v-else-if="loading" class="few-placeholder">{{ t('common.loading') }}</div>
          <div v-else class="few-placeholder">{{ t('fileEditor.empty') }}</div>
        </div>
      </div>

      <!-- 底部状态栏：光标位置 / 路径 / 语言 / 大小与权限 / 保存状态 -->
      <div class="few-status">
        <span class="few-status-item">{{ t('fileEditor.pos', { line, col }) }}</span>
        <span class="few-status-path" :title="currentPath">{{ currentPath || '—' }}</span>
        <el-select
          v-if="currentPath"
          class="few-lang"
          size="small"
          :model-value="langPick"
          :title="t('fileEditor.lang')"
          @update:model-value="onLangChange"
        >
          <el-option
            v-for="opt in langOptions"
            :key="opt.value"
            :label="opt.label"
            :value="opt.value"
          />
        </el-select>
        <span v-if="meta" class="few-status-item">
          {{ formatSize(meta.size) }} · {{ meta.permissions }}
        </span>
        <span class="few-status-item" :class="dirty ? 'is-dirty' : 'is-saved'">
          {{ dirty ? t('fileEditor.unsaved') : t('fileEditor.savedState') }}
        </span>
      </div>

      <!-- 右下角拉伸手柄 -->
      <div class="few-resize" @mousedown.stop.prevent="startResize" />
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import CodeEditor from '@/components/CodeEditor.vue'
import { getFileInfo, listFiles, readFile, writeFile, type FileEntry } from '@/api/file'
import {
  LANG_OPTIONS,
  langFromPath,
  langLabel,
  type EditorLangName,
  type LangOption,
} from '@/utils/editorLang'
import {
  Close,
  Document,
  Edit,
  Folder,
  FolderOpened,
  HardDrive,
  Home,
  Minimize,
  Refresh,
  Save,
} from '@/icons'

const props = withDefaults(
  defineProps<{
    /** 要打开的文件路径；为空表示只开窗口，从左侧树里挑 */
    filePath?: string
    /** 每次「使用编辑器打开」自增，用于在已挂载的窗口里换文件 / 从最小化还原 */
    token?: number
    /** 树根（家目录）；为空时向后端要一次 */
    homePath?: string
    /** 父级（文件管理页）是否处于激活态：切走路由时整个浮窗隐藏 */
    active?: boolean
  }>(),
  { filePath: '', token: 0, homePath: '', active: true },
)

const emit = defineEmits<{
  /** 用户点了关闭：由父级卸载组件（浮窗没有自己的销毁开关） */
  (e: 'close'): void
  (e: 'saved', path: string): void
}>()

const { t } = useI18n()

// ── 文档状态 ────────────────────────────────────────────────

const currentPath = ref('')
const content = ref('')
/** 上次读盘 / 保存时的内容，用来判断「是否脏」 */
const original = ref('')
const meta = ref<FileEntry | null>(null)
const loading = ref(false)
const saving = ref(false)
const line = ref(1)
const col = ref(1)

const fileName = computed(() => currentPath.value.split('/').filter(Boolean).pop() || '')
const dirty = computed(() => !!currentPath.value && content.value !== original.value)

// ── 语法语言：默认按扩展名自动判断，可在状态栏手动指定（按文件记住） ──

/** 路径 → 手选语言；没记录的走自动判断 */
const langOverrides = new Map<string, EditorLangName>()
const langPick = ref<EditorLangName | 'auto'>('auto')
/** 按扩展名推断出来的语言（下拉里「自动（X）」用得上） */
const autoLang = computed<EditorLangName>(() =>
  currentPath.value ? langFromPath(currentPath.value) : 'text',
)
/** 传给 CodeEditor：auto 时给 undefined，让组件自己按 path 判断 */
const editorLang = computed<EditorLangName | undefined>(() =>
  langPick.value === 'auto' ? undefined : langPick.value,
)
const langOptions = computed<LangOption[]>(() => [
  { value: 'auto', label: t('fileEditor.autoLang', { name: langLabel(autoLang.value) }) },
  ...LANG_OPTIONS,
])

function onLangChange(v: EditorLangName | 'auto') {
  if (v === 'auto') langOverrides.delete(currentPath.value)
  else langOverrides.set(currentPath.value, v)
  langPick.value = v
}

// ── 窗口状态：最小化 / 位置 / 尺寸 ────────────────────────────

const minimized = ref(false)
/**
 * 层级压在 Element Plus 的弹层（对话框 / 下拉 2000+、Message 3000+）之下：
 * 这样文件管理里弹出对话框一定盖在浮窗之上，浮窗内的下拉（el-select）也能正常浮出来。
 */
const zIndex = ref(1500)
const x = ref(0)
const y = ref(0)
const w = ref(980)
const h = ref(620)

const shown = computed(() => props.active && !minimized.value)
const dockShown = computed(() => props.active && minimized.value)
const winStyle = computed(() => ({
  left: `${x.value}px`,
  top: `${y.value}px`,
  width: `${w.value}px`,
  height: `${h.value}px`,
  zIndex: zIndex.value,
}))

function clamp(v: number, min: number, max: number) {
  return Math.min(Math.max(v, min), Math.max(min, max))
}

/** 视口变化时把窗口拉回可见区域 */
function clampToViewport() {
  const vw = window.innerWidth
  const vh = window.innerHeight
  w.value = clamp(w.value, 520, vw - 16)
  h.value = clamp(h.value, 320, vh - 16)
  x.value = clamp(x.value, 0, Math.max(0, vw - 120))
  y.value = clamp(y.value, 0, Math.max(0, vh - 40))
}

let dragOffsetX = 0
let dragOffsetY = 0
let dragging = false

function onDragMove(e: MouseEvent) {
  if (!dragging) return
  x.value = clamp(e.clientX - dragOffsetX, 0, Math.max(0, window.innerWidth - 120))
  y.value = clamp(e.clientY - dragOffsetY, 0, Math.max(0, window.innerHeight - 40))
}

function stopDrag() {
  dragging = false
  window.removeEventListener('mousemove', onDragMove)
  window.removeEventListener('mouseup', stopDrag)
}

function startDrag(e: MouseEvent) {
  // 按钮区不触发拖动（它自己 @mousedown.stop 了，这里再兜一层）
  if ((e.target as HTMLElement)?.closest('.few-header-right')) return
  dragging = true
  dragOffsetX = e.clientX - x.value
  dragOffsetY = e.clientY - y.value
  window.addEventListener('mousemove', onDragMove)
  window.addEventListener('mouseup', stopDrag)
}

let resizing = false
let resizeStartX = 0
let resizeStartY = 0
let resizeStartW = 0
let resizeStartH = 0

function onResizeMove(e: MouseEvent) {
  if (!resizing) return
  w.value = clamp(resizeStartW + (e.clientX - resizeStartX), 520, window.innerWidth - x.value)
  h.value = clamp(resizeStartH + (e.clientY - resizeStartY), 320, window.innerHeight - y.value)
}

function stopResize() {
  resizing = false
  window.removeEventListener('mousemove', onResizeMove)
  window.removeEventListener('mouseup', stopResize)
}

function startResize(e: MouseEvent) {
  resizing = true
  resizeStartX = e.clientX
  resizeStartY = e.clientY
  resizeStartW = w.value
  resizeStartH = h.value
  window.addEventListener('mousemove', onResizeMove)
  window.addEventListener('mouseup', stopResize)
}

function bringToFront() {
  // 上限留在 2000 以下，避免抬着抬着盖住对话框与下拉
  zIndex.value = Math.min(Math.max(zIndex.value, 1500) + 1, 1900)
}

function restore() {
  minimized.value = false
  bringToFront()
}

// ── 文件读写 ────────────────────────────────────────────────

async function loadMeta(path: string) {
  try {
    const res = await getFileInfo(path)
    meta.value = res.data ?? null
  } catch {
    meta.value = null
  }
}

/**
 * 打开文件。`force` 用于「外部指定打开」的场景（父级菜单项 / 首次挂载），
 * 此时不做脏检查——否则会为一个用户没参与过的动作弹确认框。
 */
async function openPath(path: string, force = false) {
  if (!path) return
  if (!force && dirty.value) {
    try {
      await ElMessageBox.confirm(
        t('fileEditor.dirtySwitch', { name: fileName.value }),
        t('fileEditor.dirtyTitle'),
        {
          confirmButtonText: t('fileEditor.discard'),
          cancelButtonText: t('common.cancel'),
          type: 'warning',
        },
      )
    } catch {
      return
    }
  }
  loading.value = true
  try {
    const res = await readFile(path)
    const text = res.data?.content ?? ''
    // 二进制文件（含 NUL）进编辑器没意义，直接拒绝
    if (text.includes('\u0000')) {
      ElMessage.warning(t('fileEditor.binary'))
      return
    }
    currentPath.value = path
    original.value = text
    content.value = text
    meta.value = null
    // 这个文件之前手选过语言就沿用，否则回到自动判断
    langPick.value = langOverrides.get(path) ?? 'auto'
    void loadMeta(path)
  } catch {
    // 由拦截器统一提示
  } finally {
    loading.value = false
  }
}

async function reloadFile() {
  if (!currentPath.value) return
  await openPath(currentPath.value, true)
}

async function saveFile(silent = false) {
  if (!currentPath.value) return false
  saving.value = true
  try {
    await writeFile(currentPath.value, content.value)
    original.value = content.value
    emit('saved', currentPath.value)
    if (!silent) ElMessage.success(t('common.saveSuccess'))
    void loadMeta(currentPath.value)
    return true
  } catch {
    return false
  } finally {
    saving.value = false
  }
}

/** 关闭：脏文件先给「保存并关闭 / 放弃修改 / 取消」三选一 */
async function requestClose() {
  if (dirty.value) {
    try {
      await ElMessageBox.confirm(
        t('fileEditor.dirtyClose', { name: fileName.value }),
        t('fileEditor.dirtyTitle'),
        {
          confirmButtonText: t('fileEditor.saveAndClose'),
          cancelButtonText: t('fileEditor.discard'),
          type: 'warning',
        },
      )
      const ok = await saveFile(true)
      if (!ok) return
    } catch (e) {
      // cancel = 放弃修改；close / 其它 = 什么都不做
      if (e !== 'cancel') return
    }
  }
  emit('close')
}

function onCursor(pos: { line: number; col: number }) {
  line.value = pos.line
  col.value = pos.col
}

function formatSize(bytes: number): string {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)
  const v = bytes / Math.pow(1024, i)
  return `${i === 0 ? v : v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`
}

// ── 左侧文件树（懒加载，只按需请求展开的目录） ────────────────

interface TreeNode {
  name: string
  path: string
  is_dir: boolean
  children?: TreeNode[]
  icon?: 'home' | 'root'
}

const treeData = ref<TreeNode[]>([])
const treeLoading = ref(false)
const treeProps = { label: 'name', children: 'children', isLeaf: (d: any) => !d.is_dir }

async function loadTreeRoot() {
  treeLoading.value = true
  try {
    let home = props.homePath
    if (!home) {
      const res = await listFiles('')
      home = res.data?.home || res.data?.current_path || '/'
    }
    treeData.value = [{ name: home, path: home, is_dir: true, icon: 'home' }]
  } catch {
    treeData.value = [{ name: '/', path: '/', is_dir: true, icon: 'root' }]
  } finally {
    treeLoading.value = false
  }
}

async function refreshTree() {
  await loadTreeRoot()
}

async function loadTreeNode(node: any, resolve: (data: TreeNode[]) => void) {
  const path: string | undefined = node.data?.path
  // level 0 由 :data 提供，再 resolve 会重复渲染一组根节点
  if (node.level === 0 || !path) {
    resolve([])
    return
  }
  try {
    const res = await listFiles(path)
    resolve(
      (res.data?.entries || []).map((e) => ({ name: e.name, path: e.path, is_dir: e.is_dir })),
    )
  } catch {
    resolve([])
  }
}

function onTreeNodeClick(data: TreeNode) {
  if (!data.is_dir) void openPath(data.path)
}

// ── 快捷键：窗口内 Ctrl / Cmd + S 保存 ───────────────────────

/**
 * 捕获阶段监听：命中时 stopPropagation，避免同一个按键被页面上其它
 * 编辑器（例如文件管理的编辑对话框）再消费一次。
 */
function onKeydown(e: KeyboardEvent) {
  if (minimized.value || !props.active || !currentPath.value) return
  if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 's') return
  e.preventDefault()
  e.stopPropagation()
  if (!saving.value) void saveFile()
}

onMounted(async () => {
  x.value = Math.max(0, Math.round((window.innerWidth - w.value) / 2))
  y.value = Math.min(Math.max(0, Math.round((window.innerHeight - h.value) / 2) - 40), 120)
  window.addEventListener('keydown', onKeydown, true)
  window.addEventListener('resize', clampToViewport)
  await loadTreeRoot()
  if (props.filePath) await openPath(props.filePath, true)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown, true)
  window.removeEventListener('resize', clampToViewport)
  stopDrag()
  stopResize()
})

// 父级再次「使用编辑器打开」（可能换了文件，也可能窗口正缩在图标里）
watch(
  () => [props.filePath, props.token],
  () => {
    // 缩成图标时无论是否带文件都先还原，让窗口重新可见
    if (minimized.value) restore()
    if (!props.filePath) return
    void openPath(props.filePath, true)
  },
)
</script>

<style scoped lang="scss">
.few-window {
  position: fixed;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--el-bg-color);
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
  box-shadow: var(--el-box-shadow-dark);

  &.is-loading {
    cursor: progress;
  }
}

// ── 顶部工具栏 ──────────────────────────────────────────────

.few-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 6px 8px 6px 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  background: var(--el-fill-color-light);
  cursor: move;
  user-select: none;

  &-left {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  &-icon {
    color: var(--el-color-primary);
  }

  &-right {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    cursor: default;
  }
}

.few-title {
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.few-dirty-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--el-color-warning);
  flex-shrink: 0;
}

// ── 中间：左树 + 右编辑器 ────────────────────────────────────

.few-body {
  flex: 1;
  display: flex;
  min-height: 0;
}

.few-sidebar {
  width: 240px;
  min-width: 160px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--el-border-color-lighter);
  background: var(--el-bg-color-page);

  &-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px 4px 12px;
    font-size: 12px;
    font-weight: 600;
    border-bottom: 1px solid var(--el-border-color-lighter);
  }
}

.few-tree-scroll {
  flex: 1;
  padding: 6px 4px;
}

.few-tree-node {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
}

.few-tree-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
}

.few-editor-area {
  flex: 1;
  min-width: 0;
  padding: 8px;
}

.few-editor {
  height: 100%;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  overflow: hidden;
}

.few-placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  font-size: 13px;
  color: var(--el-text-color-placeholder);
}

// ── 底部状态栏 ──────────────────────────────────────────────

.few-status {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 4px 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  border-top: 1px solid var(--el-border-color-lighter);
  background: var(--el-fill-color-light);

  &-item {
    flex-shrink: 0;
  }

  &-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  // 语言选择：状态栏里压扁一点，别把路径挤没了
  .few-lang {
    width: 130px;
    flex-shrink: 0;

    :deep(.el-select__wrapper) {
      min-height: 22px;
      font-size: 12px;
    }
  }

  .is-dirty {
    color: var(--el-color-warning);
  }

  .is-saved {
    color: var(--el-color-success);
  }
}

// ── 拉伸手柄 ────────────────────────────────────────────────

.few-resize {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 14px;
  height: 14px;
  cursor: nwse-resize;
  background: linear-gradient(
    135deg,
    transparent 0,
    transparent 50%,
    var(--el-border-color-light) 50%,
    var(--el-border-color-light) 100%
  );
}

// ── 最小化：底部小图标 ──────────────────────────────────────

.few-dock {
  position: fixed;
  left: 16px;
  bottom: 16px;
  z-index: 1500;
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 260px;
  padding: 6px 10px;
  font-size: 12px;
  color: var(--el-text-color-regular);
  background: var(--el-bg-color-overlay);
  border: 1px solid var(--el-border-color-light);
  border-radius: 16px;
  box-shadow: var(--el-box-shadow-light);
  cursor: pointer;

  &-icon {
    color: var(--el-color-primary);
  }

  &-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  &-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--el-color-warning);
    flex-shrink: 0;
  }

  &-close:hover {
    color: var(--el-color-danger);
  }
}
</style>
