import { http } from '@/utils/request'

export interface PluginOption {
  name: string
  label: string
  type: 'string' | 'number' | 'bool' | 'select' | 'multiselect' | 'dir'
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
  options: PluginOption[]
  actions: PluginAction
}

export function pluginList(params: { slot?: string; scope?: string; site_id?: number }) {
  return http.get('/plugin/list', { params })
}

export function pluginRun(payload: {
  name: string
  action?: string
  site_id?: number
  options?: Record<string, string>
}) {
  return http.post('/plugin/run', payload)
}
