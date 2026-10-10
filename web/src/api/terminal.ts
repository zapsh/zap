// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

// ── 连接类型 ───────────────────────────────────────────────

export interface SshConnection {
  id: number
  /** 归属用户 id（0 = 系统级/历史连接，仅管理员可见） */
  owner_id?: number
  /** 归属用户名 */
  owner_name?: string
  name: string
  host: string
  port: number
  username: string
  auth_type: 'password' | 'key'
  /** 跳板机连接 id：0 = 直连，否则经该连接做 TCP 转发（ProxyJump） */
  jump_conn_id: number
  password: string
  /** 是否已保存密码；false 且 auth_type=password 时连接需弹窗临时输入 */
  has_password: boolean
  ssh_key_name: string
  remark: string
  status: number
  sort_order: number
  created_at: number
  updated_at: number
}

export interface CreateConnectionPayload {
  name: string
  host: string
  port?: number
  username?: string
  auth_type?: string
  password?: string
  ssh_key_name?: string
  /** 跳板机连接 id（0 = 直连） */
  jump_conn_id?: number
  remark?: string
}

export interface UpdateConnectionPayload {
  name?: string
  host?: string
  port?: number
  username?: string
  auth_type?: string
  password?: string
  ssh_key_name?: string
  /** 跳板机连接 id（0 = 改为直连） */
  jump_conn_id?: number
  remark?: string
  status?: number
  sort_order?: number
}

// ── API ────────────────────────────────────────────────────

/** 获取所有连接列表 */
export function getConnections() {
  return http.get<ApiResponse<SshConnection[]>>('/terminal/connections')
}

/** 获取单个连接详情 */
export function getConnection(id: number) {
  return http.get<ApiResponse<SshConnection>>(`/terminal/connections/${id}`)
}

/** 创建新的 SSH 连接 */
export function createConnection(data: CreateConnectionPayload) {
  return http.post<ApiResponse>('/terminal/connections/create', data)
}

/** 更新 SSH 连接 */
export function updateConnection(id: number, data: UpdateConnectionPayload) {
  return http.post<ApiResponse>(`/terminal/connections/${id}/update`, data)
}

/** 删除 SSH 连接 */
export function deleteConnection(id: number) {
  return http.post<ApiResponse>(`/terminal/connections/${id}/delete`)
}

/** 测试连接 */
export function testConnection(id: number) {
  return http.get<ApiResponse<{ success: boolean; message: string }>>('/terminal/connections/test', {
    params: { id },
  })
}

/** 推送公钥到远程主机（需要远程密码做一次性认证） */
export function pushKeyToHost(id: number, password: string) {
  return http.post<ApiResponse>(`/terminal/connections/${id}/push-key`, { password })
}

/** 表单直推公钥（连接无需先保存，添加/编辑对话框内使用） */
export function pushKeyDirect(data: {
  host: string
  port: number
  username: string
  ssh_key_name: string
  password: string
}) {
  return http.post<ApiResponse>('/terminal/push-key', data)
}

// ── 我的 SSH 密钥（存自己的家目录 ~/.ssh）────────────────────

export interface UserSshKey {
  name: string
  /** 恒为 user：密钥保存在本人家目录 ~/.ssh（zap_ 前缀） */
  scope: 'user'
  comment: string
  fingerprint: string
  created_at: number
}

/** 我的密钥列表响应：items + 兼容字段（运行模式固定为独立系统用户） */
export interface SshKeysPayload {
  items: UserSshKey[]
}

/** 我的密钥列表（admin 额外含系统级密钥，保持历史连接可选） */
export function getUserSshKeys() {
  return http.get<ApiResponse<SshKeysPayload>>('/terminal/keys')
}

/** 生成新密钥并保存到自己的家目录 ~/.ssh */
export function generateUserKey(data: {
  name: string
  key_type?: string
  bits?: number
  comment?: string
}) {
  return http.post<ApiResponse>('/terminal/keys/generate', data)
}

/** 导入私钥到自己的家目录 ~/.ssh */
export function importUserKey(data: {
  name: string
  private_key: string
  public_key?: string
  comment?: string
}) {
  return http.post<ApiResponse>('/terminal/keys/import', data)
}

/** 删除自己的密钥（家目录文件一并清理） */
export function deleteUserKey(name: string) {
  return http.post<ApiResponse>('/terminal/keys/delete', { name })
}

/** 查看自己的公钥内容 */
export function getUserKeyPublic(name: string) {
  return http.get<ApiResponse<{ public_key: string; comment: string; fingerprint: string }>>(
    '/terminal/keys/public',
    { params: { name } },
  )
}

/** 查看自己的私钥内容（仅本人可读取家目录文件） */
export function getUserKeyPrivate(name: string) {
  return http.get<ApiResponse<{ name: string; private_key: string }>>('/terminal/keys/private', {
    params: { name },
  })
}

// ── SFTP 文件管理（复用终端连接）──────────────────────────────

export interface SftpEntry {
  name: string
  /** 完整路径（可直接用于下载 / 删除 / 重命名） */
  path: string
  is_dir: boolean
  size: number
  /** Unix 时间戳（秒） */
  mtime: number
  /** 原始权限位（含文件类型位） */
  mode: number
}

export interface SftpListPayload {
  /** 解析后的当前绝对路径 */
  cwd: string
  items: SftpEntry[]
}

/** 列出远端目录（path 为空 = 远端登录用户的家目录） */
export function sftpList(id: number, path: string) {
  return http.get<ApiResponse<SftpListPayload>>('/terminal/sftp/list', {
    params: { id, path },
  })
}

/** 下载远端文件（返回 base64 内容） */
export function sftpDownload(id: number, path: string) {
  return http.get<ApiResponse<{ name: string; content: string }>>('/terminal/sftp/download', {
    params: { id, path },
  })
}

/** 上传文件到远端目录（内容 base64） */
export function sftpUpload(id: number, path: string, name: string, content: string) {
  return http.post<ApiResponse<{ path: string }>>('/terminal/sftp/upload', {
    id,
    path,
    name,
    content,
  })
}

/** 新建目录 */
export function sftpMkdir(id: number, path: string) {
  return http.post<ApiResponse>('/terminal/sftp/mkdir', { id, path })
}

/** 删除文件或目录 */
export function sftpRemove(id: number, path: string, isDir: boolean) {
  return http.post<ApiResponse>('/terminal/sftp/remove', { id, path, is_dir: isDir })
}

/** 重命名 / 移动 */
export function sftpRename(id: number, oldPath: string, newPath: string) {
  return http.post<ApiResponse>('/terminal/sftp/rename', {
    id,
    old_path: oldPath,
    new_path: newPath,
  })
}

// ── 会话录制与回放 ──────────────────────────────────────────

export interface SessionRecording {
  id: number
  conn_id: number
  /** 连接已被删除时为空 */
  conn_name: string
  conn_host: string
  /** Unix 时间戳（秒） */
  started_at: number
  duration_ms: number
  size_bytes: number
}

/** 录制列表（conn_id 缺省 = 全部连接） */
export function getRecordings(connId?: number) {
  return http.get<ApiResponse<SessionRecording[]>>('/terminal/recordings', {
    params: connId ? { conn_id: connId } : {},
  })
}

/** 取回一条录制的 asciinema cast 文本（只录了远端输出，不含键盘输入） */
export function getRecording(id: number) {
  return http.get<ApiResponse<{ id: number; content: string }>>(`/terminal/recordings/${id}`)
}

/** 删除一条录制（文件 + 记录） */
export function deleteRecording(id: number) {
  return http.post<ApiResponse>(`/terminal/recordings/${id}/delete`)
}
