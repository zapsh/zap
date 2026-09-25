import { http } from '@/utils/request'

/** 四层转发规则 */
export interface StreamRule {
  id: number
  name: string
  listen_ip: string
  listen_port: number
  /** tcp / udp */
  protocol: string
  target_host: string
  target_port: number
  remark: string
  /** 1 启用 / 0 停用 */
  status: number
  created_at: number
  updated_at: number
}

export interface StreamPayload {
  name: string
  listen_port: number
  target_host: string
  target_port: number
  listen_ip?: string
  protocol?: string
  remark?: string
  status?: number
}

export interface StreamStatus {
  /** 是否探测到 Nginx */
  installed: boolean
  /** 是否带 stream 模块（--with-stream） */
  supported: boolean
  /** 主配置是否已 include zap-stream.conf */
  included: boolean
  file: string
  running: boolean
  error: string
}

export const getStreamStatus = () =>
  http.get<{ code: number; message: string; data: StreamStatus }>('/system/stream/status')

export const getStreamList = () =>
  http.get<{ code: number; message: string; data: { items: StreamRule[] } }>('/system/stream/list')

export const addStreamRule = (data: StreamPayload) => http.post('/system/stream/add', data)

export const updateStreamRule = (data: Partial<StreamPayload> & { id: number }) =>
  http.post('/system/stream/update', data)

export const deleteStreamRule = (id: number) => http.post('/system/stream/delete', { id })

/** 按库里规则重新渲染并生效 */
export const applyStream = () => http.post('/system/stream/apply')
