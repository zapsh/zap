<template>
  <!--
    非模态浮窗：Teleport 到 body（文件管理自身 overflow:hidden，且浮窗要能盖住整个视口）。
    生命周期由父组件（文件管理）控制：父级 v-if 决定挂载/卸载，本组件只管显示、最小化与关闭意图。
    内部是多标签模型：一个文件一个标签，各自保留内容、光标、语言选择与 dirty 状态。
  -->
  <Teleport to="body">
    <!-- 最小化态：缩成底部小图标，点图标还原，点 × 关闭 -->
    <div v-if="dockShown" class="few-dock" :title="dockTitle" @click="restore">
      <el-icon class="few-dock-icon"><Edit /></el-icon>
      <span class="few-dock-name">{{ dockName }}</span>
      <span v-if="dirtyCount" class="few-dock-dot" :title="t('fileEditor.unsaved')" />
      <el-icon class="few-dock-close" @click.stop="requestClose"><Close /></el-icon>
    </div>

    <div
      v-else-if="shown"
      ref="winRoot"
      class="few-window"
      :class="{ 'is-expanded': expanded }"
      :style="winStyle"
      @mousedown="bringToFront"
    >
      <!-- 顶部工具栏：标题 + 保存/保存全部/重新加载/最小化/关闭，整条可拖动（按钮区除外） -->
      <div class="few-header" @mousedown="startDrag">
        <div class="few-header-left">
          <el-icon class="few-header-icon"><Edit /></el-icon>
          <span class="few-title">{{ activeName || t('fileEditor.title') }}</span>
          <span
            v-if="activeTab && activeDirty"
            class="few-dirty-dot"
            :title="t('fileEditor.unsaved')"
          />
          <span v-if="tabs.length > 1" class="few-tabs-count">
            {{ t('fileEditor.filesCount', { n: tabs.length }) }}
          </span>
        </div>
        <div class="few-header-right" @mousedown.stop>
          <el-button
            size="small"
            type="primary"
            :disabled="!activeTab || activeTab.saving"
            :loading="!!activeTab?.saving"
            @click="saveFile()"
          >
            <el-icon><Save /></el-icon>
            {{ t('common.save') }}
          </el-button>
          <el-button
            v-if="dirtyCount > 1"
            size="small"
            :title="t('fileEditor.saveAll')"
            @click="saveAllTabs"
          >
            <el-icon><Save /></el-icon>
            {{ t('fileEditor.saveAll') }}
          </el-button>
          <el-button size="small" :disabled="!activeTab || activeTab.loading" @click="reloadFile">
            <el-icon><Refresh /></el-icon>
            {{ t('fileEditor.reload') }}
          </el-button>
          <!-- 文件管理器插件槽位：与保存/重载同行，传入当前目录路径 -->
          <PluginSlot placement-slot="file.editor" mode="toolbar" :cwd="currentDir" />
          <el-button
            size="small"
            text
            :title="expanded ? t('fileEditor.exitFullscreen') : t('fileEditor.fullscreen')"
            @click="toggleExpand"
          >
            <el-icon><component :is="expanded ? FullScreenExit : FullScreen" /></el-icon>
          </el-button>
          <el-button size="small" text :title="t('fileEditor.minimize')" @click="minimizeWin">
            <el-icon><Minimize /></el-icon>
          </el-button>
          <el-button size="small" text :title="t('common.close')" @click="requestClose">
            <el-icon><Close /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 中间：左侧文件树 + 右侧（标签条 + 编辑器） -->
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
              ref="treeRef"
              :data="treeData"
              :props="treeProps"
              :load="loadTreeNode"
              node-key="path"
              lazy
              highlight-current
              :current-node-key="activePath || treeRootPath"
              @node-click="onTreeNodeClick"
              @node-contextmenu="onTreeContextMenu"
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
          <!-- 标签条：已打开的文件都在这里；点标签切换，× 关闭单个，右键菜单关闭其他/全部；溢出时下拉定位 -->
          <div v-if="tabs.length" class="few-tabbar">
            <div class="few-tabs" ref="tabsScrollRef" @contextmenu.prevent="onTabbarContext">
              <div
                v-for="tab in tabs"
                :key="tab.path"
                class="few-tab"
                :class="{ 'is-active': tab.path === activePath }"
                :title="tab.path"
                @click="activateTab(tab)"
                @mousedown.middle.prevent="closeTab(tab)"
                @contextmenu.prevent.stop="onTabContext(tab, $event)"
              >
                <span class="few-tab-name">{{ nameOf(tab.path) }}</span>
                <span v-if="tabDirty(tab)" class="few-tab-dot" :title="t('fileEditor.unsaved')" />
                <el-icon class="few-tab-close" @click.stop="closeTab(tab)"><Close /></el-icon>
              </div>
            </div>
            <el-dropdown class="few-tabs-more" trigger="click" @command="activatePath">
              <el-button size="small" text :title="t('fileEditor.allTabs')">
                <el-icon><MoreFilled /></el-icon>
              </el-button>
              <template #dropdown>
                <el-dropdown-menu class="few-tabs-menu">
                  <el-dropdown-item
                    v-for="tab in tabs"
                    :key="tab.path"
                    :command="tab.path"
                    :class="{ 'is-active': tab.path === activePath }"
                  >
                    <span class="few-tabs-menu-name" @click="activateTab(tab)">
                      {{ nameOf(tab.path) }}
                    </span>
                    <el-icon class="few-tabs-menu-close" @click.stop="closeTab(tab)">
                      <Close />
                    </el-icon>
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>

          <!-- 右键标签菜单 -->
          <div
            v-if="tabMenu.show"
            class="few-tab-ctx"
            :style="{ left: tabMenu.x + 'px', top: tabMenu.y + 'px' }"
            @click.stop
            @contextmenu.prevent
          >
            <div v-if="tabMenu.target" class="few-tab-ctx-item" @click="closeMenuTarget">
              {{ t('fileEditor.close') }}
            </div>
            <div
              v-if="tabMenu.target"
              class="few-tab-ctx-item"
              @click="closeMenuOthers"
            >
              {{ t('fileEditor.closeOthers') }}
            </div>
            <div class="few-tab-ctx-item" @click="closeAllTabs">{{ t('fileEditor.closeAll') }}</div>
          </div>

          <!-- 每个标签一个编辑器实例：v-show 切换，各自的 undo 历史与光标都留着 -->
          <div class="few-editors">
            <CodeEditor
              v-for="tab in tabs"
              v-show="tab.path === activePath"
              :key="tab.path"
              v-model="tab.content"
              class="few-editor"
              :path="tab.path"
              :lang="langOf(tab)"
              :active="tab.path === activePath"
              @cursor="(pos) => onCursor(tab, pos)"
            />
            <div v-if="!tabs.length" class="few-placeholder">{{ t('fileEditor.empty') }}</div>
          </div>
        </div>
      </div>

      <!-- 底部状态栏：光标位置 / 路径 / 语言 / 大小与权限 / 保存状态 -->
      <div class="few-status">
        <span class="few-status-item">
          {{ t('fileEditor.pos', { line: activeTab?.line ?? 1, col: activeTab?.col ?? 1 }) }}
        </span>
        <span class="few-status-path" :title="activePath">{{ activePath || '—' }}</span>
        <el-select
          v-if="activeTab"
          class="few-lang"
          size="small"
          :model-value="activeTab.langPick"
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
        <span v-if="activeTab?.meta" class="few-status-item">
          {{ formatSize(activeTab.meta.size) }} · {{ activeTab.meta.permissions }}
        </span>
        <span class="few-status-item" :class="activeDirty ? 'is-dirty' : 'is-saved'">
          {{ activeDirty ? t('fileEditor.unsaved') : t('fileEditor.savedState') }}
        </span>
      </div>

      <!-- 右下角拉伸手柄（全屏时隐藏） -->
      <div v-show="!expanded" class="few-resize" @mousedown.stop.prevent="startResize" />
    </div>

    <!-- 左侧树右键菜单：新建目录 / 新建文件 / 重命名 / 删除 -->
    <div v-if="treeMenuVisible" class="fm-context-backdrop" @click="closeTreeMenu" />
    <div
      v-if="treeMenuVisible"
      ref="treeMenuRef"
      class="fm-context-menu"
      :style="{ left: treeMenuX + 'px', top: treeMenuY + 'px' }"
      @click.stop
    >
      <div class="fm-context-item" @click="treeNewDir">
        <el-icon><FolderAdd /></el-icon>
        <span>{{ t('filesLocal.newDir') }}</span>
      </div>
      <div class="fm-context-item" @click="treeNewFile">
        <el-icon><DocumentAdd /></el-icon>
        <span>{{ t('filesLocal.newFile') }}</span>
      </div>
      <div class="fm-context-item" @click="treeRename">
        <el-icon><Edit /></el-icon>
        <span>{{ t('filesLocal.rename') }}</span>
      </div>
      <div class="fm-context-item danger" @click="treeDelete">
        <el-icon><Delete /></el-icon>
        <span>{{ t('common.delete') }}</span>
      </div>
    </div>

    <!-- 新建目录 -->
    <el-dialog v-model="mkdirVisible" :title="t('filesLocal.newDir')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.dirName')">
          <el-input
            v-model="mkdirName"
            :placeholder="t('filesLocal.dirNamePlaceholder')"
            @keydown.enter.prevent="doMkdir"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="mkdirVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doMkdir">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 新建文件 -->
    <el-dialog v-model="newFileVisible" :title="t('filesLocal.newFile')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.fileName')">
          <el-input
            v-model="newFileName"
            :placeholder="t('filesLocal.fileNamePlaceholder')"
            @keydown.enter.prevent="doNewFile"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="newFileVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doNewFile">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 重命名 -->
    <el-dialog v-model="renameVisible" :title="t('filesLocal.rename')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.newName')">
          <el-input
            v-model="renameName"
            :placeholder="t('filesLocal.newNamePlaceholder')"
            @keydown.enter.prevent="doRename"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="renameVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doRename">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import CodeEditor from '@/components/CodeEditor.vue'
import PluginSlot from '@/components/PluginSlot.vue'
import { getFileInfo, listFiles, readFile, writeFile, mkdir, deleteFile, renameFile, type FileEntry } from '@/api/file'
import {
  LANG_OPTIONS,
  langFromPath,
  langLabel,
  type EditorLangName,
  type LangOption,
} from '@/utils/editorLang'
import {
  Close,
  Delete,
  Document,
  DocumentAdd,
  Edit,
  Folder,
  FolderAdd,
  FolderOpened,
  FullScreen,
  FullScreenExit,
  HardDrive,
  Home,
  Minimize,
  MoreFilled,
  Refresh,
  Save,
} from '@/icons'

const props = withDefaults(
  defineProps<{
    /** 要打开的文件路径；为空表示只开窗口，从左侧树里挑 */
    filePath?: string
    /** 要打开的路径是否为目录：是则进入「文件夹视图」而非直接打开文件 */
    isDir?: boolean
    /** 每次「使用编辑器打开」自增，用于在已挂载的窗口里加标签 / 从最小化还原 */
    token?: number
    /** 树根（家目录）；为空时向后端要一次 */
    homePath?: string
    /** 父级（文件管理页）是否处于激活态：切走路由时整个浮窗隐藏 */
    active?: boolean
  }>(),
  { filePath: '', isDir: false, token: 0, homePath: '', active: true },
)

const emit = defineEmits<{
  /** 用户点了关闭：由父级卸载组件（浮窗没有自己的销毁开关） */
  (e: 'close'): void
  (e: 'saved', path: string): void
}>()

const { t } = useI18n()

// ── 标签页模型 ──────────────────────────────────────────────

interface EditorTab {
  path: string
  content: string
  /** 上次读盘 / 保存时的内容，用来判断「是否脏」 */
  original: string
  meta: FileEntry | null
  /** 正在重新读盘 */
  loading: boolean
  /** 正在保存 */
  saving: boolean
  line: number
  col: number
  /** 手选的语法语言；'auto' = 按扩展名判断 */
  langPick: EditorLangName | 'auto'
}

const tabs = ref<EditorTab[]>([])
const activePath = ref('')

const activeTab = computed(() => tabs.value.find((tab) => tab.path === activePath.value) ?? null)
const activeDirty = computed(() => (activeTab.value ? tabDirty(activeTab.value) : false))
const dirtyTabs = computed(() => tabs.value.filter(tabDirty))
const dirtyCount = computed(() => dirtyTabs.value.length)
const activeName = computed(() => (activeTab.value ? nameOf(activeTab.value.path) : ''))

function nameOf(path: string) {
  return path.split('/').filter(Boolean).pop() || ''
}

function tabDirty(tab: EditorTab) {
  return tab.content !== tab.original
}

function activateTab(tab: EditorTab) {
  activePath.value = tab.path
  scrollActiveIntoView()
}

/** 当前所在目录：左侧树基准目录优先，没有则取当前文件的父目录（供插件如 Git 作为工作目录） */
const currentDir = computed(() =>
  treeRootPath.value || (activePath.value ? parentOf(activePath.value) : ''),
)

/** 标签条滚动容器；切换标签后把当前标签滚进可视区 */
const tabsScrollRef = ref<HTMLElement | null>(null)
function scrollActiveIntoView() {
  nextTick(() => {
    const el = tabsScrollRef.value?.querySelector('.few-tab.is-active') as HTMLElement | null
    el?.scrollIntoView({ inline: 'nearest', block: 'nearest' })
  })
}

// ── 语法语言：默认按扩展名自动判断，可在状态栏手动指定（按文件记住） ──

/** 路径 → 手选语言；没记录的走自动判断 */
const langOverrides = new Map<string, EditorLangName>()

function langOf(tab: EditorTab): EditorLangName | undefined {
  return tab.langPick === 'auto' ? undefined : tab.langPick
}

const langOptions = computed<LangOption[]>(() => {
  const auto = activeTab.value ? langFromPath(activeTab.value.path) : 'text'
  return [
    { value: 'auto', label: t('fileEditor.autoLang', { name: langLabel(auto) }) },
    ...LANG_OPTIONS,
  ]
})

function onLangChange(v: EditorLangName | 'auto') {
  const tab = activeTab.value
  if (!tab) return
  if (v === 'auto') langOverrides.delete(tab.path)
  else langOverrides.set(tab.path, v)
  tab.langPick = v
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

/** 全屏 / 最大化：铺满整个视口（优先用浏览器 Fullscreen API，禁用时退化为填满窗口） */
const winRoot = ref<HTMLElement | null>(null)
const expanded = ref(false)
let savedGeo: { x: number; y: number; w: number; h: number } | null = null

const shown = computed(() => props.active && !minimized.value)
const dockShown = computed(() => props.active && minimized.value)

/** 退出浏览器 Fullscreen API（若无全屏态则空操作） */
function exitFullscreenApi() {
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {})
}

/** 切换全屏 / 最大化：记录原几何，退出时还原 */
async function toggleExpand() {
  if (expanded.value) {
    expanded.value = false
    exitFullscreenApi()
    if (savedGeo) {
      x.value = savedGeo.x
      y.value = savedGeo.y
      w.value = savedGeo.w
      h.value = savedGeo.h
      savedGeo = null
    }
    return
  }
  savedGeo = { x: x.value, y: y.value, w: w.value, h: h.value }
  expanded.value = true
  const el = winRoot.value
  if (el && document.fullscreenEnabled && el.requestFullscreen) {
    try {
      await el.requestFullscreen()
    } catch {
      // 被拒绝（如 iframe 限制、非用户手势）：保留「填满窗口」即可
    }
  }
}
const winStyle = computed(() => {
  // 全屏 / 最大化：直接铺满视口，忽略 x/y/w/h
  if (expanded.value) {
    return { left: '0px', top: '0px', width: '100vw', height: '100vh', zIndex: zIndex.value }
  }
  return {
    left: `${x.value}px`,
    top: `${y.value}px`,
    width: `${w.value}px`,
    height: `${h.value}px`,
    zIndex: zIndex.value,
  }
})

/** 最小化的图标上显示什么：优先当前文件名，多个标签补一句「共 N 个文件」 */
const dockName = computed(() => activeName.value || t('fileEditor.title'))
const dockTitle = computed(() =>
  tabs.value.length > 1
    ? `${activePath.value} · ${t('fileEditor.filesCount', { n: tabs.value.length })}`
    : activePath.value || t('fileEditor.title'),
)

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

// ── 左侧树以「打开的目录」为根 ───────────────────────────────

/** 左侧树当前根目录（打开目录 / 打开文件时取其父目录）；文件管理里的树根不设 */
const treeRootPath = ref('')
const treeRef = ref<any>(null)

/** 取路径的父目录（已到根则返回 '/'） */
function parentOf(path: string): string {
  const p = path.replace(/\/+$/, '')
  if (p === '' || p === '/') return '/'
  const idx = p.lastIndexOf('/')
  return idx <= 0 ? '/' : p.slice(0, idx)
}

/**
 * 把左侧树的根改成指定目录：树只展示该目录及其子目录，
 * 让「用编辑器打开」的目录成为浏览的基准（右侧不再重复列目录）。
 */
async function rootTreeAt(path: string) {
  treeRootPath.value = path
  treeData.value = [
    {
      name: nameOf(path) || path,
      path,
      is_dir: true,
      icon: path === props.homePath ? 'home' : path === '/' ? 'root' : undefined,
    },
  ]
  await nextTick()
  const node = treeRef.value?.store.getNode(path)
  if (node) await node.expand()
  treeRef.value?.setCurrentKey(path)
}

// ── 文件读写 ────────────────────────────────────────────────

async function loadMeta(path: string) {
  const tab = tabs.value.find((item) => item.path === path)
  if (!tab) return
  try {
    const res = await getFileInfo(path)
    tab.meta = res.data ?? null
  } catch {
    tab.meta = null
  }
}

/**
 * 打开一个文件：已开过的直接切到那个标签，否则读盘后新开一个标签。
 * 多标签下「打开文件」不再销毁任何东西，所以不需要脏检查。
 */
async function openPath(path: string) {
  if (!path) return
  const exist = tabs.value.find((tab) => tab.path === path)
  if (exist) {
    activePath.value = path
    return
  }
  let text: string
  try {
    const res = await readFile(path)
    text = res.data?.content ?? ''
  } catch {
    // 由拦截器统一提示
    return
  }
  // 二进制文件（含 NUL）进编辑器没意义，直接拒绝
  if (text.includes('\u0000')) {
    ElMessage.warning(t('fileEditor.binary'))
    return
  }
  tabs.value.push({
    path,
    content: text,
    original: text,
    meta: null,
    loading: false,
    saving: false,
    line: 1,
    col: 1,
    // 这个文件之前手选过语言就沿用，否则回到自动判断
    langPick: langOverrides.get(path) ?? 'auto',
  })
  activePath.value = path
  void loadMeta(path)
}

/** 重新读盘：脏文件先问「保存 / 放弃 / 取消」 */
async function reloadFile() {
  const tab = activeTab.value
  if (!tab) return
  if (tabDirty(tab)) {
    const choice = await confirmDirty(t('fileEditor.dirtySwitch', { name: nameOf(tab.path) }), {
      saveText: t('common.save'),
    })
    if (choice === 'cancel') return
    if (choice === 'save' && !(await saveTab(tab, true))) return
  }
  tab.loading = true
  try {
    const res = await readFile(tab.path)
    tab.content = res.data?.content ?? ''
    tab.original = tab.content
    void loadMeta(tab.path)
  } catch {
    // 由拦截器统一提示
  } finally {
    tab.loading = false
  }
}

async function saveTab(tab: EditorTab, silent = false): Promise<boolean> {
  tab.saving = true
  try {
    await writeFile(tab.path, tab.content)
    tab.original = tab.content
    emit('saved', tab.path)
    if (!silent) ElMessage.success(t('common.saveSuccess'))
    void loadMeta(tab.path)
    return true
  } catch {
    return false
  } finally {
    tab.saving = false
  }
}

async function saveFile() {
  const tab = activeTab.value
  if (!tab) return
  await saveTab(tab)
}

/** 工具栏上的「保存全部」：串行保存所有脏标签，返回是否全部成功 */
async function saveAllTabs(): Promise<boolean> {
  let ok = true
  for (const tab of [...dirtyTabs.value]) {
    if (!(await saveTab(tab, true))) ok = false
  }
  if (ok) ElMessage.success(t('common.saveSuccess'))
  return ok
}

/** 单个标签的关闭：脏文件给「保存并关闭 / 放弃修改 / 取消」三选一 */
async function closeTab(tab: EditorTab) {
  if (tabDirty(tab)) {
    const choice = await confirmDirty(t('fileEditor.dirtyClose', { name: nameOf(tab.path) }))
    if (choice === 'cancel') return
    if (choice === 'save' && !(await saveTab(tab, true))) return
  }
  const idx = tabs.value.indexOf(tab)
  tabs.value.splice(idx, 1)
  if (activePath.value !== tab.path) return
  // 关掉的是当前标签：优先切右边的，没有就切左边的
  const next = tabs.value[idx] ?? tabs.value[idx - 1]
  activePath.value = next ? next.path : ''
}

// ── 标签右键菜单：关闭 / 关闭其他 / 关闭全部 ────────────────────

const tabMenu = reactive({ show: false, x: 0, y: 0, target: null as EditorTab | null })

function hideTabMenu() {
  tabMenu.show = false
}

function onTabbarContext(e: MouseEvent) {
  tabMenu.target = null
  tabMenu.x = e.clientX
  tabMenu.y = e.clientY
  tabMenu.show = true
}

function onTabContext(tab: EditorTab, e: MouseEvent) {
  tabMenu.target = tab
  tabMenu.x = e.clientX
  tabMenu.y = e.clientY
  tabMenu.show = true
}

/** 右键菜单的「关闭」：关闭目标标签（菜单自身点完需手动收起） */
function closeMenuTarget() {
  if (tabMenu.target) void closeTab(tabMenu.target)
  hideTabMenu()
}

/** 右键菜单的「关闭其他」：目标可能为空时跳过 */
function closeMenuOthers() {
  if (tabMenu.target) void closeOtherTabs(tabMenu.target)
}

/** 下拉「所有标签」里点某条：切换过去 */
function activatePath(p: string) {
  const tab = tabs.value.find((t) => t.path === p)
  if (tab) activateTab(tab)
}

/** 关闭除 target 外的所有标签（脏文件先确认） */
async function closeOtherTabs(target: EditorTab) {
  hideTabMenu()
  const others = tabs.value.filter((t) => t.path !== target.path)
  const dirty = others.filter(tabDirty)
  if (dirty.length) {
    const choice = await confirmDirty(t('fileEditor.dirtyCloseOthers', { n: dirty.length }), {
      saveText: t('fileEditor.saveAll'),
      discardText: t('fileEditor.discardAll'),
    })
    if (choice === 'cancel') return
    if (choice === 'save') {
      for (const tab of dirty) {
        if (!(await saveTab(tab, true))) return
      }
    }
  }
  tabs.value = tabs.value.filter((t) => t.path === target.path)
  activePath.value = target.path
}

/** 关闭全部标签（脏文件先确认） */
async function closeAllTabs() {
  hideTabMenu()
  const dirty = tabs.value.filter(tabDirty)
  if (dirty.length) {
    const choice = await confirmDirty(t('fileEditor.dirtyCloseAll', { n: dirty.length }), {
      saveText: t('fileEditor.saveAllAndClose'),
      discardText: t('fileEditor.discardAll'),
    })
    if (choice === 'cancel') return
    if (choice === 'save') {
      for (const tab of dirty) {
        if (!(await saveTab(tab, true))) return
      }
    }
  }
  tabs.value = []
  activePath.value = ''
}

/** 最小化：退出浏览器全屏（若有），窗口仍保持「最大化」直到还原 */
function minimizeWin() {
  exitFullscreenApi()
  minimized.value = true
}

/** 关闭整个窗口：有脏标签时给「全部保存并关闭 / 全部放弃 / 取消」 */
async function requestClose() {
  if (dirtyCount.value) {
    const choice = await confirmDirty(t('fileEditor.dirtyCloseAll', { n: dirtyCount.value }), {
      saveText: t('fileEditor.saveAllAndClose'),
      discardText: t('fileEditor.discardAll'),
    })
    if (choice === 'cancel') return
    if (choice === 'save' && !(await saveAllTabs())) return
  }
  emit('close')
}

/**
 * 脏文件三选一。
 * confirm = 保存按钮文案，cancel = 放弃按钮文案，关闭弹窗（X / ESC）= 取消。
 */
async function confirmDirty(
  message: string,
  opts?: { saveText?: string; discardText?: string },
): Promise<'save' | 'discard' | 'cancel'> {
  try {
    await ElMessageBox.confirm(message, t('fileEditor.dirtyTitle'), {
      confirmButtonText: opts?.saveText ?? t('fileEditor.saveAndClose'),
      cancelButtonText: opts?.discardText ?? t('fileEditor.discard'),
      type: 'warning',
    })
    return 'save'
  } catch (e) {
    return e === 'cancel' ? 'discard' : 'cancel'
  }
}

function onCursor(tab: EditorTab, pos: { line: number; col: number }) {
  tab.line = pos.line
  tab.col = pos.col
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
  // 树根已经被钉在某个目录上（打开的文件 / 目录所在目录）时就地刷新，
  // 别把它拽回家目录——否则用户在当前目录点一下刷新就跳回去了
  const root = treeRootPath.value
  if (root) {
    await rootTreeAt(root)
    return
  }
  await loadTreeRoot()
  // 回到默认（家目录）根，清掉「以某目录为基准」的临时根
  treeRootPath.value = ''
}

async function loadTreeNode(node: any, resolve: (data: TreeNode[]) => void) {
  const path: string | undefined = node.data?.path
  // 没有路径的节点（占位根）不加载；其余目录（含被设为树根的那一层）都按需列出子目录
  if (!path) {
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
  // 目录：默认展开/收起；文件：开成编辑标签
  if (!data.is_dir) void openPath(data.path)
}

// ── 左侧树右键菜单（新建目录 / 新建文件 / 重命名 / 删除） ────

const treeMenuVisible = ref(false)
const treeMenuX = ref(0)
const treeMenuY = ref(0)
const treeMenuRef = ref<HTMLElement | null>(null)
const treeMenuNode = ref<TreeNode | null>(null)
/** 新建操作的目标父目录：目录节点 → 其下；文件节点 → 其父目录 */
const createTargetDir = ref('')

// 新建 / 重命名对话框
const mkdirVisible = ref(false)
const mkdirName = ref('')
const newFileVisible = ref(false)
const newFileName = ref('')
const renameVisible = ref(false)
const renameName = ref('')
const renameTargetPath = ref('')

/** 在指定目录下拼完整路径；dir 为空则回退到树根（家目录） */
function joinInDir(dir: string, name: string): string {
  const base = dir && dir.length ? dir : treeRootPath.value || '/'
  return !base || base === '/' ? `/${name}` : `${base}/${name}`
}

/** 右键节点：文件在其父目录创建，目录在其下创建；先按鼠标位置放，再夹回视口 */
async function onTreeContextMenu(event: Event, data: TreeNode) {
  event.preventDefault()
  const me = event as MouseEvent
  treeMenuNode.value = data
  createTargetDir.value = data.is_dir ? data.path : parentOf(data.path)
  treeMenuX.value = me.clientX
  treeMenuY.value = me.clientY
  treeMenuVisible.value = true

  await nextTick()
  const el = treeMenuRef.value
  if (!el) return
  const margin = 8
  const maxX = Math.max(margin, window.innerWidth - el.offsetWidth - margin)
  const maxY = Math.max(margin, window.innerHeight - el.offsetHeight - margin)
  treeMenuX.value = Math.min(Math.max(me.clientX, margin), maxX)
  treeMenuY.value = Math.min(Math.max(me.clientY, margin), maxY)
}

function closeTreeMenu() {
  treeMenuVisible.value = false
}

function treeNewDir() {
  treeMenuVisible.value = false
  mkdirName.value = ''
  mkdirVisible.value = true
}

function treeNewFile() {
  treeMenuVisible.value = false
  newFileName.value = ''
  newFileVisible.value = true
}

function treeRename() {
  treeMenuVisible.value = false
  const node = treeMenuNode.value
  if (!node) return
  renameTargetPath.value = node.path
  renameName.value = node.name
  renameVisible.value = true
}

async function treeDelete() {
  treeMenuVisible.value = false
  const node = treeMenuNode.value
  if (!node) return
  try {
    await ElMessageBox.confirm(
      t('filesLocal.delConfirmOne', { name: node.name }),
      t('filesLocal.delWarning'),
      { type: 'warning', confirmButtonText: t('filesLocal.delConfirmBtn') },
    )
  } catch {
    return
  }
  try {
    await deleteFile(node.path)
    ElMessage.success(t('filesLocal.deleteOk'))
    closeTabsUnder(node.path)
    await refreshTreeNode(parentOf(node.path))
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.deleteFailed'))
  }
}

async function doMkdir() {
  if (!mkdirName.value.trim()) {
    ElMessage.warning(t('filesLocal.needDirName'))
    return
  }
  const fullPath = joinInDir(createTargetDir.value, mkdirName.value.trim())
  try {
    await mkdir(fullPath)
    ElMessage.success(t('filesLocal.dirCreated'))
    mkdirVisible.value = false
    await refreshTreeNode(createTargetDir.value)
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.errMkdir', { name: mkdirName.value }))
  }
}

async function doNewFile() {
  if (!newFileName.value.trim()) {
    ElMessage.warning(t('filesLocal.needFileName'))
    return
  }
  const fullPath = joinInDir(createTargetDir.value, newFileName.value.trim())
  try {
    await writeFile(fullPath, '')
    ElMessage.success(t('filesLocal.fileCreated'))
    newFileVisible.value = false
    await refreshTreeNode(createTargetDir.value)
    void openPath(fullPath)
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.errNewFile', { name: newFileName.value }))
  }
}

async function doRename() {
  if (!renameTargetPath.value || !renameName.value.trim()) {
    ElMessage.warning(t('filesLocal.needNewName'))
    return
  }
  const newPath = joinInDir(parentOf(renameTargetPath.value), renameName.value.trim())
  try {
    await renameFile(renameTargetPath.value, newPath)
    ElMessage.success(t('filesLocal.renamed'))
    renameVisible.value = false
    renameOpenTabs(renameTargetPath.value, newPath)
    await refreshTreeNode(parentOf(renameTargetPath.value))
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.errRename', { name: renameName.value }))
  }
}

/** 刷新某个树节点（强制重新懒加载其下的子项），让新建/改名/删除立即可见 */
async function refreshTreeNode(path: string) {
  const tree = treeRef.value
  if (!tree) {
    await refreshTree()
    return
  }
  const node = tree.store.getNode(path)
  if (!node) {
    await refreshTree()
    return
  }
  node.loaded = false
  await node.expand()
  tree.setCurrentKey(path)
}

/** 删除节点后，关掉指向该路径（或其子路径）的已打开标签 */
function closeTabsUnder(p: string) {
  const norm = p.replace(/\/+$/, '')
  const next = tabs.value.filter(
    (tab) => tab.path !== norm && !tab.path.startsWith(norm + '/'),
  )
  tabs.value = next
  if (!next.find((t) => t.path === activePath.value)) {
    const last = next[next.length - 1]
    activePath.value = last ? last.path : ''
  }
}

/** 重命名节点后，同步更新指向该路径的已打开标签路径 */
function renameOpenTabs(oldPath: string, newPath: string) {
  const norm = oldPath.replace(/\/+$/, '')
  const remap = (path: string) =>
    path === norm
      ? newPath
      : path.startsWith(norm + '/')
        ? newPath + path.slice(norm.length)
        : path
  for (const tab of tabs.value) tab.path = remap(tab.path)
  activePath.value = remap(activePath.value)
}

// ── 快捷键：窗口内 Ctrl / Cmd + S 保存当前标签 ───────────────

/**
 * 捕获阶段监听：命中时 stopPropagation，避免同一个按键被页面上其它
 * 编辑器（例如文件管理的编辑对话框）再消费一次。
 */
function onKeydown(e: KeyboardEvent) {
  if (minimized.value || !props.active || !activeTab.value) return
  const mod = e.ctrlKey || e.metaKey
  // 切换标签：Ctrl/Cmd + Tab（Shift 反向）
  if (mod && e.key === 'Tab') {
    e.preventDefault()
    e.stopPropagation()
    const order = tabs.value
    if (order.length > 1) {
      const i = order.findIndex((t) => t.path === activePath.value)
      const dir = e.shiftKey ? -1 : 1
      activateTab(order[(i + dir + order.length) % order.length])
    }
    return
  }
  // 关闭当前标签：Ctrl/Cmd + W
  if (mod && e.key.toLowerCase() === 'w') {
    e.preventDefault()
    e.stopPropagation()
    void closeTab(activeTab.value)
    return
  }
  // 保存当前标签：Ctrl/Cmd + S
  if (mod && e.key.toLowerCase() === 's') {
    e.preventDefault()
    e.stopPropagation()
    if (!activeTab.value.saving) void saveFile()
  }
}

onMounted(async () => {
  x.value = Math.max(0, Math.round((window.innerWidth - w.value) / 2))
  y.value = Math.min(Math.max(0, Math.round((window.innerHeight - h.value) / 2) - 40), 120)
  window.addEventListener('keydown', onKeydown, true)
  window.addEventListener('resize', clampToViewport)
  window.addEventListener('click', hideTabMenu)
  await loadTreeRoot()
  if (props.filePath) {
    if (props.isDir) await rootTreeAt(props.filePath)
    else {
      // 文件：左侧树以「文件所在目录」为基准，并打开该文件
      await rootTreeAt(parentOf(props.filePath))
      await openPath(props.filePath)
    }
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown, true)
  window.removeEventListener('resize', clampToViewport)
  window.removeEventListener('click', hideTabMenu)
  exitFullscreenApi()
  stopDrag()
  stopResize()
})

// 父级再次「使用编辑器打开」（可能换了文件 / 目录，也可能窗口正缩在图标里）
watch(
  () => [props.filePath, props.token, props.isDir],
  () => {
    // 缩成图标时无论是否带文件都先还原，让窗口重新可见
    if (minimized.value) restore()
    // 没选具体文件 / 目录：左侧树回到家目录根（默认基准）
    if (!props.filePath) {
      void loadTreeRoot().then(() => {
        treeRootPath.value = ''
      })
      return
    }
    if (props.isDir) void rootTreeAt(props.filePath)
    else {
      void rootTreeAt(parentOf(props.filePath))
      void openPath(props.filePath)
    }
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

  // 全屏 / 最大化：去掉圆角与边框，真正铺满视口
  &.is-expanded {
    border-radius: 0;
    border-width: 0;
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

.few-tabs-count {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 400;
  color: var(--el-text-color-placeholder);
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
  display: flex;
  flex-direction: column;
  padding: 6px 8px 8px;
  gap: 6px;
}

// ── 标签条 ─────────────────────────────────────────────────

.few-tabbar {
  display: flex;
  align-items: stretch;
  gap: 4px;
  flex-shrink: 0;
}

.few-tabs {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  overflow-x: auto;
  padding-bottom: 4px;

  &::-webkit-scrollbar {
    height: 4px;
  }

  &::-webkit-scrollbar-thumb {
    background: var(--el-border-color-light);
    border-radius: 2px;
  }
}

.few-tabs-more {
  flex-shrink: 0;
  align-self: center;
}

.few-tabs-menu {
  :deep(.el-dropdown-menu__item) {
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: 320px;

    &.is-active {
      color: var(--el-color-primary);
      font-weight: 600;
    }
  }

  &-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  &-close {
    margin-left: 8px;
    flex-shrink: 0;

    &:hover {
      color: var(--el-color-danger);
    }
  }
}

// 标签右键菜单
.few-tab-ctx {
  position: fixed;
  z-index: 2500;
  min-width: 140px;
  padding: 4px;
  background: var(--el-bg-color-overlay);
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
  box-shadow: var(--el-box-shadow-light);

  &-item {
    padding: 6px 10px;
    font-size: 12px;
    color: var(--el-text-color-regular);
    border-radius: 4px;
    cursor: pointer;

    &:hover {
      background: var(--el-fill-color-light);
      color: var(--el-color-primary);
    }
  }
}

// 左侧树右键菜单（与文件管理同款，复用时同名类）
.fm-context-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1998;
}

.fm-context-menu {
  position: fixed;
  z-index: 1999;
  min-width: 160px;
  max-height: calc(100vh - 16px);
  overflow-y: auto;
  background: var(--el-bg-color-overlay);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  box-shadow: var(--el-box-shadow-light);
  padding: 6px 0;
}

.fm-context-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  font-size: 13px;
  color: var(--el-text-color-regular);
  cursor: pointer;

  &:hover {
    background: var(--el-fill-color-light);
    color: var(--el-color-primary);
  }

  &.danger:not(.disabled) {
    color: var(--el-color-danger);

    &:hover {
      background: var(--el-color-danger-light-9);
    }
  }
}

.fm-context-divider {
  height: 1px;
  background: var(--el-border-color-lighter);
  margin: 6px 0;
}

.few-tab {
  display: flex;
  align-items: center;
  gap: 4px;
  max-width: 180px;
  flex-shrink: 0;
  padding: 2px 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  background: var(--el-fill-color-light);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  cursor: pointer;

  &:hover {
    color: var(--el-text-color-primary);
  }

  &.is-active {
    color: var(--el-color-primary);
    font-weight: 600;
    background: var(--el-bg-color);
    border-color: var(--el-color-primary-light-5);
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

  &-close {
    font-size: 12px;
    border-radius: 50%;
    flex-shrink: 0;

    &:hover {
      color: var(--el-color-danger);
    }
  }
}

// ── 编辑器 ─────────────────────────────────────────────────

.few-editors {
  flex: 1;
  min-height: 0;
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
