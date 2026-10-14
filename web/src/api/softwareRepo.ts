// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

/** 发行版识别结果（family: debian | rhel，空 = 不支持在线换源） */
export interface DistroInfo {
  id: string
  version: string
  prettyName: string
  family: string
  suite: string
}

/** 一个源文件（kind: main = 发行版主源，apt / rpm = 目录里的源片段） */
export interface RepoFile {
  path: string
  name: string
  kind: 'main' | 'apt' | 'rpm'
  size: number
  mtime: number
  enabled: boolean
}

export interface MirrorSite {
  id: string
  label: string
  region: 'cn' | 'intl'
  url: string
}

/** 第三方源预设（内容已按当前发行版生成） */
export interface RepoPreset {
  id: string
  label: string
  desc: string
  family: string
  file: string
  content: string
}

export interface RepoStatus {
  distro: DistroInfo
  manager: string
  files: RepoFile[]
}

export interface RepoCatalog {
  distro: DistroInfo
  /** 当前发行版是否支持在线换源（repo 目录结构是否与 Rocky 系一致） */
  switchable: boolean
  mirrors: MirrorSite[]
  presets: RepoPreset[]
}

export interface RepoBackup {
  name: string
  size: number
  mtime: number
}

/** 换源预览的一条改动 */
export interface SwitchChange {
  path: string
  content: string
}

const LONG_TIMEOUT = { timeout: 10 * 60 * 1000 }

/** 系统识别 + 源文件清单（系统管理 → 软件源，仅 admin） */
export function getRepoStatus() {
  return http.get<ApiResponse<RepoStatus>>('/system/repo/status')
}

/** 镜像站目录 + 第三方源预设 */
export function getRepoCatalog() {
  return http.get<ApiResponse<RepoCatalog>>('/system/repo/catalog')
}

/** 换源预览：返回每个文件将要写入的内容（写入前先看） */
export function previewSwitch(mirrorId: string, includeEpel: boolean) {
  return http.post<ApiResponse<{ changes: SwitchChange[] }>>('/system/repo/preview', {
    mirror_id: mirrorId,
    include_epel: includeEpel,
  })
}

/** 执行换源（逐文件自动备份 + 原子写入，可选顺带更新缓存） */
export function applySwitch(mirrorId: string, includeEpel: boolean, updateCache: boolean) {
  return http.post<ApiResponse<{ written: string[]; cacheUpdated: boolean }>>(
    '/system/repo/apply',
    { mirror_id: mirrorId, include_epel: includeEpel, update_cache: updateCache },
    LONG_TIMEOUT,
  )
}

/** 读一个源文件 */
export function getRepoFile(path: string) {
  return http.get<ApiResponse<{ path: string; content: string }>>('/system/repo/file', {
    params: { path },
  })
}

/** 保存源文件（手动编辑 / 添加第三方源共用） */
export function saveRepoFile(path: string, content: string) {
  return http.post<ApiResponse<{ path: string; backup: string | null }>>('/system/repo/file', {
    path,
    content,
  })
}

/** 删除第三方源文件（删除前自动备份） */
export function deleteRepoFile(path: string) {
  return http.post<ApiResponse<{ path: string }>>('/system/repo/delete', { path })
}

/** 备份列表（「最初的源」在这里还原） */
export function getRepoBackups() {
  return http.get<ApiResponse<{ dir: string; items: RepoBackup[] }>>('/system/repo/backups')
}

/** 预览一个备份文件的内容 */
export function getRepoBackup(name: string) {
  return http.get<ApiResponse<{ name: string; content: string }>>('/system/repo/backup', {
    params: { name },
  })
}

/** 从备份还原 */
export function restoreRepoBackup(backup: string, path: string) {
  return http.post<ApiResponse<{ path: string }>>('/system/repo/restore', { backup, path })
}

/** 更新包管理器缓存（apt-get update / dnf makecache） */
export function updateRepoCache() {
  return http.post<ApiResponse<{ manager: string; output: string }>>(
    '/system/repo/cache',
    {},
    LONG_TIMEOUT,
  )
}
