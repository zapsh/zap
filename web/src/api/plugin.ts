import type { AxiosRequestConfig } from 'axios'
import { http } from '@/utils/request'
import { getToken } from '@/utils/auth'

export interface PluginOption {
  name: string
  label: string
  type: 'string' | 'number' | 'bool' | 'select' | 'multiselect' | 'dir' | 'file' | 'files'
  default?: string
  required?: boolean
  placeholder?: string
  desc?: string
  choices?: { label: string; value: string }[] | string[]
}

export interface PluginAction {
  [key: string]: string
}

export interface PluginInfo {
  name: string
  title: string
  scope: string
  placement: string
  label: string
  icon: string
  tab: string
  /** 是否异步执行（日志以 SSE 流回传，而非一次性返回） */
  async?: boolean
  options: PluginOption[]
  actions: PluginAction
}

export function pluginList(params: { slot?: string; scope?: string; site_id?: number }) {
  return http.get('/plugin/list', { params })
}

export function pluginRun(
  payload: {
    name: string
    action?: string
    site_id?: number
    options?: Record<string, string>
  },
  config?: AxiosRequestConfig,
) {
  // composer 等联网任务可能耗时数分钟，需放宽前端超时（后端 TimeoutLayer 为 1800s）。
  return http.post('/plugin/run', payload, { timeout: 600000, ...config })
}

/** 取消正在运行的异步插件（task_id 来自 plugin/run 的返回）。 */
export function pluginCancel(task_id: string) {
  return http.post('/plugin/cancel', { token: getToken(), task_id })
}
