// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

// ── types ──────────────────────────────────────────────────

/** 一个可查看的日志源（由 zapexec 探测产出） */
export interface LogSource {
  /** `journal:<unit>` / `journal:` / `kernel:` / `file:<path>` */
  id: string
  /** 显示名 */
  name: string
  /** 分组：内核 / 系统 / Web / PHP / 数据库 / 面板 / 安全 / 服务 */
  group: string
  /** journal | file | kernel */
  kind: string
  /** 副标题：路径或 unit 名 */
  desc: string
  /** 文件大小（仅 file；其余为 0） */
  size: number
  /** 管理员手动添加的自定义路径（前端据此允许删除） */
  custom?: boolean
}

/** 手动添加的自定义日志路径 */
export interface CustomLogSource {
  name: string
  path: string
}

export interface LogSourcesData {
  sources: LogSource[]
  /** 机器上 journalctl 是否可用（不可用则只能看文件日志） */
  journal: boolean
}

export interface LogReadData {
  lines: string[]
  /** 下次增量读取的游标；为空表示不支持增量（如 dmesg），只能整段重取 */
  cursor: string
  /** 实际读取的源描述 */
  source: string
  count: number
}

// ── API ────────────────────────────────────────────────────

/** 列出可查看的日志源 */
export function getLogSources() {
  return http.get<ApiResponse<LogSourcesData>>('/system/logs/sources')
}

/**
 * 读取某个日志源。
 *
 * `cursor` 为空表示首次读取（取末尾 `lines` 行），之后带上上次返回的 cursor 只取新增内容。
 */
export function readSystemLog(params: {
  source: string
  cursor?: string
  lines?: number
  keyword?: string
}) {
  return http.get<ApiResponse<LogReadData>>('/system/logs/read', { params })
}

/** 列出手动添加的自定义日志路径 */
export function getCustomLogSources() {
  return http.get<ApiResponse<{ sources: CustomLogSource[] }>>('/system/logs/custom')
}

/** 添加自定义日志路径（name 留空则用文件名） */
export function addCustomLogSource(name: string, path: string) {
  return http.post<ApiResponse<null>>('/system/logs/custom/add', { name, path })
}

/** 删除自定义日志路径（只删登记，不删日志文件） */
export function removeCustomLogSource(path: string) {
  return http.post<ApiResponse<null>>('/system/logs/custom/remove', { path })
}
