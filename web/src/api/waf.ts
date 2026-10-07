// SPDX-License-Identifier: AGPL-3.0-only
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
  /** 组件已在盘上、只差挂到 nginx：可一键开启（不必重装） */
  enable_ready: boolean
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

/** 一键开启：补 load_module + conf.d 启用文件并重载 */
export function enableWaf() {
  return http.post<{ code: number; message: string; data: { enabled: string; reload: string } }>(
    '/system/waf/enable',
  )
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

/** 单条审计命中规则（节 H 解析结果） */
export interface WafAuditMessage {
  /** 动作：Warning / Access denied with code 403 (phase 2) ... */
  action: string
  /** 规则 id（如 932160 / 949110） */
  id: string
  /** 规则描述（msg） */
  msg: string
  /** severity 名称（CRITICAL / WARNING ...） */
  severity: string
  /** severity 原始数字 */
  severity_raw: string
  /** 规则文件 */
  file: string
  /** 规则行号 */
  line: string
  /** 匹配到的具体数据 */
  data: string
  /** 标签（attack-rce / paranoia-level/1 ...） */
  tags: string[]
}

/** 按 unique_id 查询返回的审计明细 */
export interface WafAuditData {
  path: string
  /** 兼容旧视图：尾部模式时即原始文本；unique_id 模式时为该条原始块 */
  content: string
  /** 是否命中（unique_id 模式） */
  found?: boolean
  unique_id?: string
  /** 节 A 头部 */
  header?: {
    timestamp: string
    unique_id: string
    client_ip: string
    client_port: string
    server_ip: string
    server_port: string
  }
  /** 节 B 请求行 */
  request?: {
    line: string
    method: string
    uri: string
    protocol: string
  }
  /** 节 H 命中的规则列表 */
  messages?: WafAuditMessage[]
  /** unique_id 模式下的完整原始条目 */
  raw?: string
}

export function getWafAudit(lines = 200, unique_id?: string) {
  return http.get<{ code: number; message: string; data: WafAuditData }>(
    '/system/waf/audit',
    { params: { lines, ...(unique_id ? { unique_id } : {}) } },
  )
}
