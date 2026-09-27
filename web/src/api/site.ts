import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

// ── 站点日志 ────────────────────────────────────────────────

/** 日志文件：当前日志（scope=current）或归档（scope=archive） */
export interface SiteLogFile {
  name: string
  /** access | error */
  kind: string
  /** current | archive */
  scope: string
  /** 归档日期 YYYYMMDD（当前日志为空） */
  date: string
  size: number
  mtime: number
}

export interface SiteLogsData {
  path: string
  lines: string[]
  count: number
  size: number
}

export interface SiteLogsArchivesData {
  current: SiteLogFile[]
  archives: SiteLogFile[]
}

/** 读取站点日志尾部行（kind: access | error；archive 为空 = 当前日志） */
export async function getSiteLogs(params: {
  id: number
  kind?: string
  archive?: string
  lines?: number
  keyword?: string
  status?: string
}) {
  return http.get<ApiResponse<SiteLogsData>>('/site/logs', { params })
}

/** 当前日志与历史归档列表 */
export async function getSiteLogArchives(id: number) {
  return http.get<ApiResponse<SiteLogsArchivesData>>('/site/logs/archives', { params: { id } })
}

/** 清空当前日志（kind 空 = access + error） */
export async function clearSiteLogs(id: number, kind = '') {
  return http.post<ApiResponse>('/site/logs/clear', { id, kind })
}

/** 立即轮转（按天切割归档） */
export async function rotateSiteLogs(id: number) {
  return http.post<ApiResponse>('/site/logs/rotate', { id })
}

// ── 站点流量分析 ────────────────────────────────────────────

export interface SiteTrafficPoint {
  /** YYYYMMDD */
  day: string
  bytes: number
  requests: number
}

export interface SiteTrafficTop {
  path: string
  hits: number
  bytes: number
}

export interface SiteTrafficData {
  daily: SiteTrafficPoint[]
  top: SiteTrafficTop[]
  today_bytes: number
  month_bytes: number
  /** 统计周期 YYYYMM */
  month: string
  total_bytes: number
}

/** 站点流量分析：按天曲线 + Top URL + 汇总 */
export async function getSiteTraffic(id: number, days = 30) {
  return http.get<ApiResponse<SiteTrafficData>>('/site/traffic', { params: { id, days } })
}

// ── 站点安全（WAF / 限速 / 限并发）────────────────────────────

export interface SiteSecurity {
  /** 站点启用 WAF（仍需全局已安装并启用 ModSecurity） */
  waf_enable: boolean
  /** WAF 模式：0=跟随全局 1=拦截(SecRuleEngine On) 2=仅检测(DetectionOnly) */
  waf_mode: number
  /** 站点自定义 ModSecurity 规则（管理组维护） */
  waf_rules: string
  /** 开启站点独立 WAF 审计日志（写入站点日志目录 waf.log） */
  waf_audit: boolean
  /** 限速 / WAF 白名单：IP 或 CIDR，逗号或换行分隔（管理组维护） */
  whitelist: string
  /** 限速干跑：命中只记日志不拦截（需 nginx ≥ 1.17.1） */
  limit_dry_run: boolean
  /** 请求限速 */
  limit_req_enable: boolean
  /** 每秒请求数上限 */
  limit_req_rate: number
  /** 突发放行数 */
  limit_req_burst: number
  /** 并发连接限制 */
  limit_conn_enable: boolean
  /** 单 IP 并发上限 */
  limit_conn_num: number
}

export interface SiteSecurityData {
  sec: SiteSecurity
  /** 套餐是否开放站点 WAF */
  waf_allowed: boolean
  /** 全局 WAF 是否已安装并启用 */
  waf_ready: boolean
  /** waf_ready=false 时的逐项原因（来自执行端检查） */
  blockers: string[]
}

/** 站点安全配置（含两项能力判定，供 UI 决定开关可用性） */
export async function getSiteSecurity(id: number) {
  return http.get<ApiResponse<SiteSecurityData>>('/site/security', { params: { id } })
}

export interface SecurityCaps {
  waf_allowed: boolean
  waf_ready: boolean
  /** waf_ready=false 时的逐项原因（来自执行端检查） */
  blockers: string[]
}

/** 安全能力判定（不依赖站点 id）：新建站点时用它决定 WAF 开关能否打开 */
export async function getSecurityCaps() {
  return http.get<ApiResponse<SecurityCaps>>('/site/security/caps')
}

/** 保存站点安全配置：后端保存后立即同步该站点 vhost */
export async function saveSiteSecurity(id: number, sec: SiteSecurity) {
  return http.post<ApiResponse>('/site/security/save', { id, sec })
}

// ── 站点应用（Application Manager）───────────────────────────

/** 站点应用：一个应用 = 一个 systemd unit，以站点用户身份运行 */
export interface SiteApp {
  id: number
  name: string
  /** python | nodejs */
  app_type: string
  workdir: string
  entry: string
  /** 自定义启动命令（空 = 用类型默认模板） */
  command: string
  port: number
  env: string
  autostart: boolean
  running: boolean
  /** systemd is-active 结果：active / inactive / unknown ... */
  state: string
  active: boolean
  enabled: boolean
  pid: number
}

/** 当前操作者在该站点上的应用能力（套餐控制） */
export interface SiteAppCaps {
  allowed: boolean
  types: string[]
  /** 每站点应用数上限（0 = 不限） */
  max_apps: number
  /** 应用可监听端口下界（0 = 不限） */
  port_min: number
  /** 应用可监听端口上界（0 = 不限） */
  port_max: number
  /** 该用户全部站点合计应用数上限（0 = 不限） */
  max_total: number
  /** 该用户已部署的应用数 */
  used_total: number
}

export function getSiteAppCaps(site_id: number) {
  return http.get<ApiResponse<SiteAppCaps>>('/site/app/caps', { params: { site_id } })
}

export function getSiteApps(site_id: number) {
  return http.get<ApiResponse<{ apps: SiteApp[] }>>('/site/app/list', { params: { site_id } })
}

export interface SiteAppDeployPayload {
  site_id: number
  name: string
  app_type: string
  workdir?: string
  entry?: string
  command?: string
  port?: number
  env?: string
  autostart?: boolean
  install_deps?: boolean
}

export function deploySiteApp(p: SiteAppDeployPayload) {
  return http.post<ApiResponse>('/site/app/deploy', p)
}

/** start | stop | restart | enable | disable */
export function siteAppAction(site_id: number, name: string, action: string) {
  return http.post<ApiResponse>('/site/app/action', { site_id, name, action })
}

export function removeSiteApp(site_id: number, name: string) {
  return http.post<ApiResponse>('/site/app/remove', { site_id, name })
}

export function getSiteAppLog(site_id: number, name: string, lines = 200) {
  return http.get<ApiResponse<{ lines: string[] }>>('/site/app/log', {
    params: { site_id, name, lines },
  })
}
