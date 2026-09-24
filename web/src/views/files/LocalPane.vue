<template>
  <div class="file-manager">
    <!-- 左侧目录树 -->
    <div class="fm-sidebar">
      <div class="fm-sidebar-header">
        <span>{{ t('filesLocal.tree') }}</span>
        <el-button :icon="Refresh" size="small" text @click="refreshTree" />
      </div>
      <el-scrollbar class="fm-tree-scroll">
        <el-tree
          ref="treeRef"
          :data="treeData"
          :props="treeProps"
          node-key="path"
          :load="loadTreeNode"
          lazy
          highlight-current
          :expand-on-click-node="true"
          @node-click="onTreeNodeClick"
        >
          <template #default="{ node, data }">
            <span class="fm-tree-node">
              <el-icon :size="16">
                <Home v-if="data.icon === 'home'" />
                <HardDrive v-else-if="data.icon === 'root'" />
                <FolderOpened v-else-if="node.expanded" />
                <Folder v-else />
              </el-icon>
              <span class="fm-tree-label" :class="{ mono: !!data.icon }" :title="data.path">
                {{ node.label }}
              </span>
            </span>
          </template>
        </el-tree>
      </el-scrollbar>
    </div>

    <!-- 右侧文件列表 -->
    <div class="fm-main">
      <!-- 工具栏 -->
      <div class="fm-toolbar">
        <div class="fm-toolbar-left">
          <el-breadcrumb separator=">" class="fm-crumbs">
            <el-breadcrumb-item v-for="(crumb, idx) in crumbs" :key="crumb.path">
              <a
                href="javascript:void(0)"
                :title="crumb.title"
                @click="navigateToCrumb(idx)"
                :class="{ 'is-last': idx === crumbs.length - 1 }"
              >
                <el-icon v-if="crumb.icon" :size="14" class="fm-crumb-icon">
                  <Home v-if="crumb.icon === 'home'" />
                  <HardDrive v-else />
                </el-icon>
                <span v-else>{{ crumb.label }}</span>
              </a>
            </el-breadcrumb-item>
          </el-breadcrumb>
        </div>
        <div class="fm-toolbar-right">
          <el-button-group class="view-toggle">
            <el-button
              :type="viewMode === 'list' ? 'primary' : ''"
              size="small"
              @click="viewMode = 'list'"
            >
              <el-icon><List /></el-icon>
            </el-button>
            <el-button
              :type="viewMode === 'grid' ? 'primary' : ''"
              size="small"
              @click="viewMode = 'grid'"
            >
              <el-icon><Grid /></el-icon>
            </el-button>
          </el-button-group>
          <el-upload :show-file-list="false" :http-request="handleUpload" multiple>
            <el-button size="small" :loading="uploadBusy">
              <el-icon><Upload /></el-icon>
              {{ t('filesLocal.uploadFile') }}
            </el-button>
          </el-upload>
          <el-upload :show-file-list="false" :http-request="handleUpload" multiple directory>
            <el-button size="small" :loading="uploadBusy">
              <el-icon><FolderOpened /></el-icon>
              {{ t('filesLocal.uploadDir') }}
            </el-button>
          </el-upload>
          <el-button size="small" @click="showMkdirDialog">
            <el-icon><FolderAdd /></el-icon>
            {{ t('filesLocal.newDir') }}
          </el-button>
          <el-button size="small" @click="showNewFileDialog">
            <el-icon><DocumentAdd /></el-icon>
            {{ t('filesLocal.newFile') }}
          </el-button>
          <el-button size="small" @click="refreshList" :loading="loading">
            <el-icon><Refresh /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 选中项操作条：常显，避免选中/取消时布局抖动；未选中时按钮禁用 -->
      <div class="fm-selection-bar">
        <span class="fm-selection-label">
          {{ t('filesLocal.selectedCount', { n: selectionCount, total: fileList.length }) }}
        </span>
        <el-button-group class="fm-selection-actions">
          <el-button size="small" :disabled="!canOpen" @click="openSelected">
            <el-icon><Open /></el-icon>
            {{ t('filesLocal.open') }}
          </el-button>
          <el-button size="small" :disabled="!canCopy" @click="showCopyDialog(false)">
            <el-icon><Copy /></el-icon>
            {{ t('filesLocal.copy') }}
          </el-button>
          <el-button size="small" :disabled="!canDuplicate" @click="showDuplicateDialog">
            <el-icon><Copy /></el-icon>
            {{ t('filesLocal.duplicate') }}
          </el-button>
          <el-button size="small" :disabled="!canMove" @click="showMoveDialog(false)">
            <el-icon><Move /></el-icon>
            {{ t('filesLocal.move') }}
          </el-button>
          <el-button size="small" :disabled="!canDownload" @click="downloadSelected">
            <el-icon><Download /></el-icon>
            {{ t('filesLocal.download') }}
          </el-button>
          <el-button size="small" :disabled="!canArchive" @click="showArchiveDialog">
            <el-icon><Archive /></el-icon>
            {{ t('filesLocal.archive') }}
          </el-button>
          <el-button size="small" :disabled="!canRename" @click="showRenameDialog(singleSelected!)">
            <el-icon><Edit /></el-icon>
            {{ t('filesLocal.rename') }}
          </el-button>
          <el-button
            size="small"
            :disabled="!canSetPermissions"
            @click="showPermDialogForSelection"
          >
            <el-icon><Setting /></el-icon>
            {{ t('filesLocal.perm') }}
          </el-button>
          <el-button
            v-if="isAdmin"
            size="small"
            :disabled="!hasSelection"
            @click="showOwnerDialogForSelection"
          >
            <el-icon><User /></el-icon>
            {{ t('filesLocal.owner') }}
          </el-button>
          <el-button size="small" type="danger" :disabled="!canRemove" @click="removeSelected">
            <el-icon><Delete /></el-icon>
            {{ t('common.delete') }}
          </el-button>
        </el-button-group>
        <el-button size="small" text :disabled="!hasSelection" @click="clearSelection">
          {{ t('filesLocal.clearSelection') }}
        </el-button>
      </div>

      <!-- 文件列表区：拖拽文件/文件夹到此处即上传到当前目录（文件夹保留层级） -->
      <div
        class="fm-list-area"
        @dragenter.prevent="onDragEnter"
        @dragover.prevent="onDragOver"
        @dragleave="onDragLeave"
        @drop.prevent="handleDrop"
      >
        <!-- 文件列表 - 列表视图 -->
        <div v-if="viewMode === 'list'" class="fm-table-wrap">
          <el-table
            ref="tableRef"
            :data="fileList"
            v-loading="loading"
            stripe
            :row-class-name="rowClassName"
            @row-click="onRowClick"
            @row-dblclick="onRowDblClick"
            @selection-change="onTableSelectionChange"
            @row-contextmenu="onTableRowContextMenu"
            style="width: 100%"
          >
            <el-table-column type="selection" width="40" />
            <el-table-column :label="t('common.name')" min-width="260">
              <template #default="{ row }">
                <div class="fm-file-name">
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
                <span v-else class="text-muted">-</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('filesLocal.colModified')" width="180">
              <template #default="{ row }">
                {{ row.modified }}
              </template>
            </el-table-column>
            <el-table-column :label="t('filesLocal.perm')" width="110">
              <template #default="{ row }">
                <el-button
                  link
                  type="primary"
                  class="mono fm-perm-btn"
                  :title="t('filesLocal.tipPerm')"
                  @click.stop="showPermDialog(row)"
                >
                  {{ row.permissions }}
                </el-button>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.user')" width="110">
              <template #default="{ row }">
                <!-- admin 可点击直接修改属主/属组，其余角色只读展示 -->
                <el-button
                  v-if="isAdmin"
                  link
                  type="primary"
                  class="mono"
                  :title="t('filesLocal.tipOwner')"
                  @click.stop="showOwnerDialog(row)"
                >
                  {{ row.owner || '-' }}
                </el-button>
                <span v-else-if="row.owner" class="mono">{{ row.owner }}</span>
                <span v-else class="text-muted">-</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.group')" width="110">
              <template #default="{ row }">
                <el-button
                  v-if="isAdmin"
                  link
                  type="primary"
                  class="mono"
                  :title="t('filesLocal.tipOwner')"
                  @click.stop="showOwnerDialog(row)"
                >
                  {{ row.group || '-' }}
                </el-button>
                <span v-else-if="row.group" class="mono">{{ row.group }}</span>
                <span v-else class="text-muted">-</span>
              </template>
            </el-table-column>
          </el-table>
        </div>

        <!-- 文件列表 - 网格视图 -->
        <div v-else class="fm-grid-wrap">
          <el-scrollbar>
            <div class="fm-grid" v-loading="loading">
              <div
                v-for="row in fileList"
                :key="row.path"
                class="fm-grid-item"
                :class="{ selected: isSelected(row) }"
                @click="onGridItemClick(row, $event)"
                @dblclick="onGridItemDblClick(row)"
                @contextmenu.prevent="onGridContextMenu($event, row)"
              >
                <el-checkbox
                  :model-value="isSelected(row)"
                  @click.stop
                  @change="toggleRow(row)"
                  class="fm-grid-check"
                />
                <el-icon
                  :size="40"
                  :color="row.is_dir ? 'var(--el-color-primary)' : 'var(--el-text-color-secondary)'"
                >
                  <Folder v-if="row.is_dir" />
                  <Document v-else />
                </el-icon>
                <span class="fm-grid-name" :title="row.name">{{ row.name }}</span>
                <span v-if="!row.is_dir" class="fm-grid-size">{{ formatSize(row.size) }}</span>
              </div>
              <div v-if="fileList.length === 0 && !loading" class="fm-grid-empty">
                {{ t('filesLocal.emptyDir') }}
              </div>
            </div>
          </el-scrollbar>
        </div>

        <!-- 拖拽上传遮罩：仅在拖拽进入列表区时出现；drop 由外层 handleDrop 接管
             （el-upload 的 drag 分支会把目录结构压平，所以不用它） -->
        <div v-if="dragActive" class="fm-dropzone">
          <div class="fm-dropzone-inner">
            <el-icon :size="42"><Upload /></el-icon>
            <div class="fm-dropzone-text">
              {{ t('filesLocal.dropHint', { path: currentPath || t('filesLocal.currentDir') }) }}
            </div>
          </div>
        </div>

        <!-- 上传进度面板：只渲染「正在上传」的文件行，其余用聚合统计展示，4000+ 文件也不卡 -->
        <div v-if="uploadPanelVisible && (uploadTotal || uploadScanning)" class="fm-upload-panel">
          <div class="fm-upload-head" @click="uploadPanelCollapsed = !uploadPanelCollapsed">
            <el-icon><Upload /></el-icon>
            <span class="fm-upload-title">{{ uploadPanelTitle }}</span>
            <span v-if="uploadTotal" class="fm-upload-count">
              {{ uploadFinishCount }}/{{ uploadTotal }}
            </span>
            <el-icon class="fm-upload-toggle" :class="{ expanded: !uploadPanelCollapsed }">
              <ArrowDown />
            </el-icon>
            <el-icon class="fm-upload-close" @click.stop="closeUploadPanel"><Close /></el-icon>
          </div>
          <el-progress
            v-if="uploadTotal"
            :percentage="uploadPercent"
            :stroke-width="6"
            :show-text="false"
          />
          <div v-show="!uploadPanelCollapsed" class="fm-upload-body">
            <div v-if="uploadTotal" class="fm-upload-summary">
              <span>{{ t('filesLocal.uploadOk', { n: uploadDone }) }}</span>
              <span v-if="uploadFailed" class="failed">
                {{ t('filesLocal.uploadFail', { n: uploadFailed }) }}
              </span>
              <span>{{ t('filesLocal.uploadWait', { n: uploadPendingCount }) }}</span>
            </div>

            <!-- 正在上传的文件 -->
            <div v-if="activeUploads.length" class="fm-upload-list">
              <div v-for="task in activeUploads" :key="task.id" class="fm-upload-item">
                <el-icon class="fm-upload-item-icon uploading"><Loading /></el-icon>
                <span class="fm-upload-item-name" :title="task.relPath">{{ task.relPath }}</span>
                <span class="fm-upload-item-size">{{ formatSize(task.size) }}</span>
                <span class="fm-upload-item-status uploading">{{ task.percent }}%</span>
              </div>
            </div>

            <!-- 失败的文件：可单个重试或全部重试 -->
            <div v-if="failedUploads.length" class="fm-upload-failed">
              <div class="fm-upload-failed-head">
                <span>{{ t('filesLocal.uploadFail', { n: failedUploads.length }) }}</span>
                <el-button size="small" text type="primary" @click.stop="retryAllFailed">
                  {{ t('filesLocal.retryAll') }}
                </el-button>
              </div>
              <div class="fm-upload-list">
                <div v-for="task in failedUploads" :key="task.id" class="fm-upload-item">
                  <el-icon class="fm-upload-item-icon failed"><CircleCloseFilled /></el-icon>
                  <span class="fm-upload-item-name" :title="task.relPath">{{ task.relPath }}</span>
                  <span class="fm-upload-item-size">{{ formatSize(task.size) }}</span>
                  <el-button size="small" text type="primary" @click.stop="retryUpload(task)">
                    {{ t('filesLocal.retry') }}
                  </el-button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 新建目录对话框 -->
    <!-- el-form 渲染的是原生 form + 单输入框，回车会触发浏览器隐式提交（页面刷新），
         故 @submit.prevent 阻止提交，回车确认交给 @keydown.enter -->
    <el-dialog v-model="mkdirVisible" :title="t('filesLocal.newDir')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.dirName')">
          <el-input
            v-model="mkdirName"
            :placeholder="t('filesLocal.dirNamePlaceholder')"
            @keydown.enter.prevent="onEnterConfirm($event, doMkdir)"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="mkdirVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doMkdir">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 新建文件对话框 -->
    <el-dialog v-model="newFileVisible" :title="t('filesLocal.newFile')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.fileName')">
          <el-input
            v-model="newFileName"
            :placeholder="t('filesLocal.fileNamePlaceholder')"
            @keydown.enter.prevent="onEnterConfirm($event, doNewFile)"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="newFileVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doNewFile">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 重命名对话框 -->
    <el-dialog v-model="renameVisible" :title="t('filesLocal.rename')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.newName')">
          <el-input
            v-model="renameName"
            :placeholder="t('filesLocal.newNamePlaceholder')"
            @keydown.enter.prevent="onEnterConfirm($event, doRename)"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="renameVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" @click="doRename">{{ t('common.confirm') }}</el-button>
      </template>
    </el-dialog>

    <!-- 复制副本对话框：与重命名一样先确认名字，默认名由 nextCopyName 自动生成 -->
    <el-dialog v-model="dupVisible" :title="t('filesLocal.duplicate')" width="400px">
      <el-form @submit.prevent>
        <el-form-item :label="t('filesLocal.dupName')">
          <el-input
            v-model="dupName"
            :placeholder="t('filesLocal.dupNamePlaceholder')"
            @keydown.enter.prevent="onEnterConfirm($event, doDuplicate)"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dupVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="dupSaving" @click="doDuplicate">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 修改权限对话框（cPanel 风格：八进制数字与 rwx 勾选双向联动） -->
    <el-dialog v-model="permVisible" :title="t('filesLocal.permTitle')" width="480px">
      <div class="fm-perm-target">
        <el-icon :size="16">
          <Folder v-if="permTarget?.is_dir" />
          <Document v-else />
        </el-icon>
        <span class="mono">{{ permTarget?.path }}</span>
        <span v-if="permTargets.length > 1" class="fm-perm-count">
          {{ t('filesLocal.moreItems', { n: permTargets.length }) }}
        </span>
      </div>

      <div class="fm-perm-value">
        <span class="fm-perm-value-label">{{ t('filesLocal.permValue') }}</span>
        <el-input
          v-model="permInput"
          class="fm-perm-input mono"
          :maxlength="isAdmin ? 4 : 3"
          :placeholder="permInputPlaceholder"
          @keyup.enter="doChmod"
          @input="onPermInput"
        />
        <span class="fm-perm-hint">{{ permInputHint }}</span>
      </div>

      <table class="fm-perm-table">
        <thead>
          <tr>
            <th class="fm-perm-owner"></th>
            <th>{{ t('filesLocal.permRead') }}</th>
            <th>{{ t('filesLocal.permWrite') }}</th>
            <th>{{ t('filesLocal.permExec') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td class="fm-perm-owner">{{ t('common.user') }}</td>
            <td><el-checkbox v-model="permBits.ur" @change="onPermBitsChange" /></td>
            <td><el-checkbox v-model="permBits.uw" @change="onPermBitsChange" /></td>
            <td><el-checkbox v-model="permBits.ux" @change="onPermBitsChange" /></td>
          </tr>
          <tr>
            <td class="fm-perm-owner">{{ t('common.userGroup') }}</td>
            <td><el-checkbox v-model="permBits.gr" @change="onPermBitsChange" /></td>
            <td><el-checkbox v-model="permBits.gw" @change="onPermBitsChange" /></td>
            <td><el-checkbox v-model="permBits.gx" @change="onPermBitsChange" /></td>
          </tr>
          <tr>
            <td class="fm-perm-owner">{{ t('filesLocal.permOther') }}</td>
            <td><el-checkbox v-model="permBits.or" @change="onPermBitsChange" /></td>
            <td><el-checkbox v-model="permBits.ow" @change="onPermBitsChange" /></td>
            <td><el-checkbox v-model="permBits.ox" @change="onPermBitsChange" /></td>
          </tr>
          <tr v-if="isAdmin" class="fm-perm-special">
            <td class="fm-perm-owner">{{ t('filesLocal.permSpecial') }}</td>
            <td>
              <el-checkbox v-model="permBits.suid" @change="onPermBitsChange">Set UID</el-checkbox>
            </td>
            <td>
              <el-checkbox v-model="permBits.sgid" @change="onPermBitsChange">Set GID</el-checkbox>
            </td>
            <td>
              <el-checkbox v-model="permBits.sticky" @change="onPermBitsChange">Sticky</el-checkbox>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-if="!isAdmin" class="fm-perm-note">
        {{ t('filesLocal.permNote') }}
      </div>

      <div class="fm-perm-preview">
        {{ t('filesLocal.permPreview') }}<span class="mono">{{ permOct }}</span>
        <span class="text-muted">({{ permRwx }})</span>
      </div>

      <div class="fm-perm-recursive">
        <el-checkbox v-model="permRecursive">{{ t('filesLocal.recursive') }}</el-checkbox>
      </div>

      <template #footer>
        <el-button @click="permVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="permSaving" @click="doChmod">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 修改属主/属组对话框（仅 admin）：chown/chgrp，支持递归 -->
    <el-dialog v-model="ownVisible" :title="t('filesLocal.ownTitle')" width="460px">
      <div class="fm-perm-target">
        <el-icon :size="16">
          <Folder v-if="ownTarget?.is_dir" />
          <Document v-else />
        </el-icon>
        <span class="mono">{{ ownTarget?.path }}</span>
        <span v-if="ownTargets.length > 1" class="fm-perm-count">
          {{ t('filesLocal.moreItems', { n: ownTargets.length }) }}
        </span>
      </div>

      <el-form label-width="70px" @submit.prevent>
        <el-form-item :label="t('filesLocal.ownUser')">
          <el-input v-model="ownUser" :placeholder="t('filesLocal.ownUserPlaceholder')" clearable />
        </el-form-item>
        <el-form-item :label="t('filesLocal.ownGroupLabel')">
          <el-input
            v-model="ownGroup"
            :placeholder="t('filesLocal.ownGroupPlaceholder')"
            clearable
          />
        </el-form-item>
      </el-form>

      <div class="fm-perm-recursive">
        <el-checkbox v-model="ownRecursive">{{ t('filesLocal.recursive') }}</el-checkbox>
      </div>

      <div class="fm-perm-note">
        {{ t('filesLocal.ownNote') }}
      </div>

      <template #footer>
        <el-button @click="ownVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="ownSaving" @click="doChown">
          {{ t('common.confirm') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 文件编辑对话框 -->
    <el-dialog
      v-model="editVisible"
      :title="t('filesLocal.editTitle', { name: editingFile })"
      width="70%"
      top="5vh"
      @opened="onEditorOpened"
    >
      <CodeEditor
        v-model="editContent"
        class="fm-editor"
        :path="editingFullPath"
        :placeholder="t('filesLocal.editorPlaceholder')"
      />
      <div class="fm-editor-tip">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ t('filesLocal.editorTip', { key: saveShortcut }) }}</span>
      </div>
      <template #footer>
        <el-button @click="editVisible = false">{{ t('common.close') }}</el-button>
        <el-button type="primary" :loading="saving" @click="doSaveEdit">
          {{ t('common.save') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 复制 / 移动 / 打包：统一走目录选择窗口（与新建站点的「选择已有目录」一致） -->
    <DirPicker
      v-model="pickVisible"
      :title="pickTitle"
      :start-path="currentPath"
      :confirm-text="pickConfirmText"
      :confirm-loading="pickSaving"
      @confirm="onPickConfirm"
    >
      <template #extra="{ path }">
        <div v-if="pickMode === 'archive'" class="fm-archive-name">
          <span class="fm-archive-label">{{ t('filesLocal.archiveLabel') }}</span>
          <el-input
            v-model="archiveName"
            :placeholder="t('filesLocal.archivePlaceholder')"
            @keydown.enter.prevent="onPickConfirm(path)"
          />
          <div v-if="archiveFileName" class="fm-archive-hint">
            {{ t('filesLocal.archiveWillCreate', { path: `${path}/${archiveFileName}` }) }}
          </div>
        </div>
      </template>
    </DirPicker>

    <!-- 右键菜单 -->
    <div v-if="contextMenuVisible" class="fm-context-backdrop" @click="closeContextMenu" />
    <div
      v-if="contextMenuVisible"
      class="fm-context-menu"
      :style="{ left: contextMenuX + 'px', top: contextMenuY + 'px' }"
      @click.stop
    >
      <div class="fm-context-item" :class="{ disabled: !canOpen }" @click="openSelectedFromMenu">
        <el-icon><Open /></el-icon>
        <span>{{ t('filesLocal.open') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canCopy }" @click="copyFromMenu">
        <el-icon><Copy /></el-icon>
        <span>{{ t('filesLocal.copyTo') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canDuplicate }" @click="duplicateFromMenu">
        <el-icon><Copy /></el-icon>
        <span>{{ t('filesLocal.duplicate') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canMove }" @click="moveFromMenu">
        <el-icon><Move /></el-icon>
        <span>{{ t('filesLocal.moveTo') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canDownload }" @click="downloadFromMenu">
        <el-icon><Download /></el-icon>
        <span>{{ t('filesLocal.download') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canArchive }" @click="archiveFromMenu">
        <el-icon><Archive /></el-icon>
        <span>{{ t('filesLocal.archive') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canRename }" @click="renameFromMenu">
        <el-icon><Edit /></el-icon>
        <span>{{ t('filesLocal.rename') }}</span>
      </div>
      <div class="fm-context-item" :class="{ disabled: !canSetPermissions }" @click="permFromMenu">
        <el-icon><Setting /></el-icon>
        <span>{{ t('filesLocal.perm') }}</span>
      </div>
      <div
        v-if="isAdmin"
        class="fm-context-item"
        :class="{ disabled: !hasSelection }"
        @click="ownFromMenu"
      >
        <el-icon><User /></el-icon>
        <span>{{ t('filesLocal.ownerGroup') }}</span>
      </div>
      <div class="fm-context-divider" />
      <div class="fm-context-item danger" :class="{ disabled: !canRemove }" @click="removeFromMenu">
        <el-icon><Delete /></el-icon>
        <span>{{ t('common.delete') }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import {
  ref,
  reactive,
  computed,
  onMounted,
  onBeforeUnmount,
  nextTick,
  watch,
  shallowReactive,
  markRaw,
} from 'vue'
import {
  Refresh,
  Upload,
  List,
  Grid,
  Folder,
  FolderOpened,
  Document,
  FolderAdd,
  DocumentAdd,
  Home,
  HardDrive,
  Open,
  Copy,
  Download,
  Move,
  Archive,
  Delete,
  Edit,
  Setting,
  Close,
  Loading,
  ArrowDown,
  CircleCloseFilled,
  User,
  InfoFilled,
} from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { ElTree } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { useUserStore } from '@/stores/user'
import {
  listFiles,
  readFile,
  writeFile,
  deleteFile,
  mkdir,
  renameFile,
  downloadFile as downloadFileApi,
  uploadFiles,
  chmodFile,
  chownFile,
  copyFile,
  archiveFiles,
  type FileEntry,
} from '@/api/file'
import CodeEditor from '@/components/CodeEditor.vue'
import DirPicker from '@/components/DirPicker.vue'

const { t } = useI18n()

// ── store ──────────────────────────────────────────────────

const userStore = useUserStore()
// 写操作面向所有登录用户（admin / user / reseller）开放；
// 普通用户的访问范围由后端按「本人 home 与私有 tmp」白名单兜底。
/** 仅 admin 可设置特殊位（Set UID / Set GID / Sticky），其余角色只能改 rwx */
const isAdmin = computed(() => userStore.roles.includes('admin'))

// ── state ──────────────────────────────────────────────────

const loading = ref(false)
/** 当前目录；空串表示"还没定位"——首次列目录不带 path，由后端落到家目录 */
const currentPath = ref('')
/** 家目录（后端返回，如 /home/admin）：侧栏根节点 + 地址栏起点 */
const homePath = ref('')
const fileList = ref<FileEntry[]>([])
const viewMode = ref<'list' | 'grid'>('list')
/** 当前选中的条目路径集合（支持多选） */
const selectedEntries = ref<Set<string>>(new Set())

/** 选中的条目对象（按 fileList 顺序） */
const selectedItems = computed<FileEntry[]>(() =>
  fileList.value.filter((e) => selectedEntries.value.has(e.path)),
)
const selectionCount = computed(() => selectedEntries.value.size)
const hasSelection = computed(() => selectionCount.value > 0)
const singleSelected = computed<FileEntry | null>(() =>
  hasSelection.value ? selectedItems.value[0] : null,
)

/** 当前能不能执行「下载」：单文件直接下；多选或目录则走打包 */
const canDownloadDirectly = computed(
  () => selectionCount.value === 1 && !singleSelected.value?.is_dir,
)
const hasDirectory = computed(() => selectedItems.value.some((e) => e.is_dir))

const canOpen = computed(() => selectionCount.value === 1)
const canRename = computed(() => selectionCount.value === 1)
const canDuplicate = computed(() => selectionCount.value === 1)
const canSetPermissions = computed(() => hasSelection.value)
const canCopy = computed(() => hasSelection.value)
const canMove = computed(() => hasSelection.value)
const canArchive = computed(() => hasSelection.value)
const canRemove = computed(() => hasSelection.value)
const canDownload = computed(() => hasSelection.value)

// Context menu
const contextMenuVisible = ref(false)
const contextMenuX = ref(0)
const contextMenuY = ref(0)

// Tree / Table refs
const treeRef = ref<InstanceType<typeof ElTree>>()
const tableRef = ref<any>(null)
const treeProps = { label: 'name', children: 'children', isLeaf: (data: any) => !data.is_dir }

interface TreeNode {
  name: string
  path: string
  is_dir: boolean
  children?: TreeNode[]
  /** 根节点的类型标记：家目录 / 系统根目录（普通目录不带） */
  icon?: 'home' | 'root'
}

const treeData = ref<TreeNode[]>([])

/** 侧栏根节点：家目录（cPanel 风格，普通目录都在它下面）；管理员额外给一个系统根目录 */
function buildTreeData(): TreeNode[] {
  const nodes: TreeNode[] = []
  if (homePath.value) {
    nodes.push({ name: homePath.value, path: homePath.value, is_dir: true, icon: 'home' })
  }
  // 普通用户到不了 home 之外（后端白名单），只有管理员需要这个入口
  if (isAdmin.value) {
    nodes.push({ name: '/', path: '/', is_dir: true, icon: 'root' })
  }
  // 兜底：万一没拿到 home，也留个根目录入口，别让侧栏空着
  if (nodes.length === 0) {
    nodes.push({ name: '/', path: '/', is_dir: true, icon: 'root' })
  }
  return nodes
}

/** el-tree 的 getNode 在节点不存在/未加载时会返回空值，统一兜成 null */
function treeNode(key: string) {
  return treeRef.value?.getNode(key) ?? null
}

/** 展开家目录、高亮当前目录（懒加载树里没加载到的层级保持原样） */
async function revealInTree() {
  await nextTick()
  const tree = treeRef.value
  if (!tree) return
  if (homePath.value) treeNode(homePath.value)?.expand()
  const current = currentPath.value
  if (current && treeNode(current)) tree.setCurrentKey(current)
  else if (homePath.value) tree.setCurrentKey(homePath.value)
}

/** 导航后同步高亮：目标目录已经在树里（展开过）才动，避免误展开一堆分支 */
function syncTree() {
  const current = currentPath.value
  if (current && treeNode(current)) treeRef.value?.setCurrentKey(current)
}

// Dialogs
const mkdirVisible = ref(false)
const mkdirName = ref('')
const newFileVisible = ref(false)
const newFileName = ref('')
const renameVisible = ref(false)
const renameTarget = ref<FileEntry | null>(null)
const renameName = ref('')
const dupVisible = ref(false)
const dupTarget = ref<FileEntry | null>(null)
const dupName = ref('')
const dupSaving = ref(false)

// 复制 / 移动 / 打包：共用一个目录选择窗口（打包额外带压缩包名称）
type PickMode = 'copy' | 'move' | 'archive'
const pickVisible = ref(false)
const pickMode = ref<PickMode>('copy')
const pickSaving = ref(false)
const archiveName = ref('')
const pickTitle = computed(
  () =>
    ({
      copy: t('filesLocal.pickCopyTitle'),
      move: t('filesLocal.pickMoveTitle'),
      archive: t('filesLocal.pickArchiveTitle'),
    })[pickMode.value],
)
const pickConfirmText = computed(
  () =>
    ({
      copy: t('filesLocal.pickCopyConfirm'),
      move: t('filesLocal.pickMoveConfirm'),
      archive: t('filesLocal.pickArchiveConfirm'),
    })[pickMode.value],
)
/** 打包时最终生成的文件名（后端会自动补 .zip），用于窗口内的预览提示 */
const archiveFileName = computed(() => {
  const n = archiveName.value.trim()
  if (!n) return ''
  return n.toLowerCase().endsWith('.zip') ? n : `${n}.zip`
})

const editVisible = ref(false)
const editingFile = ref('')
const editingFullPath = ref('')
const editContent = ref('')
const saving = ref(false)

/** macOS 用 ⌘ + S，其余平台用 Ctrl + S：提示文案与实际监听的按键保持一致 */
const isMac = /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent)
const saveShortcut = computed(() => (isMac ? '⌘ + S' : 'Ctrl + S'))

/** 编辑器内 Ctrl / Cmd + S 直接保存（浏览器自己的「保存网页」要拦掉） */
function onEditKeydown(e: KeyboardEvent) {
  if (!editVisible.value) return
  if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 's') return
  e.preventDefault()
  if (saving.value) return
  void doSaveEdit()
}

// ── breadcrumbs ────────────────────────────────────────────

interface Crumb {
  label: string
  path: string
  title: string
  /** 首段用图标代替文字：家目录 = 房子，系统根目录 = 磁盘 */
  icon?: 'home' | 'root'
}

/**
 * 地址栏段落：
 * - 家目录内：首段是家目录图标（悬停可见完整路径），后面是相对路径，
 *   地址栏比整条绝对路径短得多，和左侧「以家目录为根」的树也对得上；
 * - 家目录外（管理员翻系统目录）：首段是根目录图标，后面是绝对路径。
 */
const crumbs = computed<Crumb[]>(() => {
  const home = homePath.value
  const current = currentPath.value || home || '/'

  if (home && (current === home || current.startsWith(home + '/'))) {
    const rest = current.slice(home.length).split('/').filter(Boolean)
    return [
      { label: home, path: home, title: t('filesLocal.crumbHome', { path: home }), icon: 'home' },
      ...rest.map((name, idx) => {
        const path = `${home}/${rest.slice(0, idx + 1).join('/')}`
        return { label: name, path, title: path }
      }),
    ]
  }

  const segments = current.split('/').filter(Boolean)
  return [
    { label: '/', path: '/', title: t('filesLocal.crumbRoot'), icon: 'root' },
    ...segments.map((name, idx) => {
      const path = `/${segments.slice(0, idx + 1).join('/')}`
      return { label: name, path, title: path }
    }),
  ]
})

// ── tree ───────────────────────────────────────────────────

/**
 * 懒加载子目录。
 *
 * 坑：`lazy` 模式下 el-tree 初始化时会跳过 setData，并对「根节点」调一次 load
 * （node.level === 0），再把返回值 append 成顶层节点。我们的顶层两个节点
 * （家目录 / 系统根目录）由 `:data` 提供，所以根调用必须返回空数组——
 * 否则根节点的 data 是那个数组本身、取不到 path，一旦回落到 `/` 就会把
 * 整个根目录的列表追加成第三组顶级节点（侧栏看起来重复）。
 */
async function loadTreeNode(node: any, resolve: (data: TreeNode[]) => void) {
  const path: string | undefined = node.data?.path
  if (node.level === 0 || !path) {
    resolve([])
    return
  }
  try {
    const res = await listFiles(path)
    const entries = res.data?.entries || []
    resolve(
      entries
        .filter((e) => e.is_dir)
        .map((e) => ({
          name: e.name,
          path: e.path,
          is_dir: true,
        })),
    )
  } catch {
    resolve([])
  }
}

function onTreeNodeClick(data: TreeNode) {
  if (data.path) {
    navigateTo(data.path)
  }
}

async function refreshTree() {
  // 换一个全新的根节点数组，el-tree 会丢掉懒加载缓存重新拉取
  treeData.value = buildTreeData()
  await revealInTree()
  refreshList()
}

// ── file list ──────────────────────────────────────────────

async function loadFileList() {
  loading.value = true
  try {
    // 首次不带 path：后端落到家目录（管理员也是），回来后再以实际目录为准
    const res = await listFiles(currentPath.value)
    const data = res.data
    if (data?.home) homePath.value = data.home
    if (data?.current_path) currentPath.value = data.current_path
    fileList.value = data?.entries || []
    clearSelection()
    syncTree()
  } catch {
    // handled by interceptor
  } finally {
    loading.value = false
  }
}

function refreshList() {
  loadFileList()
}

function navigateTo(path: string) {
  currentPath.value = path
  loadFileList()
}

function navigateToCrumb(idx: number) {
  const crumb = crumbs.value[idx]
  // 最后一段就是当前目录，点它不必再请求一次
  if (!crumb || idx === crumbs.value.length - 1) return
  navigateTo(crumb.path)
}

/** 当前目录下的完整路径（首次列目录前 currentPath 为空，按根目录拼） */
function joinCurrent(name: string): string {
  const dir = currentPath.value
  return !dir || dir === '/' ? `/${name}` : `${dir}/${name}`
}

/** 同步选中态到 el-table 时，忽略其 selection-change 回灌，避免互相覆盖 */
let syncingSelection = false

function syncTableSelection() {
  const t = tableRef.value
  if (!t) return
  syncingSelection = true
  t.clearSelection?.()
  fileList.value.forEach((r) => {
    if (selectedEntries.value.has(r.path)) t.toggleRowSelection?.(r, true)
  })
  syncingSelection = false
}

function clearSelection() {
  selectedEntries.value = new Set()
  syncTableSelection()
}

function toggleRow(row: FileEntry) {
  const next = new Set(selectedEntries.value)
  if (next.has(row.path)) next.delete(row.path)
  else next.add(row.path)
  selectedEntries.value = next
  syncTableSelection()
}

function setSelection(row: FileEntry) {
  selectedEntries.value = new Set([row.path])
  syncTableSelection()
}

function isSelected(row: FileEntry) {
  return selectedEntries.value.has(row.path)
}

let lastClickedPath = ''

function handleRowClick(row: FileEntry, event?: MouseEvent) {
  if (event && (event.ctrlKey || event.metaKey)) {
    toggleRow(row)
    lastClickedPath = row.path
    return
  }
  if (event && event.shiftKey && lastClickedPath) {
    const paths = fileList.value.map((e) => e.path)
    const start = paths.indexOf(lastClickedPath)
    const end = paths.indexOf(row.path)
    if (start !== -1 && end !== -1) {
      const range = fileList.value.slice(Math.min(start, end), Math.max(start, end) + 1)
      const next = new Set(selectedEntries.value)
      range.forEach((e) => next.add(e.path))
      selectedEntries.value = next
      syncTableSelection()
      return
    }
  }
  setSelection(row)
  lastClickedPath = row.path
}

function onRowClick(row: FileEntry, _column: any, event: MouseEvent) {
  handleRowClick(row, event)
}

async function onRowDblClick(row: FileEntry) {
  if (row.is_dir) {
    navigateTo(row.path)
  } else {
    await openFileEditor(row)
  }
}

function onTableSelectionChange(rows: FileEntry[]) {
  if (syncingSelection) return
  selectedEntries.value = new Set(rows.map((r) => r.path))
}

function onGridItemClick(row: FileEntry, event: MouseEvent) {
  handleRowClick(row, event)
}

function onGridItemDblClick(row: FileEntry) {
  onRowDblClick(row)
}

function rowClassName({ row }: { row: FileEntry }) {
  return isSelected(row) ? 'selected-row' : ''
}

function onTableRowContextMenu(row: FileEntry, _column: any, event: MouseEvent) {
  openContextMenu(event, row)
}

function onGridContextMenu(event: MouseEvent, row: FileEntry) {
  openContextMenu(event, row)
}

function openContextMenu(event: MouseEvent, row?: FileEntry) {
  event.preventDefault()
  if (row && !isSelected(row)) setSelection(row)
  contextMenuX.value = event.clientX
  contextMenuY.value = event.clientY
  contextMenuVisible.value = true
}

function closeContextMenu() {
  contextMenuVisible.value = false
}

function openSelectedFromMenu() {
  closeContextMenu()
  openSelected()
}
function copyFromMenu() {
  closeContextMenu()
  showCopyDialog(false)
}
function duplicateFromMenu() {
  closeContextMenu()
  showDuplicateDialog()
}
function moveFromMenu() {
  closeContextMenu()
  showMoveDialog(false)
}
function downloadFromMenu() {
  closeContextMenu()
  downloadSelected()
}
function archiveFromMenu() {
  closeContextMenu()
  showArchiveDialog()
}
function renameFromMenu() {
  closeContextMenu()
  if (singleSelected.value) showRenameDialog(singleSelected.value)
}
function permFromMenu() {
  closeContextMenu()
  showPermDialogForSelection()
}
function ownFromMenu() {
  closeContextMenu()
  showOwnerDialogForSelection()
}
function removeFromMenu() {
  closeContextMenu()
  removeSelected()
}

// ── file operations ────────────────────────────────────────

async function handleDownload(row: FileEntry) {
  if (row.is_dir) {
    ElMessage.warning(t('filesLocal.errDownloadDir'))
    return
  }
  try {
    const blob = await downloadFileApi(row.path)
    const url = window.URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = row.name
    a.click()
    window.URL.revokeObjectURL(url)
    ElMessage.success(t('filesLocal.downloadOk'))
  } catch {
    // handled by interceptor
  }
}

async function openFileEditor(row: FileEntry) {
  try {
    const res = await readFile(row.path)
    editingFile.value = row.name
    editingFullPath.value = row.path
    editContent.value = res.data?.content || ''
    editVisible.value = true
  } catch {
    // handled
  }
}

async function doSaveEdit() {
  saving.value = true
  try {
    const fullPath = editingFullPath.value || joinCurrent(editingFile.value)
    await writeFile(fullPath, editContent.value)
    ElMessage.success(t('filesLocal.saveOk'))
    editVisible.value = false
    loadFileList()
  } catch {
    // handled
  } finally {
    saving.value = false
  }
}

function onEditorOpened() {
  // Focus the textarea
}

/**
 * 对话框回车确认：忽略输入法组词（中文输入）过程中的回车，
 * 否则按回车选词时会用半截名字直接提交。
 */
function onEnterConfirm(e: Event, action: () => void) {
  const ev = e as KeyboardEvent
  if (ev.isComposing || ev.keyCode === 229) return
  action()
}

function showMkdirDialog() {
  mkdirName.value = ''
  mkdirVisible.value = true
}

async function doMkdir() {
  if (!mkdirName.value.trim()) {
    ElMessage.warning(t('filesLocal.needDirName'))
    return
  }
  const fullPath = joinCurrent(mkdirName.value.trim())
  try {
    await mkdir(fullPath)
    ElMessage.success(t('filesLocal.dirCreated'))
    mkdirVisible.value = false
    loadFileList()
    refreshTree()
  } catch {
    // handled
  }
}

function showNewFileDialog() {
  newFileName.value = ''
  newFileVisible.value = true
}

async function doNewFile() {
  if (!newFileName.value.trim()) {
    ElMessage.warning(t('filesLocal.needFileName'))
    return
  }
  const fullPath = joinCurrent(newFileName.value.trim())
  try {
    await writeFile(fullPath, '')
    ElMessage.success(t('filesLocal.fileCreated'))
    newFileVisible.value = false
    loadFileList()
  } catch {
    // handled
  }
}

function showRenameDialog(row: FileEntry) {
  renameTarget.value = row
  renameName.value = row.name
  renameVisible.value = true
}

async function doRename() {
  if (!renameTarget.value || !renameName.value.trim()) {
    ElMessage.warning(t('filesLocal.needNewName'))
    return
  }
  const newPath = joinCurrent(renameName.value.trim())

  try {
    await renameFile(renameTarget.value.path, newPath)
    ElMessage.success(t('filesLocal.renamed'))
    renameVisible.value = false
    loadFileList()
    refreshTree()
  } catch {
    // handled
  }
}

// ── 上传（点选 / 目录 / 拖拽） ────────────────────────────────

/** 拖拽的文件是否悬停在列表区上方 */
const dragActive = ref(false)

function onDragEnter() {
  dragActive.value = true
}

function onDragOver() {
  dragActive.value = true
}

/** 用 relatedTarget 判断，移到列表区内部（表格 / 遮罩）不算离开，避免闪烁 */
function onDragLeave(e: DragEvent) {
  const el = e.currentTarget as HTMLElement | null
  const to = e.relatedTarget as Node | null
  if (!el || !to || !el.contains(to)) dragActive.value = false
}

/**
 * 递归展开拖入的目录项。
 *
 * 不能用 `dataTransfer.files`：拖文件夹时它只给一个 0 字节的目录项，既没有内部文件
 * 也没有层级。这里用 `webkitGetAsEntry()` 自己遍历，并把 `entry.fullPath`
 * （形如 `/dir/sub/a.txt`）写回 `webkitRelativePath`，复用「上传目录」按钮同一条链路，
 * 由后端按相对路径逐级建目录还原结构。
 */
function walkEntry(entry: FileSystemEntry, out: File[]): Promise<void> {
  if (entry.isFile) {
    return new Promise((resolve) => {
      ;(entry as FileSystemFileEntry).file(
        (file) => {
          // webkitRelativePath 是 File.prototype 上的只读 getter（无 setter），
          // 用 defineProperty 造一个同名自有属性，让 uploadFiles 读到层级路径
          Object.defineProperty(file, 'webkitRelativePath', {
            value: entry.fullPath.replace(/^\//, ''),
            configurable: true,
          })
          out.push(file)
          resolve()
        },
        // 单个文件读失败（无权限 / 拖拽期间被删）就跳过，不阻断整次拖拽
        () => resolve(),
      )
    })
  }
  if (!entry.isDirectory) return Promise.resolve()

  const reader = (entry as FileSystemDirectoryEntry).createReader()
  // Chromium 单次 readEntries 最多返回 100 条，必须循环读到空才会拿到全部子项
  const readBatch = () =>
    new Promise<FileSystemEntry[]>((resolve) => reader.readEntries(resolve, () => resolve([])))

  return (async () => {
    for (;;) {
      const batch = await readBatch()
      if (!batch.length) break
      for (const child of batch) await walkEntry(child, out)
    }
  })()
}

/** 能拿到 entry 就走递归（保留层级）；拿不到时退回 dataTransfer.files（无层级） */
async function collectDroppedFiles(dt: DataTransfer | null): Promise<File[]> {
  const entries = Array.from(dt?.items ?? [])
    .filter((item) => item.kind === 'file')
    .map((item) => item.webkitGetAsEntry?.())
    .filter((entry): entry is FileSystemEntry => !!entry)

  if (entries.length) {
    const out: File[] = []
    for (const entry of entries) await walkEntry(entry, out)
    return out
  }
  return dt ? Array.from(dt.files) : []
}

// ── 上传队列（拖拽 / 按钮共用）────────────────────────────────
// 一个文件一个请求：后端本来就是逐个 multipart field 处理，拆开才有「每个文件自己的
// 进度」，也避免一次把整个目录读进内存；同时传的文件数由 UPLOAD_CONCURRENCY 控制。

/** 并发数：太小浪费带宽，太大进度条会乱跳 */
const UPLOAD_CONCURRENCY = 3

type UploadStatus = 'pending' | 'uploading' | 'done' | 'failed'

interface UploadTask {
  id: number
  file: File
  /** 目标目录：重试时沿用原目录，避免用户切目录后传错地方 */
  dir: string
  /** 相对路径（目录上传带层级，如 `upload/index.php`），用于展示 */
  relPath: string
  size: number
  /** 已传字节：按 percent 折算，避免把 multipart 头部算进进度 */
  loaded: number
  percent: number
  status: UploadStatus
}

interface UploadJob {
  task: UploadTask
  dir: string
  /** 批量入队时没有等待者，可以不传 */
  resolve?: () => void
}

const uploadPanelVisible = ref(false)
const uploadPanelCollapsed = ref(false)
const uploadBusy = ref(false)
/** 拖拽目录时正在递归遍历 entry（还没开始传），用于显示「读取中」 */
const uploadScanning = ref(false)

/** 只放「正在上传」的文件（≤ 并发数），逐行显示进度；结束立即移出，避免渲染上千行 */
const activeUploads = ref<UploadTask[]>([])
/** 失败的文件：留在列表里，支持单个/全部重新上传 */
const failedUploads = ref<UploadTask[]>([])
/**
 * 整轮的聚合统计：入队/完成时增量更新。
 * 之前用 computed 对全部任务做 filter/reduce，4000+ 文件时每次进度事件都是 O(n)，
 * 直接把主线程打满；改成计数器后每次更新都是 O(1)。
 */
const uploadTotal = ref(0)
const uploadTotalBytes = ref(0)
const uploadLoadedBytes = ref(0)
const uploadDone = ref(0)
const uploadFailed = ref(0)

const uploadJobs: UploadJob[] = []
let activeJobs = 0
let uploadSeq = 0

/** 已出结果的文件数（成功 + 失败） */
const uploadFinishCount = computed(() => uploadDone.value + uploadFailed.value)
/** 还没开始的排队数（不含正在上传的） */
const uploadPendingCount = computed(() =>
  Math.max(0, uploadTotal.value - uploadFinishCount.value - activeUploads.value.length),
)
/** 本轮是否还有未完成的文件 */
const uploadInFlight = computed(() => uploadTotal.value > uploadFinishCount.value)

/** 总进度：按字节加权；全是空文件时退化成按文件数 */
const uploadPercent = computed(() => {
  if (!uploadTotal.value) return 0
  const ratio = uploadTotalBytes.value
    ? uploadLoadedBytes.value / uploadTotalBytes.value
    : uploadFinishCount.value / uploadTotal.value
  return Math.min(100, Math.round(ratio * 100))
})

const uploadPanelTitle = computed(() => {
  if (uploadScanning.value && !uploadTotal.value) return t('filesLocal.uploadingScan')
  if (uploadInFlight.value) {
    return t('filesLocal.uploadingCount', { n: uploadTotal.value })
  }
  return uploadFailed.value ? t('filesLocal.uploadEndFailed') : t('filesLocal.uploadEndOk')
})

/** 开始新一轮前清空上一轮的统计与显示 */
function resetUploadRound() {
  activeUploads.value = []
  failedUploads.value = []
  uploadTotal.value = 0
  uploadTotalBytes.value = 0
  uploadLoadedBytes.value = 0
  uploadDone.value = 0
  uploadFailed.value = 0
  uploadPanelCollapsed.value = false
}

function createUploadTask(dir: string, file: File): UploadTask {
  return shallowReactive<UploadTask>({
    id: ++uploadSeq,
    // File 是平台对象，不要深响应，否则传给 FormData 可能出问题
    file: markRaw(file),
    dir,
    relPath: file.webkitRelativePath || file.name,
    size: file.size,
    loaded: 0,
    percent: 0,
    status: 'pending',
  })
}

/**
 * 批量入队（拖拽目录走这里）。
 * 关键优化：4000+ 文件时只做一次统计写入，避免每个文件都触发一轮响应式更新。
 */
function enqueueUploads(dir: string, files: File[]) {
  if (!files.length) return
  if (!uploadInFlight.value && uploadTotal.value) resetUploadRound()
  uploadPanelVisible.value = true
  uploadBusy.value = true

  let totalBytes = 0
  for (const file of files) {
    totalBytes += file.size
    uploadJobs.push({ task: createUploadTask(dir, file), dir })
  }
  uploadTotal.value += files.length
  uploadTotalBytes.value += totalBytes
  pumpUploadQueue()
}

/** 单个文件入队（按钮上传走这里）；返回的 Promise 在该文件结束（成功或失败）时 resolve */
function enqueueUpload(dir: string, file: File): Promise<void> {
  if (!uploadInFlight.value && uploadTotal.value) resetUploadRound()
  uploadPanelVisible.value = true
  uploadBusy.value = true
  uploadTotal.value++
  uploadTotalBytes.value += file.size
  return new Promise((resolve) => {
    uploadJobs.push({ task: createUploadTask(dir, file), dir, resolve })
    pumpUploadQueue()
  })
}

function pumpUploadQueue() {
  while (activeJobs < UPLOAD_CONCURRENCY && uploadJobs.length) {
    const job = uploadJobs.shift()
    if (!job) break
    activeJobs++
    void runUploadJob(job)
  }
}

async function runUploadJob(job: UploadJob) {
  const { task } = job
  task.status = 'uploading'
  activeUploads.value.push(task)
  let lastPercent = -1
  try {
    await uploadFiles(job.dir, [task.file], (percent) => {
      // 同一百分比不重复发响应式更新，进一步降低高频事件带来的重渲染
      if (percent === lastPercent) return
      lastPercent = percent
      const nextLoaded = Math.round((task.size * percent) / 100)
      uploadLoadedBytes.value += nextLoaded - task.loaded
      task.percent = percent
      task.loaded = nextLoaded
    })
    task.status = 'done'
    task.percent = 100
    uploadLoadedBytes.value += task.size - task.loaded
    task.loaded = task.size
    uploadDone.value++
  } catch {
    // 失败原因由响应拦截器统一提示，这里只标记这一行，不影响同批其他文件
    task.status = 'failed'
    uploadFailed.value++
    failedUploads.value.push(task)
  } finally {
    const idx = activeUploads.value.indexOf(task)
    if (idx !== -1) activeUploads.value.splice(idx, 1)
    activeJobs--
    job.resolve?.()
    pumpUploadQueue()
    if (!activeJobs && !uploadJobs.length) finishUploadRound()
  }
}

/** 整轮结束：聚合提示一次并刷新列表；全部成功时自动收起进度面板 */
function finishUploadRound() {
  const total = uploadTotal.value
  const failed = uploadFailed.value
  uploadBusy.value = false
  loadFileList()
  refreshTree()
  if (failed === 0) {
    ElMessage.success(t('filesLocal.uploadedAll', { n: total }))
    setTimeout(() => {
      if (!uploadBusy.value) uploadPanelVisible.value = false
    }, 3000)
  } else if (failed < total) {
    ElMessage.warning(t('filesLocal.uploadSummary', { ok: total - failed, failed }))
  } else {
    ElMessage.error(t('filesLocal.uploadAllFailed', { n: failed }))
  }
}

/**
 * 重试单个失败文件：从失败列表移回队列重新上传。
 * 不重置整轮统计，只回退该文件的失败计数与已计字节，顶部进度会实时反映。
 */
function retryUpload(task: UploadTask) {
  const idx = failedUploads.value.indexOf(task)
  if (idx === -1) return
  failedUploads.value.splice(idx, 1)
  uploadFailed.value--
  uploadLoadedBytes.value -= task.loaded
  task.loaded = 0
  task.percent = 0
  task.status = 'pending'
  uploadPanelVisible.value = true
  uploadBusy.value = true
  uploadJobs.push({ task, dir: task.dir })
  pumpUploadQueue()
}

/** 重试全部失败文件 */
function retryAllFailed() {
  for (const task of [...failedUploads.value]) retryUpload(task)
}

function closeUploadPanel() {
  uploadPanelVisible.value = false
}

/**
 * 拖拽落点：文件与文件夹都支持。
 *
 * 走自己的 drop（而不是 el-upload 的 drag）是因为它的拖拽分支靠 `entry.file()`
 * 拿文件，`webkitRelativePath` 为空，目录会被压平；这里由 walkEntry 把层级补回去。
 */
async function handleDrop(e: DragEvent) {
  dragActive.value = false
  // 目录要递归遍历，文件多时可能花上一两秒：超过 300ms 才提示，避免普通拖文件闪一下
  const scanTimer = setTimeout(() => {
    uploadScanning.value = true
    uploadPanelVisible.value = true
  }, 300)
  let files: File[] = []
  try {
    files = await collectDroppedFiles(e.dataTransfer)
  } finally {
    clearTimeout(scanTimer)
    uploadScanning.value = false
  }
  if (!files.length) return
  const dir = currentPath.value
  enqueueUploads(dir, files)
}

async function handleUpload(options: any) {
  await enqueueUpload(currentPath.value, options.file as File)
}

// ── selection actions ────────────────────────────────────────

function openSelected() {
  const item = singleSelected.value
  if (!item) return
  if (item.is_dir) {
    navigateTo(item.path)
  } else {
    openFileEditor(item)
  }
}

function downloadBlob(name: string, content: string) {
  const bytes = zapProtoB64Decode(content)
  const url = window.URL.createObjectURL(new Blob([bytes as BlobPart]))
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  window.URL.revokeObjectURL(url)
}

function zapProtoB64Decode(content: string): Uint8Array {
  const bin = atob(content.replace(/\s/g, ''))
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i)
  }
  return out
}

async function downloadSelected() {
  if (!hasSelection.value) return
  if (canDownloadDirectly.value && singleSelected.value) {
    await handleDownload(singleSelected.value)
    return
  }
  const name = archiveName.value || `download_${Date.now()}`
  const paths = selectedItems.value.map((e) => e.path)
  try {
    // 不传目标目录：后端返回 zip 的 base64 内容，这里直接触发浏览器下载
    const res = await archiveFiles(paths, name, currentPath.value)
    const data = res.data
    if (!data?.content) return
    downloadBlob(data.name, data.content)
    ElMessage.success(t('filesLocal.downloadStart'))
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.downloadFailed'))
  }
}

/** 打开目录选择窗口：复制 / 移动 / 打包共用同一个窗口，只是标题与确认文案不同 */
function openPicker(mode: PickMode, fromMenu = false) {
  if (!hasSelection.value) return
  if (fromMenu && singleSelected.value) setSelection(singleSelected.value)
  pickMode.value = mode
  if (mode === 'archive') archiveName.value = `archive_${Date.now()}`
  pickVisible.value = true
}

function showCopyDialog(fromMenu: boolean) {
  openPicker('copy', fromMenu)
}

function showMoveDialog(fromMenu: boolean) {
  openPicker('move', fromMenu)
}

function showArchiveDialog() {
  openPicker('archive')
}

async function onPickConfirm(dir: string) {
  if (pickMode.value === 'copy') await doCopy(dir)
  else if (pickMode.value === 'move') await doMove(dir)
  else await doArchive(dir)
}

async function doCopy(dir: string) {
  const items = selectedItems.value
  const target = dir.trim()
  if (!items.length || !target) {
    ElMessage.warning(t('filesLocal.needTargetDir'))
    return
  }
  // 目标就是条目当前所在目录时后端会报错，这里提前拦下
  const same = items.find((it) => `${target}/${it.name}` === it.path)
  if (same) {
    ElMessage.warning(t('filesLocal.alreadyInDir', { name: same.name }))
    return
  }
  pickSaving.value = true
  try {
    for (const item of items) {
      await copyFile(item.path, `${target}/${item.name}`)
    }
    ElMessage.success(t('filesLocal.copiedTo', { n: items.length, target }))
    pickVisible.value = false
    loadFileList()
    refreshTree()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.copyFailed'))
  } finally {
    pickSaving.value = false
  }
}

async function doMove(dir: string) {
  const items = selectedItems.value
  const target = dir.trim()
  if (!items.length || !target) {
    ElMessage.warning(t('filesLocal.needTargetDir'))
    return
  }
  if (target === currentPath.value) {
    ElMessage.warning(t('filesLocal.sameDirNoMove'))
    return
  }
  // 目录不能移动到自己或自己的子目录里
  const inside = items.find(
    (it) => it.is_dir && (target === it.path || target.startsWith(`${it.path}/`)),
  )
  if (inside) {
    ElMessage.warning(t('filesLocal.moveIntoSelf', { name: inside.name }))
    return
  }
  pickSaving.value = true
  try {
    for (const item of items) {
      await renameFile(item.path, `${target}/${item.name}`)
    }
    ElMessage.success(t('filesLocal.movedTo', { n: items.length, target }))
    pickVisible.value = false
    loadFileList()
    refreshTree()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.moveFailed'))
  } finally {
    pickSaving.value = false
  }
}

/** 打包只在目标目录里生成压缩包，不做下载 */
async function doArchive(dir: string) {
  const items = selectedItems.value
  const target = dir.trim()
  const name = archiveName.value.trim()
  if (!items.length || !target) {
    ElMessage.warning(t('filesLocal.needTargetDir'))
    return
  }
  if (!name) {
    ElMessage.warning(t('filesLocal.needArchiveName'))
    return
  }
  pickSaving.value = true
  try {
    const res = await archiveFiles(
      items.map((e) => e.path),
      name,
      currentPath.value,
      target,
    )
    ElMessage.success(
      t('filesLocal.archiveCreated', { path: res.data?.path || archiveFileName.value }),
    )
    pickVisible.value = false
    loadFileList()
    refreshTree()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.archiveFailed'))
  } finally {
    pickSaving.value = false
  }
}

/** 拆扩展名：目录、以及 `.bashrc` 这类点开头的隐藏文件都算没有扩展名 */
function splitExt(name: string, isDir: boolean): { base: string; ext: string } {
  if (isDir) return { base: name, ext: '' }
  const dot = name.lastIndexOf('.')
  if (dot <= 0) return { base: name, ext: '' }
  return { base: name.slice(0, dot), ext: name.slice(dot) }
}

/** 取父目录（副本建在原条目所在目录，不受 currentPath 影响） */
function parentDir(path: string): string {
  const i = path.lastIndexOf('/')
  return i <= 0 ? '/' : path.slice(0, i)
}

/**
 * 自动生成默认副本名：把 `_copy` 插在扩展名前（目录直接追加），
 * 如 `index.php` → `index_copy.php`；当前目录里已被占用则依次试 `_copy2`、`_copy3` …
 */
function nextCopyName(item: FileEntry): string {
  const { base, ext } = splitExt(item.name, item.is_dir)
  const taken = new Set(fileList.value.map((e) => e.name))
  for (let idx = 1; idx <= 100; idx++) {
    const name = `${base}${idx === 1 ? '_copy' : `_copy${idx}`}${ext}`
    if (!taken.has(name)) return name
  }
  return `${base}_copy${Date.now()}${ext}`
}

/** 复制副本：先让用户确认副本名（默认预填自动生成的副本名） */
function showDuplicateDialog() {
  const item = singleSelected.value
  if (!item) return
  dupTarget.value = item
  dupName.value = nextCopyName(item)
  dupVisible.value = true
}

async function doDuplicate() {
  const item = dupTarget.value
  const name = dupName.value.trim()
  if (!item || !name) {
    ElMessage.warning(t('filesLocal.needDupName'))
    return
  }
  if (name.includes('/')) {
    ElMessage.warning(t('filesLocal.dupNameNoSlash'))
    return
  }
  if (name === item.name) {
    ElMessage.warning(t('filesLocal.dupNameSame'))
    return
  }
  if (fileList.value.some((e) => e.name === name)) {
    ElMessage.warning(t('filesLocal.nameExists', { name }))
    return
  }
  dupSaving.value = true
  try {
    await copyFile(item.path, `${parentDir(item.path)}/${name}`)
    ElMessage.success(t('filesLocal.dupOk'))
    dupVisible.value = false
    loadFileList()
    refreshTree()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.copyFailed'))
  } finally {
    dupSaving.value = false
  }
}

async function removeSelected() {
  if (!hasSelection.value) return
  try {
    await ElMessageBox.confirm(
      t('filesLocal.delConfirm', {
        n: selectionCount.value,
        dirTip: hasDirectory.value ? t('filesLocal.delConfirmDirTip') : '',
      }),
      t('filesLocal.delWarning'),
      { type: 'warning', confirmButtonText: t('filesLocal.delConfirmBtn') },
    )
  } catch {
    return
  }
  try {
    for (const item of selectedItems.value) {
      await deleteFile(item.path)
    }
    ElMessage.success(t('filesLocal.deleteOk'))
    loadFileList()
    refreshTree()
  } catch (e: any) {
    ElMessage.error(e?.message || t('filesLocal.deleteFailed'))
  }
}

// ── utils ──────────────────────────────────────────────────

function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const k = 1024
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + units[i]
}

// ── 权限修改（cPanel 风格：八进制数字 ↔ rwx 勾选联动）─────

const permVisible = ref(false)
const permTarget = ref<FileEntry | null>(null)
const permTargets = ref<FileEntry[]>([])
const permInput = ref('0755')
const permSaving = ref(false)
/** 是否递归修改选中目录下的所有子项（所有角色都可用） */
const permRecursive = ref(false)

/** 特殊位掩码（Set UID / Set GID / Sticky） */
const PERM_SPECIAL_MASK = 0o7000

/** 当前用户可编辑的权限位掩码：admin 含特殊位，其余角色仅 rwx */
const permMask = computed(() => (isAdmin.value ? 0o7777 : 0o777))

/**
 * 非 admin 提交时原样带上的目标文件特殊位：
 * 后端只接受「不改变特殊位」的请求，普通用户既不能新增也无法清除特殊位。
 */
const permKeepSpecial = computed(() =>
  isAdmin.value ? 0 : (permTarget.value?.mode ?? 0) & PERM_SPECIAL_MASK,
)

/** 权限位开关：用户/组/其他的 rwx + 三个特殊位 */
const permBits = reactive({
  ur: false,
  uw: false,
  ux: false,
  gr: false,
  gw: false,
  gx: false,
  or: false,
  ow: false,
  ox: false,
  suid: false,
  sgid: false,
  sticky: false,
})

type PermBitKey = keyof typeof permBits

/** 位名 → 掩码（顺序即展示顺序） */
const PERM_BITS: Array<[PermBitKey, number]> = [
  ['ur', 0o400],
  ['uw', 0o200],
  ['ux', 0o100],
  ['gr', 0o040],
  ['gw', 0o020],
  ['gx', 0o010],
  ['or', 0o004],
  ['ow', 0o002],
  ['ox', 0o001],
  ['suid', 0o4000],
  ['sgid', 0o2000],
  ['sticky', 0o1000],
]

function bitsToMode(): number {
  const mode = PERM_BITS.reduce((acc, [key, mask]) => (permBits[key] ? acc | mask : acc), 0)
  return mode & permMask.value
}

function modeToBits(mode: number) {
  for (const [key, mask] of PERM_BITS) {
    permBits[key] = (mode & mask & permMask.value) !== 0
  }
}

/** 权限数值 → 八进制 4 位文本（493 → '0755'） */
function toOct4(mode: number): string {
  return (mode & 0o7777).toString(8).padStart(4, '0')
}

/** 权限数值 → 输入框/预览文本：admin 为 4 位（含特殊位），其余角色为 3 位 rwx */
function toPermText(mode: number): string {
  const masked = mode & permMask.value
  return isAdmin.value ? toOct4(masked) : (masked & 0o777).toString(8).padStart(3, '0')
}

/** 权限文本 → 数值（非法输入回退 0755） */
function fromOct(text: string): number {
  const digits = (text || '').trim().replace(/^0o/i, '')
  return /^[0-7]{1,4}$/.test(digits) ? parseInt(digits, 8) : 0o755
}

const permInputPlaceholder = computed(() => (isAdmin.value ? '0755' : '755'))
const permInputHint = computed(() =>
  isAdmin.value ? t('filesLocal.permHintAdmin') : t('filesLocal.permHintUser'),
)

const permMode = computed(() => bitsToMode())
const permOct = computed(() => toPermText(permMode.value))
const permRwx = computed(() => {
  const mode = permMode.value
  const triples: Array<[number, number, number, number, string]> = [
    [0o400, 0o200, 0o100, 0o4000, 's'],
    [0o040, 0o020, 0o010, 0o2000, 's'],
    [0o004, 0o002, 0o001, 0o1000, 't'],
  ]
  let out = ''
  for (const [r, w, x, special, specialChar] of triples) {
    out += mode & r ? 'r' : '-'
    out += mode & w ? 'w' : '-'
    const hasX = (mode & x) !== 0
    const hasSpecial = (mode & special) !== 0
    if (hasSpecial) out += hasX ? specialChar : specialChar.toUpperCase()
    else out += hasX ? 'x' : '-'
  }
  return out
})

function showPermDialog(row: FileEntry) {
  permTarget.value = row
  permTargets.value = [row]
  const mode = typeof row.mode === 'number' ? row.mode : fromOct(row.permissions)
  modeToBits(mode)
  permInput.value = toPermText(mode)
  permRecursive.value = false
  permVisible.value = true
}

function showPermDialogForSelection() {
  if (!hasSelection.value) return
  const items = selectedItems.value
  permTargets.value = items
  permTarget.value = items[0]
  const mode = typeof items[0].mode === 'number' ? items[0].mode : fromOct(items[0].permissions)
  modeToBits(mode)
  permInput.value = toPermText(mode)
  permRecursive.value = false
  permVisible.value = true
}

/** 输入框变化：合法则同步勾选框（不回头改写输入框，避免打断输入） */
function onPermInput(value: string) {
  const digits = (value || '').trim().replace(/^0o/i, '')
  if (!/^[0-7]{1,4}$/.test(digits)) return
  modeToBits(parseInt(digits, 8))
}

/** 勾选框变化：回写规范化的八进制文本 */
function onPermBitsChange() {
  permInput.value = toPermText(bitsToMode())
}

async function doChmod() {
  if (permTargets.value.length === 0) return
  const digits = (permInput.value || '').trim()
  if (!/^[0-7]{1,4}$/.test(digits)) {
    ElMessage.warning(
      t('filesLocal.permInvalid', {
        n: isAdmin.value ? 4 : 3,
        example: permInputPlaceholder.value,
      }),
    )
    return
  }
  // 非 admin 仅提交 rwx 位，特殊位沿用文件当前值（改动特殊位会被后端拒绝）
  const mode = (parseInt(digits, 8) & permMask.value) | permKeepSpecial.value
  permSaving.value = true
  try {
    for (const target of permTargets.value) {
      const res = await chmodFile(target.path, mode, permRecursive.value)
      const info = res.data
      if (info) {
        target.permissions = info.permissions
        target.mode = info.mode
      }
    }
    ElMessage.success(
      permRecursive.value ? t('filesLocal.permOkRecursive') : t('filesLocal.permOk'),
    )
    permVisible.value = false
    loadFileList()
  } catch {
    // handled by interceptor
  } finally {
    permSaving.value = false
  }
}

// ── 属主 / 属组修改（仅 admin）─────────────────────────────

const ownVisible = ref(false)
const ownTarget = ref<FileEntry | null>(null)
const ownTargets = ref<FileEntry[]>([])
const ownUser = ref('')
const ownGroup = ref('')
const ownSaving = ref(false)
/** 是否递归修改目录下所有子项的属主/属组 */
const ownRecursive = ref(false)

function showOwnerDialog(row: FileEntry) {
  if (!isAdmin.value) return
  ownTarget.value = row
  ownTargets.value = [row]
  ownUser.value = row.owner || ''
  ownGroup.value = row.group || ''
  ownRecursive.value = false
  ownVisible.value = true
}

function showOwnerDialogForSelection() {
  if (!isAdmin.value || !hasSelection.value) return
  const items = selectedItems.value
  ownTargets.value = items
  ownTarget.value = items[0]
  ownUser.value = items[0].owner || ''
  ownGroup.value = items[0].group || ''
  ownRecursive.value = false
  ownVisible.value = true
}

async function doChown() {
  if (ownTargets.value.length === 0) return
  const owner = ownUser.value.trim() || null
  const group = ownGroup.value.trim() || null
  if (!owner && !group) {
    ElMessage.warning(t('filesLocal.needOwnerOrGroup'))
    return
  }
  ownSaving.value = true
  try {
    let changed = 0
    for (const target of ownTargets.value) {
      // 非递归时只提交真正变化的字段；递归时原样下发，把改动传播到所有子项
      const nextOwner = ownRecursive.value ? owner : owner && owner !== target.owner ? owner : null
      const nextGroup = ownRecursive.value ? group : group && group !== target.group ? group : null
      if (!nextOwner && !nextGroup) continue
      const res = await chownFile(target.path, nextOwner, nextGroup, ownRecursive.value)
      const info = res.data
      if (info) {
        target.owner = info.owner
        target.group = info.group
      }
      changed++
    }
    if (changed === 0) {
      ElMessage.info(t('filesLocal.ownerNoChange'))
      return
    }
    ElMessage.success(
      ownRecursive.value ? t('filesLocal.ownerOkRecursive') : t('filesLocal.ownerOk'),
    )
    ownVisible.value = false
    loadFileList()
  } catch {
    // handled by interceptor
  } finally {
    ownSaving.value = false
  }
}

// ── lifecycle ──────────────────────────────────────────────

onMounted(async () => {
  // 先列一次目录：不带 path，由后端落到家目录，同时把 home 带回来做侧栏根节点
  await loadFileList()
  treeData.value = buildTreeData()
  await revealInTree()
  window.addEventListener('keydown', onEditKeydown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onEditKeydown)
})

// 切回列表视图时，把当前选中态同步到 el-table 的复选框
watch(viewMode, async (mode) => {
  if (mode === 'list') {
    await nextTick()
    syncTableSelection()
  }
})
</script>

<style scoped lang="scss">
.file-manager {
  display: flex;
  /* 高度由外层（本地/云存储 标签页容器）给定 */
  height: 100%;
  background: var(--el-bg-color);
  border-radius: 4px;
  overflow: hidden;
}

.fm-sidebar {
  width: 240px;
  min-width: 200px;
  border-right: 1px solid var(--el-border-color-lighter);
  display: flex;
  flex-direction: column;
  /*background: var(--el-bg-color-page);*/

  &-header {
    padding: 12px 16px;
    font-weight: 600;
    font-size: 14px;
    border-bottom: 1px solid var(--el-border-color-lighter);
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
}

.fm-tree-scroll {
  flex: 1;
  padding: 8px;
}

.fm-tree-node {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
}

.fm-tree-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fm-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.fm-toolbar {
  padding: 8px 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;

  &-left {
    display: flex;
    align-items: center;
  }

  &-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }
}

/* 选中项操作条：地址栏下方独立一行 */
.fm-selection-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 16px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  background: var(--el-fill-color-lighter);
  overflow-x: auto;
}

.fm-selection-actions {
  flex: none;
}

/* 打包窗口里的「压缩包名称」一行（挂在 DirPicker 的 extra 插槽下） */
.fm-archive-name {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 10px;
}

.fm-archive-label {
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.fm-archive-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.view-toggle {
  .el-button {
    padding: 5px 10px;
  }
}

/* 列表区：拖拽上传遮罩的定位参照 */
.fm-list-area {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.fm-dropzone {
  position: absolute;
  inset: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 2px dashed var(--el-color-primary);
  background: var(--el-color-primary-light-9);
}

.fm-dropzone-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  color: var(--el-color-primary);
}

.fm-dropzone-text {
  font-size: 14px;
}

/* 上传进度面板：浮在列表区右下角（z-index 高于拖拽遮罩，避免被盖住） */
.fm-upload-panel {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  z-index: 20;
  display: flex;
  flex-direction: column;
  width: 420px;
  max-width: calc(100% - 32px);
  max-height: calc(100% - 32px);
  padding: 12px 14px;
  background: var(--el-bg-color-overlay);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
  box-shadow: var(--el-box-shadow);
}

.fm-upload-head {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
  font-size: 13px;
  cursor: pointer;
  user-select: none;
}

.fm-upload-title {
  font-weight: 600;
}

.fm-upload-count {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.fm-upload-toggle {
  margin-left: auto;
  color: var(--el-text-color-secondary);
  transition: transform 0.2s;
}

.fm-upload-toggle.expanded {
  transform: rotate(180deg);
}

.fm-upload-close {
  color: var(--el-text-color-secondary);
  cursor: pointer;
}

.fm-upload-close:hover {
  color: var(--el-color-primary);
}

.fm-upload-body {
  min-height: 0;
  margin-top: 8px;
  overflow-y: auto;
}

.fm-upload-summary {
  display: flex;
  gap: 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.fm-upload-summary .failed {
  color: var(--el-color-danger);
}

.fm-upload-failed {
  margin-top: 8px;
}

.fm-upload-failed-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--el-color-danger);
}

.fm-upload-list {
  max-height: 200px;
  margin-top: 6px;
  overflow-y: auto;
}

.fm-upload-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 0;
  font-size: 12px;
}

.fm-upload-item-icon {
  flex: none;
}

.fm-upload-item-icon.uploading {
  color: var(--el-color-primary);
  animation: fm-upload-spin 1s linear infinite;
}

.fm-upload-item-icon.done {
  color: var(--el-color-success);
}

.fm-upload-item-icon.failed {
  color: var(--el-color-danger);
}

.fm-upload-item-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fm-upload-item-size {
  flex: none;
  color: var(--el-text-color-secondary);
}

.fm-upload-item-status {
  flex: none;
  min-width: 40px;
  text-align: right;
  color: var(--el-text-color-secondary);
}

.fm-upload-item-status.uploading {
  color: var(--el-color-primary);
}

.fm-upload-item-status.done {
  color: var(--el-color-success);
}

.fm-upload-item-status.failed {
  color: var(--el-color-danger);
}

@keyframes fm-upload-spin {
  to {
    transform: rotate(360deg);
  }
}

.fm-table-wrap {
  flex: 1;
  overflow: auto;
}

.fm-file-name {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}

.fm-grid-wrap {
  flex: 1;
  overflow: hidden;

  :deep(.el-scrollbar__view) {
    padding: 16px;
  }
}

.fm-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 12px;
}

.fm-grid-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 16px 8px;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.2s;
  text-align: center;

  &:hover {
    background: var(--el-fill-color-light);
  }
}

.fm-grid-name {
  margin-top: 8px;
  font-size: 12px;
  word-break: break-all;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  max-width: 100%;
}

.fm-grid-size {
  font-size: 11px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}

.fm-grid-empty {
  grid-column: 1 / -1;
  text-align: center;
  padding: 40px;
  color: var(--el-text-color-secondary);
}

.text-muted {
  color: var(--el-text-color-placeholder);
}

.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
  font-size: 12px;
}

.fm-selection-label {
  font-size: 13px;
  color: var(--el-text-color-secondary);
  white-space: nowrap;
}

:deep(.selected-row) {
  background-color: var(--el-color-primary-light-9) !important;
}

.fm-grid-item {
  position: relative;

  &.selected {
    background: var(--el-color-primary-light-9);
  }
}

.fm-grid-check {
  position: absolute;
  top: 6px;
  left: 6px;
}

.fm-context-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1998;
}

.fm-context-menu {
  position: fixed;
  z-index: 1999;
  min-width: 160px;
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
  cursor: pointer;
  color: var(--el-text-color-primary);

  &:hover:not(.disabled) {
    background: var(--el-fill-color-light);
  }

  &.disabled {
    color: var(--el-text-color-disabled);
    cursor: not-allowed;
  }

  &.danger:not(.disabled) {
    color: var(--el-color-danger);
  }
}

.fm-context-divider {
  height: 1px;
  background: var(--el-border-color-lighter);
  margin: 6px 0;
}

.fm-perm-count {
  margin-left: 8px;
  color: var(--el-text-color-secondary);
  font-size: 13px;
}

/* ── 地址栏（面包屑）──────────────────────────────────────── */
.fm-crumbs {
  /* 层级用 > 分隔；每段单独截断，路径再长也不会把工具栏挤变形 */
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
  }

  :deep(.el-breadcrumb__separator) {
    margin: 0 6px;
    font-weight: 400;
    color: var(--el-text-color-placeholder);
  }
}

/* "根"段只有图标：别被基线挤偏，也别撑出多余宽度 */
.fm-crumb-icon {
  vertical-align: middle;
}

:deep(.el-breadcrumb__item .is-last) {
  color: var(--el-text-color-primary);
  font-weight: 500;
  cursor: default;
}

.fm-editor {
  height: min(62vh, 620px);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  overflow: hidden;
}

// 编辑器下方的快捷键提示
.fm-editor-tip {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

// ── 修改权限（cPanel 风格）──────────────────────────────────

.fm-perm-btn {
  height: auto;
  padding: 0;
  font-size: 12px;
  vertical-align: baseline;
}

.fm-perm-target {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 10px;
  margin-bottom: 12px;
  font-size: 12px;
  word-break: break-all;
  background: var(--el-fill-color-light);
  border-radius: 4px;
}

.fm-perm-value {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;

  .fm-perm-value-label {
    font-size: 13px;
    color: var(--el-text-color-regular);
  }

  .fm-perm-input {
    width: 110px;
  }

  .fm-perm-hint {
    font-size: 12px;
    color: var(--el-text-color-secondary);
  }
}

.fm-perm-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;

  th {
    padding: 0 0 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--el-text-color-secondary);
    text-align: center;
  }

  td {
    padding: 4px 0;
    text-align: center;
  }

  .fm-perm-owner {
    width: 76px;
    color: var(--el-text-color-regular);
    text-align: left;
  }

  .fm-perm-special {
    td {
      padding-top: 8px;
    }

    :deep(.el-checkbox__label) {
      padding-left: 6px;
      font-size: 12px;
    }
  }
}

.fm-perm-note {
  margin-top: 10px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--el-text-color-secondary);
}

.fm-perm-preview {
  margin-top: 12px;
  font-size: 13px;
  color: var(--el-text-color-regular);
}

.fm-perm-recursive {
  margin-top: 10px;
}
</style>
