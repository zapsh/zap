// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'

export interface CronJob {
  id: string
  name: string
  script_path: string
  schedule: string
  remark: string
  enabled: boolean
  last_run_at: number
  last_run_id: string
  next_run_at: number
  created_at: number
  updated_at: number
}

export interface CronJobPayload {
  name: string
  script_path: string
  schedule: string
  remark?: string
}

export function listCronJobs() {
  return http.get<{ code: number; message: string; data: { jobs: CronJob[] } }>('/system/cron/list')
}

export function addCronJob(data: CronJobPayload) {
  return http.post<{ code: number; message: string; data: { id: string } }>('/system/cron/add', data)
}

export function updateCronJob(data: CronJobPayload & { id: string; enabled: boolean }) {
  return http.post('/system/cron/update', data)
}

export function deleteCronJob(id: string) {
  return http.post('/system/cron/delete', { id })
}

export function toggleCronJob(id: string, enabled: boolean) {
  return http.post('/system/cron/toggle', { id, enabled })
}

export function runCronJobNow(id: string) {
  return http.post<{ code: number; message: string; data: { task_id: string } }>('/system/cron/run_now', { id })
}

/** 一次运行的记录（对应后端 appstore_runs 表） */
export interface CronRunItem {
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
export function listCronRuns(jobId: string) {
  return http.get<{
    code: number
    message: string
    data: { runs: CronRunItem[]; keep: number }
  }>('/system/cron/runs', { params: { job_id: jobId } })
}

/** 清空某任务的运行历史（含日志文件与运行快照） */
export function clearCronRuns(jobId: string) {
  return http.post<{ code: number; message: string; data: { deleted: number } }>(
    '/system/cron/runs_clear',
    { id: jobId },
  )
}
