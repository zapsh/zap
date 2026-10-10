// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'

/** 自动更新配置（对应后端 update_config 单行表） */
export interface UpdateConfig {
  auto: boolean
  cron: string
  channel: string
  last_check_at: number
  last_check_version: string
  last_check_has_update: boolean
  last_error: string
}

/** 一次系统升级运行记录（后端 appstore_runs，action=zap_update） */
export interface UpdateRunInfo {
  id?: number
  task_id: string
  action?: string
  pkg?: string
  username?: string
  status: string
  exit_code?: number | null
  log_path?: string
  started_at?: number
  finished_at?: number | null
}

export interface UpdateStatusData {
  zapd_version: string
  zapexec_version: string
  config: UpdateConfig
  upgrading: boolean
  current_run: UpdateRunInfo | null
  recent_runs: UpdateRunInfo[]
}

export interface CheckResult {
  current: string
  latest: string
  has_update: boolean
}

export interface ApplyResult {
  task_id: string
  log_path: string
  latest: string
}

export interface UpdateLogData {
  task_id: string
  log: string
  offset: number
  done: boolean
  exit_code: number | null
  status: string
}

/** 系统更新状态（版本 / 自动更新配置 / 进行中与历史） */
export function getUpdateStatus() {
  return http.get<{ code: number; message: string; data: UpdateStatusData }>(
    '/system/update/status',
  )
}

/** 检查远端是否有新版本（更新 last_check_*） */
export function checkForUpdate() {
  return http.post<{ code: number; message: string; data: CheckResult }>('/system/update/check')
}

/** 保存自动更新配置（开关 / cron / 渠道） */
export function saveUpdateConfig(payload: { auto: boolean; cron: string; channel: string }) {
  return http.post<{ code: number; message: string }>('/system/update/config', payload)
}

/** 触发一次系统升级（手动） */
export function applyUpdate() {
  return http.post<{ code: number; message: string; data: ApplyResult }>('/system/update/apply')
}

/** 升级运行日志（offset 传上次已读长度，做增量轮询） */
export function getUpdateLog(runId: string, offset = 0) {
  return http.get<{ code: number; message: string; data: UpdateLogData }>(
    `/system/update/log/${runId}`,
    { params: { offset } },
  )
}
