import { http } from '@/utils/request'
import { getToken } from '@/utils/auth'
import { wsUrl } from '@/utils/base'

/**
 * 通用任务队列：应用商店安装 / Docker 镜像构建 / 备份 / 系统升级 / 计划任务
 * 都登记在同一张 `task_queue` 表，前端统一从这里读状态、看日志、发控制指令。
 */

/** 任务状态：排队中（并发组占位）/ 进行中 / 成功 / 失败 / 已取消 */
export type TaskStatus = 'pending' | 'running' | 'success' | 'failed' | 'canceled'

/** 任务大类；后端 kind 字段（appstore / docker / backup / system / cron / crontab / site） */
export type TaskKind = 'appstore' | 'docker' | 'backup' | 'system' | 'cron' | 'crontab' | 'site'

export interface TaskItem {
  task_id: string
  /** 兼容字段：与 task_id 同值，老接口（/appstore/runs）沿用 */
  run_id: string
  kind: string
  action: string
  /** 任务对象：包名 / 镜像名 / 脚本路径 / 备份目标 */
  pkg: string
  username: string
  status: TaskStatus
  exit_code: number
  log_path: string
  /** 归属键：计划任务 / 某用户的构建历史 */
  job_key: string
  /** 并发互斥组 */
  group_key: string
  title: string
  /** 0-100；-1 表示不适用 */
  progress: number
  /** 控制指令：'' / cancel / pause */
  control: string
  started_at: number
  finished_at: number
}

export interface TaskListParams {
  page?: number
  page_size?: number
  kind?: string
  action?: string
  status?: string
  username?: string
  group_key?: string
  job_key?: string
  /** 标题 / 对象 / 任务号模糊匹配 */
  keyword?: string
}

export interface TaskStats {
  pending: number
  running: number
  success: number
  failed: number
  canceled: number
  total: number
}

export const getTasks = (params: TaskListParams = {}) => http.get<any>('/task/list', { params })

export const getTaskStats = () => http.get<any>('/task/stats')

export const getTask = (taskId: string) =>
  http.get<any>('/task/detail', { params: { task_id: taskId } })

/** 增量读日志：传上一轮返回的 offset 只取新增部分 */
export const getTaskLog = (taskId: string, offset = 0) =>
  http.get<any>(`/task/log/${taskId}`, { params: { offset } })

export const cancelTask = (taskId: string) => http.post<any>('/task/cancel', { task_id: taskId })

export const pauseTask = (taskId: string) => http.post<any>('/task/pause', { task_id: taskId })

export const resumeTask = (taskId: string) => http.post<any>('/task/resume', { task_id: taskId })

/** 任务日志实时流（token 走 query：浏览器 WebSocket 不能带自定义头） */
export function taskWsUrl(taskId: string): string {
  return wsUrl(`/api/task/ws/${taskId}?token=${getToken()}`)
}
