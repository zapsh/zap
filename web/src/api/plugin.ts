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

/** 单个动作的元信息（`actions.<name>: { label, async, dangerous }` 写法时才有） */
export interface PluginActionSpec {
  name: string
  label: string
  /** 该动作单独异步执行（日志走 SSE 流），不必把整个插件设成 async */
  async?: boolean
  /** 破坏性动作：执行前宿主会二次确认，只读演示账号在后端被直接拒绝 */
  dangerous?: boolean
}

export interface PluginInfo {
  name: string
  title: string
  scope: string
  /** 主槽位（兼容旧字段）；多槽位插件请看 placements */
  placement: string
  /** manifest 里声明的全部挂载位置，一个插件可以挂多处 */
  placements?: string[]
  label: string
  icon: string
  tab: string
  /** 是否异步执行（日志以 SSE 流回传，而非一次性返回） */
  async?: boolean
  options: PluginOption[]
  actions: PluginAction
  /** 每个动作的 async / dangerous 开关 */
  action_specs?: PluginActionSpec[]
  /** 安装级别：统一 system（管理员安装到 $ZAP_PATH/plugins，全用户共享、仅可使用） */
  level?: 'system'
  version?: string
  description?: string
  author?: string
  homepage?: string
  /** 自带 HTML 界面的文件名（manifest 的 ui.html）；空 = 用结构化表单 */
  html?: string
  /** 安装来源：archive（上传包）/ appstore（应用商店）/ git（历史仓库） */
  source?: string
  /** 安装来源详情：仓库 URL 或包名 */
  src?: string
  /** 安装时间（Unix 秒） */
  installed_at?: number
}

/** 安装来源 */
export type PluginInstallSource = 'archive' | 'appstore' | 'git'

export function pluginList(params: { slot?: string; scope?: string; site_id?: number }) {
  return http.get('/plugin/list', { params })
}

/** 取插件自带的 HTML 界面内容（渲染进沙箱 iframe）。 */
export function pluginUi(params: { name: string }) {
  return http.get('/plugin/ui', { params })
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

/**
 * 上传插件包安装（multipart）。
 *
 * 字段顺序很重要：`force` / `name` 要先 append，文件最后 append ——
 * 后端按 multipart 字段到达顺序解析，文件放最后才能保证前面的选项已被读到。
 */
export function pluginInstallUpload(payload: {
  file: File
  force?: boolean
  name?: string
}) {
  const fd = new FormData()
  fd.append('force', payload.force ? 'true' : 'false')
  if (payload.name) fd.append('name', payload.name)
  fd.append('file', payload.file)
  return http.post('/plugin/install', fd, { timeout: 600000 })
}

/** 卸载插件（删除整个插件目录，仅管理员）。 */
export function pluginUninstall(payload: { name: string }) {
  return http.post('/plugin/uninstall', payload)
}
