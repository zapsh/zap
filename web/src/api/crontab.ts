// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'

export interface CronJob {
  id: string
  name: string
  schedule: string
  /** script：command 为脚本绝对路径；command：command 为 sh 命令体 */
  kind: 'script' | 'command'
  command: string
  /** 实际执行的 Linux 账号（非 admin 恒为本人账号） */
  exec_user: string
  enabled: boolean
  remark: string
  last_run_at: number
  last_run_id: string
  /** running | success | failed | '' */
  last_status: string
  next_run_at: number
  created_at: number
  updated_at: number
}

export interface CronListData {
  username: string
  /** 当前用户是否为 admin（决定「执行用户」是否可选） */
  can_choose_exec: boolean
  /** 当前用户自身的 Linux 账号 */
  exec_user: string
  jobs: CronJob[]
}

export interface CronJobPayload {
  name: string
  schedule: string
  kind: 'script' | 'command'
  command: string
  exec_user?: string
  remark?: string
}

export function listCrontab(username?: string) {
  return http.get<{ code: number; message: string; data: CronListData }>('/terminal/crontab/list', {
    params: username ? { username } : undefined,
  })
}

export function addCrontab(data: CronJobPayload) {
  return http.post<{ code: number; message: string; data: { id: string } }>('/terminal/crontab/add', data)
}

export function updateCrontab(data: CronJobPayload & { id: string; enabled: boolean }) {
  return http.post('/terminal/crontab/update', data)
}

export function deleteCrontab(id: string) {
  return http.post('/terminal/crontab/delete', { id })
}

export function toggleCrontab(id: string, enabled: boolean) {
  return http.post('/terminal/crontab/toggle', { id, enabled })
}

export function runCrontabNow(id: string) {
  return http.post<{ code: number; message: string; data: { task_id: string } }>(
    '/terminal/crontab/run_now',
    { id },
  )
}

export function readCrontabLog(run_id: string, username?: string) {
  return http.get<{
    code: number
    message: string
    data: { log: string; done: boolean; exit_code: number | null }
  }>('/terminal/crontab/log', { params: { run_id, ...(username ? { username } : {}) } })
}

/** 一次运行的记录（对应后端 appstore_runs 表） */
export interface CrontabRunItem {
  id: number
  task_id: string
  action: string
  pkg: string
  username: string
  status: string
  exit_code: number
  log_path: string
  job_key: string
  started_at: number
  finished_at: number
}

/** 某任务最近的运行历史（后端只保留最近 N 条，无需分页） */
export function listCrontabRuns(jobId: string) {
  return http.get<{
    code: number
    message: string
    data: { runs: CrontabRunItem[]; keep: number }
  }>('/terminal/crontab/runs', { params: { id: jobId } })
}

/** 清空某任务的运行历史（含日志文件） */
export function clearCrontabRuns(jobId: string) {
  return http.post<{ code: number; message: string; data: { deleted: number } }>(
    '/terminal/crontab/runs_clear',
    { id: jobId },
  )
}

/** 清理无归属的历史遗留日志（开始登记运行记录之前留下的 run-*.log） */
export function purgeCrontabLogs() {
  return http.post<{ code: number; message: string; data: { deleted: number } }>(
    '/terminal/crontab/logs_purge',
  )
}

/** 可供 admin 选择的执行用户列表（仅 admin 可调用） */
export function listCrontabExecUsers() {
  return http.get<{ code: number; message: string; data: { users: string[] } }>(
    '/terminal/crontab/exec-users',
  )
}
