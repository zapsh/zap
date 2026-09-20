<template>
  <div class="scripts-page">
    <!-- 左侧脚本树 -->
    <div class="scripts-sidebar">
      <div class="sidebar-header">
        <span class="sidebar-title">{{ t('automationScripts.title') }}</span>
        <el-button type="primary" size="small" :icon="Plus" @click="handleNewScript">
          {{ t('automationScripts.newScript') }}
        </el-button>
      </div>
      <el-scrollbar class="sidebar-tree">
        <el-tree
          :data="treeData"
          node-key="path"
          :props="treeProps"
          :expand-on-click-node="false"
          default-expand-all
          highlight-current
          @node-click="handleNodeClick"
          @node-contextmenu="handleContextMenu"
        >
          <template #default="{ data }">
            <span class="tree-node">
              <el-icon v-if="data.type === 'dir'" color="#e6a23c"><Folder /></el-icon>
              <el-icon v-else color="#409eff"><Document /></el-icon>
              <span class="tree-label">{{ data.name }}</span>
            </span>
          </template>
        </el-tree>
      </el-scrollbar>
      <div class="sidebar-tip">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ t('automationScripts.sidebarTip') }}</span>
      </div>
    </div>

    <!-- 右侧编辑器 -->
    <div class="editor-main">
      <div class="editor-toolbar">
        <span class="editor-path">{{ currentPath || t('automationScripts.selectOrNew') }}</span>
        <div class="toolbar-actions">
          <el-button
            size="small"
            type="primary"
            :disabled="!dirty || !currentPath"
            @click="handleSave"
          >
            {{ t('automationScripts.save') }}
          </el-button>
          <el-button
            size="small"
            type="success"
            :disabled="!currentPath || running"
            @click="handleRun"
          >
            {{ t('automationScripts.run') }}
          </el-button>
        </div>
      </div>
      <CodeEditor
        v-model="content"
        class="editor-area"
        :lang="editorLang"
        :readonly="!currentPath"
        :placeholder="
          currentPath
            ? t('automationScripts.placeholderEdit')
            : t('automationScripts.placeholderSelect')
        "
      />
    </div>

    <!-- 树右键菜单 -->
    <div
      v-show="ctxMenu.visible"
      class="ctx-menu"
      :style="{ top: `${ctxMenu.y}px`, left: `${ctxMenu.x}px` }"
      @contextmenu.prevent
    >
      <div class="ctx-item ctx-item-danger" @click.stop="handleCtxDelete">
        <el-icon><Delete /></el-icon>
        <span>{{ t('automationScripts.delete') }}</span>
      </div>
    </div>

    <!-- 日志抽屉 -->
    <AppStoreLogDrawer ref="logDrawerRef" />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Folder, Document, InfoFilled, Delete } from '@/icons'
import { getScriptsTree, readScript, writeScript, runScript, deleteScript } from '@/api/appstore'
import AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'
import CodeEditor from '@/components/CodeEditor.vue'
import { langFromPath } from '@/utils/editorLang'

const { t } = useI18n()

interface TreeNode {
  type: 'dir' | 'file'
  name: string
  path: string
  children?: TreeNode[]
}

const treeData = ref<TreeNode[]>([])
const treeProps = { label: 'name', children: 'children' }
const currentPath = ref('')
const content = ref('')
const originalContent = ref('')
const running = ref(false)
const dirty = computed(() => content.value !== originalContent.value)
// 脚本页默认 shell 高亮;若检测到其它语言(如 .py/.js)则按其语法高亮
const editorLang = computed(() => {
  if (!currentPath.value) return 'text'
  const l = langFromPath(currentPath.value)
  return l === 'text' ? 'shell' : l
})

async function loadTree() {
  try {
    const resp = await getScriptsTree()
    const tree = resp.data?.tree
    treeData.value = tree && tree.children ? tree.children : []
  } catch {
    // ignore
  }
}

function handleNodeClick(data: TreeNode) {
  if (data.type !== 'file') return
  openScript(data.path)
}

async function openScript(path: string) {
  if (dirty.value && currentPath.value !== path) {
    try {
      await ElMessageBox.confirm(
        t('automationScripts.unsavedSwitch'),
        t('automationScripts.notice'),
        {
          type: 'warning',
        },
      )
    } catch {
      return
    }
  }
  try {
    const resp = await readScript(path)
    currentPath.value = path
    content.value = resp.data?.content || ''
    originalContent.value = content.value
  } catch (e: any) {
    ElMessage.error(e.message || t('automationScripts.readFailed'))
  }
}

async function handleNewScript() {
  const defaultPath = 'scripts/new-script.sh'
  try {
    const { value } = await ElMessageBox.prompt(
      t('automationScripts.pathPrompt'),
      t('automationScripts.newTitle'),
      {
        inputValue: defaultPath,
        inputPlaceholder: t('automationScripts.pathPlaceholder'),
        inputValidator: (v: string) => {
          if (!v.trim()) return t('automationScripts.pathEmpty')
          if (!v.endsWith('.sh')) return t('automationScripts.mustBeSh')
          if (v.includes('..') || v.startsWith('/')) return t('automationScripts.pathInvalid')
          return true
        },
      },
    )
    const path = value.trim()
    const shebang = t('automationScripts.newScriptTemplate')
    const resp = await writeScript({ path, content: shebang })
    ElMessage.success(t('automationScripts.created'))
    currentPath.value = path
    content.value = shebang
    originalContent.value = shebang
    await loadTree()
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error(e.message || t('automationScripts.createFailed'))
  }
}

async function handleSave() {
  if (!currentPath.value) return
  try {
    await writeScript({ path: currentPath.value, content: content.value })
    originalContent.value = content.value
    ElMessage.success(t('automationScripts.saved'))
  } catch (e: any) {
    ElMessage.error(e.message || t('automationScripts.saveFailed'))
  }
}

async function handleRun() {
  if (!currentPath.value) return
  // 运行前自动保存
  if (dirty.value) {
    try {
      await writeScript({ path: currentPath.value, content: content.value })
      originalContent.value = content.value
    } catch (e: any) {
      ElMessage.error(e.message || t('automationScripts.saveFailedRun'))
      return
    }
  }
  running.value = true
  try {
    const resp = await runScript({ path: currentPath.value })
    ElMessage.success(t('automationScripts.started'))
    const name = currentPath.value.split('/').pop() || t('automationScripts.fallbackName')
    logDrawerRef.value?.openDrawer(resp.data.run_id, t('automationScripts.runLogTitle', { name }))
  } catch (e: any) {
    ElMessage.error(e.message || t('automationScripts.runFailed'))
  } finally {
    running.value = false
  }
}

// ── 树右键菜单 ────────────────────────────────────────────
const ctxMenu = ref<{ visible: boolean; x: number; y: number; node: TreeNode | null }>({
  visible: false,
  x: 0,
  y: 0,
  node: null,
})

function handleContextMenu(e: MouseEvent, data: TreeNode) {
  e.preventDefault()
  ctxMenu.value = { visible: true, x: e.clientX, y: e.clientY, node: data }
}

function closeCtxMenu() {
  ctxMenu.value.visible = false
}

async function handleCtxDelete() {
  const node = ctxMenu.value.node
  closeCtxMenu()
  if (!node) return
  const isDir = node.type === 'dir'
  try {
    await ElMessageBox.confirm(
      t(isDir ? 'automationScripts.deleteDirConfirm' : 'automationScripts.deleteFileConfirm', {
        name: node.name,
      }),
      t('automationScripts.deleteTitle'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteScript({ path: node.path })
    ElMessage.success(t('automationScripts.deleted'))
    // 删掉的正好是当前打开的脚本（或其所在目录）时，清空编辑器避免误存回已删文件
    if (currentPath.value === node.path || currentPath.value.startsWith(`${node.path}/`)) {
      currentPath.value = ''
      content.value = ''
      originalContent.value = ''
    }
    await loadTree()
  } catch (e: any) {
    ElMessage.error(e.message || t('automationScripts.deleteFailed'))
  }
}

const logDrawerRef = ref<InstanceType<typeof AppStoreLogDrawer> | null>(null)

// Ctrl/Cmd + S 保存当前脚本
function onKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
    if (!currentPath.value || !dirty.value) return
    e.preventDefault()
    void handleSave()
  }
}

onMounted(() => {
  loadTree()
  window.addEventListener('keydown', onKeydown)
  // 点击别处或窗口尺寸变化时收起右键菜单
  window.addEventListener('click', closeCtxMenu)
  window.addEventListener('resize', closeCtxMenu)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('click', closeCtxMenu)
  window.removeEventListener('resize', closeCtxMenu)
})
</script>

<style scoped>
.scripts-page {
  display: flex;
  /* 视口高 - 顶部导航 50 - 标签栏 34 - 底部 Footer 50 - app-main 上下 padding 20
     - 所属页「自动化脚本」顶部 nav pill 一行（38 + 间距 12） */
  height: calc(100vh - 204px);
  background: var(--el-bg-color);
  border-radius: 4px;
  overflow: hidden;
}

.scripts-sidebar {
  width: 280px;
  min-width: 280px;
  border-right: 1px solid var(--el-border-color-lighter);
  display: flex;
  flex-direction: column;
  background: var(--el-bg-color-page);
}

.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.sidebar-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}

.sidebar-tree {
  flex: 1;
  padding: 8px;
}

.tree-node {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
}

.tree-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.sidebar-tip {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 10px 12px;
  border-top: 1px solid var(--el-border-color-lighter);
  font-size: 11px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
  background: var(--el-fill-color-light);
}

.sidebar-tip .el-icon {
  margin-top: 2px;
  flex-shrink: 0;
}

.editor-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.editor-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.editor-path {
  font-size: 13px;
  color: var(--el-color-primary);
  font-family: monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.toolbar-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.editor-area {
  flex: 1;
  min-height: 0;
  border-top: 1px solid var(--el-border-color-lighter);
  background: var(--el-bg-color);
}

.editor-area.is-readonly {
  background: var(--el-fill-color-light);
}

/* 树右键菜单 */
.ctx-menu {
  position: fixed;
  z-index: 3000;
  min-width: 130px;
  padding: 4px 0;
  background: var(--el-bg-color-overlay);
  border: 1px solid var(--el-border-color-light);
  border-radius: 4px;
  box-shadow: var(--el-box-shadow-light);
}

.ctx-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  font-size: 13px;
  color: var(--el-text-color-regular);
  cursor: pointer;
}

.ctx-item:hover {
  background: var(--el-fill-color-light);
}

.ctx-item-danger {
  color: var(--el-color-danger);
}

.ctx-item-danger:hover {
  background: var(--el-color-danger-light-9);
}
</style>
