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
}

/** 站点安全配置（含两项能力判定，供 UI 决定开关可用性） */
export async function getSiteSecurity(id: number) {
  return http.get<ApiResponse<SiteSecurityData>>('/site/security', { params: { id } })
}

/** 保存站点安全配置：后端保存后立即同步该站点 vhost */
export async function saveSiteSecurity(id: number, sec: SiteSecurity) {
  return http.post<ApiResponse>('/site/security/save', { id, sec })
}
