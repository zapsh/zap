// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'
import type { FileListData } from '@/api/file'

// ── types ──────────────────────────────────────────────────

/** 云存储配置（对外视图，凭据只有脱敏提示） */
export interface CloudStore {
  id: string
  name: string
  /** aws_s3 / oss / cos / s3_compat */
  service: string
  service_label: string
  endpoint: string
  region: string
  bucket: string
  /** 桶内逻辑根目录（空 = 桶根） */
  root: string
  virtual_host_style: boolean
  /** access key 脱敏提示，如 LTAI****cdef */
  access_key_hint: string
  created_at: number
  updated_at: number
}

/** 服务预设（后端下发：默认端点与必填规则，前端不重复定义） */
export interface CloudServicePreset {
  id: string
  label: string
  endpoint_required: boolean
  endpoint_hint: string
  region_required: boolean
  virtual_host_default: boolean
}

export interface CloudEntry {
  name: string
  /** 相对逻辑根；目录以 `/` 结尾 */
  path: string
  is_dir: boolean
  size: number
  /** RFC3339（可能为空） */
  modified: string
  etag: string
}

export interface CloudListData {
  current_path: string
  parent_path: string
  entries: CloudEntry[]
  truncated: boolean
}

/** 新增 / 编辑提交体（编辑时密钥留空 = 沿用原凭据） */
export interface CloudStoreInput {
  id?: string
  name: string
  service: string
  endpoint: string
  region: string
  bucket: string
  root: string
  virtual_host_style: boolean
  access_key_id: string
  secret_access_key: string
  security_token: string
}

// ── 配置管理 ────────────────────────────────────────────────

/** 当前用户的云存储列表 + 服务预设 */
export function listCloudStores() {
  return http.get<ApiResponse<{ stores: CloudStore[]; services: CloudServicePreset[] }>>(
    '/system/cloud/stores',
  )
}

/** 新增 / 编辑云存储 */
export function saveCloudStore(data: CloudStoreInput) {
  return http.post<ApiResponse<{ store: CloudStore }>>('/system/cloud/store/save', data)
}

/** 删除云存储配置（不动桶内数据） */
export function deleteCloudStore(id: string) {
  return http.post<ApiResponse>('/system/cloud/store/delete', { id })
}

/** 连通性测试（列 5 个对象） */
export function testCloudStore(id: string) {
  return http.get<ApiResponse<{ entries: number; elapsed_ms: number }>>('/system/cloud/test', {
    params: { id },
    // 后端测试超时 15s，这里留出余量（默认 15s 会先断开，看不到真实报错）
    timeout: 30000,
  })
}

// ── 对象操作 ────────────────────────────────────────────────

/** 列目录（单层） */
export function listCloudFiles(id: string, path: string = '', hidden: boolean = false) {
  return http.get<ApiResponse<CloudListData>>('/system/cloud/list', {
    params: { id, path, hidden },
  })
}

/** 新建目录 */
export function cloudMkdir(id: string, path: string) {
  return http.post<ApiResponse>('/system/cloud/mkdir', { id, path })
}

/** 删除文件或目录（目录递归） */
export function cloudDelete(id: string, path: string) {
  return http.post<ApiResponse>('/system/cloud/delete', { id, path })
}

/** 重命名 / 移动 */
export function cloudRename(id: string, path: string, newPath: string) {
  return http.post<ApiResponse>('/system/cloud/rename', { id, path, new_path: newPath })
}

/** 传输类请求的超时：全局 15s 对上传/下载大文件太短，这里放到与后端一致的 30 分钟 */
const TRANSFER_TIMEOUT = 30 * 60 * 1000

/** 上传文件到指定目录（后端流式写入，支持大文件） */
export function cloudUpload(
  id: string,
  dirPath: string,
  files: File[],
  onProgress?: (percent: number) => void,
) {
  const formData = new FormData()
  for (const file of files) {
    formData.append('file', file)
  }
  return http.post<ApiResponse<{ files: string[] }>>(
    `/system/cloud/upload?id=${encodeURIComponent(id)}&path=${encodeURIComponent(dirPath)}`,
    formData,
    {
      timeout: TRANSFER_TIMEOUT,
      onUploadProgress: (event) => {
        if (!onProgress || !event.total) return
        onProgress(Math.min(100, Math.round((event.loaded / event.total) * 100)))
      },
    },
  )
}

/** 下载对象（返回 Blob） */
export function downloadCloudFile(id: string, path: string) {
  return http.download(
    `/system/cloud/download?id=${encodeURIComponent(id)}&path=${encodeURIComponent(path)}`,
    { timeout: TRANSFER_TIMEOUT },
  )
}

// ── 「从服务器上传」─────────────────────────────────────────
// 文件本来就在服务器上，不必经浏览器中转：由后端读盘后直接写入对象存储。

/** 浏览服务器上当前用户可访问的目录（`path` 留空 = 家目录） */
export function listLocalFiles(path: string = '') {
  return http.get<ApiResponse<FileListData>>('/system/cloud/local/list', { params: { path } })
}

/** 把服务器上的文件直接上传到云存储（`files` 为本地绝对路径） */
export function cloudUploadLocal(id: string, dirPath: string, files: string[]) {
  return http.post<ApiResponse<{ files: string[]; failed: string[] }>>(
    '/system/cloud/upload-local',
    { id, path: dirPath, files },
    { timeout: TRANSFER_TIMEOUT },
  )
}
