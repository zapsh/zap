// ModSecurity（WAF）API —— 可选能力：未安装时除 status 外一律返回错误
//
// WAF 能不能用取决于 nginx 当初的编译参数（动态模块要求 --with-compat），
// 所以前端**先问 status**：没装就只显示安装引导，不给任何设置项。
import { http } from '@/utils/request'

export interface WafStatus {
  installed: boolean
  nginx_installed: boolean
  nginx?: {
    bin: string
    conf: string
    version: string
    /** 是否以 --with-compat 编译：决定能否加装动态模块 */
    compat: boolean
    load_module: boolean
    running: boolean
  }
  libmodsecurity: boolean
  module: string
  rules_dir: string
  rules_dir_exists: boolean
  main_conf: string
  /** http 上下文启用文件（`modsecurity on;` 落在哪个文件里；空串表示未启用） */
  enabled: string
  /** 是否已部署 OWASP CRS */
  crs: boolean
  /** SecRuleEngine：On / Off / DetectionOnly */
  engine: string
  audit_log: string
  files: { path: string; rel: string; size: number }[]
  installable: boolean
  /** 不能自动安装的具体原因（逐条展示给用户） */
  blockers: string[]
  hint: string
}

export interface WafTaskResult {
  task_id: string
  run_id: string
  queued: boolean
  position?: number
}

export function getWafStatus() {
  return http.get<{ code: number; message: string; data: WafStatus }>('/system/waf/status')
}

export function installWaf() {
  return http.post<{ code: number; message: string; data: WafTaskResult }>('/system/waf/install')
}

export function getWafConfList() {
  return http.get<{
    code: number
    message: string
    data: { rules_dir: string; files: WafStatus['files'] }
  }>('/system/waf/conf/list')
}

export function getWafConfRead(path: string) {
  return http.get<{
    code: number
    message: string
    data: { path: string; content: string; size: number }
  }>('/system/waf/conf/read', { params: { path } })
}

export function saveWafConf(path: string, content: string) {
  return http.post<{ code: number; message: string; data: { reload?: string } }>(
    '/system/waf/conf/save',
    { path, content },
  )
}

/** 切换规则引擎形态：On（拦截）/ DetectionOnly（只记录不拦截）/ Off（关闭） */
export function setWafEngine(mode: 'On' | 'DetectionOnly' | 'Off') {
  return http.post<{
    code: number
    message: string
    data: { engine: string; reload: string }
  }>('/system/waf/engine', { mode })
}

export function getWafAudit(lines = 200) {
  return http.get<{ code: number; message: string; data: { path: string; content: string } }>(
    '/system/waf/audit',
    { params: { lines } },
  )
}
