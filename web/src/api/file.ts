// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

// ── types ──────────────────────────────────────────────────

export interface FileEntry {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: string
  /** 权限文本：八进制 4 位（如 0755，含 setuid/setgid/sticky 时为 4755） */
  permissions: string
  /** 权限原始数值（低 12 位），用于「修改权限」对话框回填（如 0o755 = 493） */
  mode: number
  owner?: string
  group?: string
  /** 是否为符号链接（软链） */
  is_symlink?: boolean
  /** 软链指向的目标路径；非软链为 undefined */
  link_target?: string
  /** 软链是否悬空（目标不存在）；仅 is_symlink 时可能为真 */
  broken?: boolean
}

export interface FileListData {
  current_path: string
  parent_path: string
  entries: FileEntry[]
  /** 当前用户的家目录（如 `/home/admin`），文件管理以此作为起点与侧栏根节点 */
  home?: string
}

export interface FileReadData {
  path: string
  content: string
  size: number
}

// ── API ────────────────────────────────────────────────────

/**
 * List directory contents
 *
 * `path` 传空串（默认）表示"由后端决定"：落到当前用户的家目录（管理员同样如此），
 * 文件管理首次打开就是这个语义，保证始终从家目录开始。
 * 响应里的 `current_path` 是服务端确认后的实际目录，`home` 即家目录。
 */
export function listFiles(path: string = '') {
  return http.get<ApiResponse<FileListData>>('/system/files/list', {
    params: { path },
  })
}

/** Read file content */
export function readFile(path: string) {
  return http.get<ApiResponse<FileReadData>>('/system/files/read', {
    params: { path },
  })
}

/** Write / create file */
export function writeFile(path: string, content: string) {
  return http.post<ApiResponse<{ path: string }>>('/system/files/write', { path, content })
}

/** Delete file or directory (moved to the recycle bin, recoverable — see #42) */
export function deleteFile(path: string) {
  return http.post<ApiResponse>('/system/files/trash', { path })
}

// ── 回收站（文件删除改为移入 trash，可恢复 / 彻底删除）────────────

export interface TrashItem {
  /** 回收站条目 id（用于恢复 / 彻底删除） */
  id: string
  /** 原始路径 */
  original: string
  /** 原文件名（original 的最后一段） */
  name: string
  /** 删除时是否为目录 */
  is_dir: boolean
  /** 删除时间（ISO / 时间戳） */
  deleted_at: string
}

/** 移入回收站（单文件 / 目录） */
export function trashFile(path: string) {
  return http.post<ApiResponse>('/system/files/trash', { path })
}

/** 列出当前账号的回收站 */
export function listTrash() {
  return http.get<ApiResponse<{ list: TrashItem[] }>>('/system/files/trash')
}

/** 从回收站恢复到原路径 */
export function restoreTrash(id: string) {
  return http.post<ApiResponse>('/system/files/restore', { id })
}

/** 从回收站彻底删除（不可恢复） */
export function purgeTrash(id: string) {
  return http.post<ApiResponse>('/system/files/purge', { id })
}

/** Create directory */
export function mkdir(path: string) {
  return http.post<ApiResponse<{ path: string }>>('/system/files/mkdir', { path })
}

/** Rename / move file */
export function renameFile(path: string, newPath: string) {
  return http.post<ApiResponse<{ old_path: string; new_path: string }>>('/system/files/rename', {
    path,
    new_path: newPath,
    content: '',
  })
}

/** Change permissions（mode 为八进制数值，如 0755 传 493） */
export function chmodFile(path: string, mode: number, recursive = false) {
  return http.post<ApiResponse<FileEntry>>('/system/files/chmod', { path, mode, recursive })
}

/** Change owner / group（仅 admin）。owner/group 为 Linux 名称，null 表示保持不变 */
export function chownFile(
  path: string,
  owner: string | null,
  group: string | null,
  recursive = false,
) {
  return http.post<ApiResponse<FileEntry>>('/system/files/chown', {
    path,
    owner,
    group,
    recursive,
  })
}

/**
 * Upload file(s) to `targetDir`.
 *
 * 目录上传（`webkitdirectory`）时浏览器把相对路径挂在 `file.webkitRelativePath`
 * （如 `dir/sub/a.txt`），这里用它作为 multipart 的文件名，由后端逐级创建目录还原结构。
 *
 * `onProgress` 是整包进度，一次传多个文件时只能得到合计百分比，
 * 所以调用方按「一个文件一个请求」来逐个显示进度。
 */
export function uploadFiles(
  targetDir: string,
  files: File[],
  onProgress?: (percent: number) => void,
) {
  const formData = new FormData()
  for (const file of files) {
    formData.append('files', file, file.webkitRelativePath || file.name)
  }
  return http.post<ApiResponse<{ files: string[]; target_dir: string }>>(
    `/system/files/upload?path=${encodeURIComponent(targetDir)}`,
    formData,
    {
      onUploadProgress: (event) => {
        if (!onProgress || !event.total) return
        onProgress(Math.min(100, Math.round((event.loaded / event.total) * 100)))
      },
    },
  )
}

/** Download file (returns Blob) */
export function downloadFile(path: string) {
  return http.download(`/system/files/download?path=${encodeURIComponent(path)}`)
}

/** Get file/directory metadata */
export function getFileInfo(path: string) {
  return http.get<ApiResponse<FileEntry>>('/system/files/info', {
    params: { path },
  })
}

/** Compute recursive directory size (returns total bytes; a plain file returns its own size) */
export function getDirSize(path: string) {
  return http.get<ApiResponse<{ path: string; size: number }>>('/system/files/dir_size', {
    params: { path },
  })
}

/** Copy file/directory to a new path */
export function copyFile(path: string, newPath: string) {
  return http.post<ApiResponse<{ path: string }>>('/system/files/copy', {
    path,
    new_path: newPath,
  })
}

export interface ArchiveData {
  name: string
  /** 打包到指定目录时返回压缩包完整路径 */
  path?: string
  /** 未指定目录（下载场景）时返回 zip 的 base64 内容 */
  content?: string
}

/**
 * Archive selected paths into a zip.
 *
 * 传 `destDir` 时后端把压缩包写进该目录并返回其路径；
 * 不传则返回 base64 内容，由调用方下载。
 */
export function archiveFiles(paths: string[], name: string, baseDir: string, destDir?: string) {
  return http.post<ApiResponse<ArchiveData>>('/system/files/archive', {
    paths,
    name,
    base_dir: baseDir,
    dest_dir: destDir,
  })
}

export interface ExtractData {
  /** 解压目标目录 */
  path: string
}

/**
 * Extract an archive (zip / tar / tar.gz / tgz) into `destDir`.
 *
 * `overwrite` 为 true 时覆盖同名文件；不支持的格式后端会返回明确错误。
 */
export function extractArchive(path: string, destDir: string, overwrite: boolean) {
  return http.post<ApiResponse<ExtractData>>('/system/files/extract', {
    path,
    dest_dir: destDir,
    overwrite,
  })
}

export interface FileSearchData {
  /** 搜索根目录（服务端确认后的实际路径） */
  root_path: string
  keyword: string
  /** 命中条数（已截断时等于 max） */
  total: number
  /** 结果是否因达到上限被截断 */
  truncated: boolean
  entries: FileEntry[]
}

/**
 * 递归搜索文件名（限当前权限目录内；keyword 支持 `*` 通配，如 `*.php`、`config*`）。
 * `path` 为搜索根（默认当前目录），后端带数量/深度上限防 DoS。
 */
export function searchFiles(path: string, keyword: string, max = 500) {
  return http.get<ApiResponse<FileSearchData>>('/system/files/search', {
    params: { path, keyword, max },
  })
}
