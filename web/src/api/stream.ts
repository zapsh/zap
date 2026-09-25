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
  /** 合并展示：`10.0.1.10:3306` 或 upstream 名 */
  target: string
  /** basic（按字段渲染）/ advanced（整段自定义配置） */
  mode: string
  /** advanced：stream 块内的自定义配置 */
  raw: string
  /** basic：single = 单后端直接转发；group = upstream 负载组 */
  backend_mode: string
  /** basic：负载组成员，一行一个 host:port[ 参数] */
  targets: string
  /** basic：listen 附加参数（reuseport / ssl / backlog=1024） */
  listen_opts: string
  proxy_connect_timeout: string
  proxy_timeout: string
  /** UDP 等无连接场景要等几个响应包；0 = 不输出 */
  proxy_responses: number
  ssl_enable: boolean
  ssl_certificate: string
  ssl_certificate_key: string
  ssl_protocols: string
  ssl_ciphers: string
  /** 证书库里的证书 id（0 = 用下面的手工路径） */
  ssl_certificate_id: number
  ssl_preread: boolean
  /** 自定义 proxy_pass 变量（如 $backend），空则用本规则后端 */
  proxy_pass: string
  /** server 块内附加指令 */
  extra: string
  remark: string
  /** 1 启用 / 0 停用 */
  status: number
  created_at: number
  updated_at: number
}

export interface StreamPayload {
  name: string
  listen_port: number
  /** 后端地址：`10.0.1.10:3306` 或 upstream 名（不带端口）；优先于下面两个字段 */
  target: string
  target_host?: string
  target_port?: number
  listen_ip?: string
  protocol?: string
  remark?: string
  status?: number
  mode?: string
  raw?: string
  backend_mode?: string
  targets?: string
  listen_opts?: string
  proxy_connect_timeout?: string
  proxy_timeout?: string
  proxy_responses?: number
  ssl_enable?: boolean
  ssl_certificate?: string
  ssl_certificate_key?: string
  ssl_protocols?: string
  ssl_ciphers?: string
  ssl_certificate_id?: number
  ssl_preread?: boolean
  proxy_pass?: string
  extra?: string
}

export interface StreamStatus {
  /** 是否探测到 Nginx */
  installed: boolean
  /** 是否带 stream 模块（--with-stream） */
  supported: boolean
  /** 主配置是否已 include zap-stream.conf */
  included: boolean
  /** zap-stream.conf 的绝对路径 */
  file: string
  /** nginx 主配置绝对路径（探测结果，便于排查） */
  conf: string
  /** nginx 可执行文件 */
  bin: string
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

/** 全局自定义片段（stream 块顶部指令：resolver / map / 公共 upstream） */
export const getStreamGlobal = () =>
  http.get<{ code: number; message: string; data: { content: string } }>('/system/stream/global')

export const saveStreamGlobal = (content: string) =>
  http.post('/system/stream/global/save', { content })

/** 证书库里可选的证书（stream 监听侧 TLS 用） */
export interface StreamCertOption {
  id: number
  name: string
  domains: string
  not_after: number
}

export const getStreamCerts = () =>
  http.get<{ code: number; message: string; data: { items: StreamCertOption[] } }>(
    '/system/stream/certs',
  )
