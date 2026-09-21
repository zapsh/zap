import { http } from '@/utils/request'

/** Nginx 运行状态 */
export interface NginxStatus {
  installed?: boolean
  conf_file?: string
  conf_dir?: string
  bin?: string
  version?: string
  running?: boolean
  pid?: number | null
  /** 是否存在 systemd unit nginx */
  systemd?: boolean
  systemd_active?: boolean
  /** 默认站点（面板托管）配置文件路径 */
  default_conf?: string
  /** 默认站点当前是否开启 IP 访问（true = 欢迎页，false = 断开 444） */
  default_ip_access?: boolean
  /** 默认站点欢迎页文件（开启时可编辑该文件定制内容） */
  default_page?: string
}

/** 可编辑配置文件条目 */
export interface NginxConfFile {
  path: string
  /** 相对 conf 目录路径 */
  rel: string
  name: string
  is_main: boolean
  size: number
  mtime: number
}

export interface NginxConfListData {
  installed: boolean
  conf_file: string
  conf_dir: string
  files: NginxConfFile[]
}

export interface NginxConfContent {
  path: string
  is_main: boolean
  content: string
  size: number
  mtime: number
}

export interface NginxConfSaveData {
  path: string
  backup: string
  tested: boolean
  reloaded: boolean
  reason?: string
}

export interface NginxControlData {
  action: string
  method: 'systemd' | 'binary'
  state: 'running' | 'stopped'
}

export const getNginxStatus = () =>
  http.get<{ code: number; message: string; data: NginxStatus }>('/system/nginx/status')

export const listNginxConfs = () =>
  http.get<{ code: number; message: string; data: NginxConfListData }>('/system/nginx/config')

export const readNginxConf = (path: string) =>
  http.get<{ code: number; message: string; data: NginxConfContent }>('/system/nginx/config/content', {
    params: { path },
  })

export const saveNginxConf = (path: string, content: string) =>
  http.post<{ code: number; message: string; data: NginxConfSaveData }>('/system/nginx/config/save', {
    path,
    content,
  })

export const controlNginx = (action: 'start' | 'stop' | 'restart' | 'reload') =>
  http.post<{ code: number; message: string; data: NginxControlData }>('/system/nginx/control', { action })

export interface NginxDefaultVhostData {
  enable: boolean
  reloaded: boolean
  reason?: string
  default_conf: string
  default_page: string
}

/** 设置默认站点（IP / 未匹配域名兜底）：enable=true 展示欢迎页，false 直接断开 */
export const setNginxDefaultVhost = (enable: boolean) =>
  http.post<{ code: number; message: string; data: NginxDefaultVhostData }>(
    '/system/nginx/default-vhost',
    { enable },
  )

/** stub_status 状态页采集指标 */
export interface NginxStubMetrics {
  active: number
  accepts: number
  handled: number
  requests: number
  reading: number
  writing: number
  waiting: number
}

export interface NginxStubStatus {
  enabled?: boolean
  /** 状态页所在端口（默认站点的 HTTP 端口，通常 80） */
  port?: number | null
  /** 状态页路径，如 /nginx_status */
  path?: string
  running?: boolean
  metrics?: NginxStubMetrics | null
  worker_processes?: string
  worker_connections?: string
  processes?: { master: number; workers: number; cache: number; total: number }
}

export interface NginxStubSetData {
  enable: boolean
  port?: number | null
  reloaded: boolean
  reason?: string
}

/** 查询 Nginx 状态页 stub_status 并采集指标 */
export const getNginxStubStatus = () =>
  http.get<{ code: number; message: string; data: NginxStubStatus }>('/system/nginx/stub-status')

/** 开启 / 关闭 Nginx 状态页 */
export const setNginxStubStatus = (enable: boolean) =>
  http.post<{ code: number; message: string; data: NginxStubSetData }>('/system/nginx/stub-status', {
    enable,
  })
