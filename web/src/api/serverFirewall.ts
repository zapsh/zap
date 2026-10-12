// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

/** 防火墙后端：Linux 上的 firewalld / ufw / nftables / iptables */
export type FirewallBackend =
  | 'firewalld'
  | 'ufw'
  | 'nftables'
  | 'iptables'
  | 'pf'
  | 'ipfw'
  | 'none'

export interface FirewallRule {
  /** 各后端自解释的删除标识 */
  id: string
  port: number
  proto: string
  action: 'accept' | 'drop'
  /** 来源 IP / CIDR，空表示不限来源 */
  source: string
  comment: string
  /** 命中面板监听端口（受保护，不允许 drop 或删除放行规则） */
  protected: boolean
}

export interface FirewallStatus {
  backend: FirewallBackend
  /** running=确实在过滤流量；installed=只装了命令、当前并未生效；none=未检测到 */
  detected: 'running' | 'installed' | 'none'
  active: boolean
  enabled: boolean
  /** WSL 环境：共享内核，规则可能不生效 */
  wsl: boolean
  panel_port: number
  rules: FirewallRule[]
  /** 端口转发规则（DNAT） */
  forwards: FirewallForward[]
  /** IP 黑名单 */
  blacklist: FirewallBlackEntry[]
  /** 内核 IPv4 转发是否开启（端口转发前置条件） */
  ip_forward: boolean
  /** 当前后端是否支持端口转发（ufw 不支持） */
  forward_supported: boolean
  /** 是否处于锁服状态（封禁全部、仅白名单放通） */
  locked_down: boolean
}

/** 端口转发规则：外部 <proto>/<listen_port> → <target_addr>:<target_port> */
export interface FirewallForward {
  id: string
  proto: 'tcp' | 'udp' | string
  listen_port: number
  target_addr: string
  target_port: number
  comment: string
}

/** IP 黑名单条目：拒绝该 IP / CIDR 的全部入站流量 */
export interface FirewallBlackEntry {
  id: string
  ip: string
  comment: string
}

/** IP 白名单条目：放行优先，命中后黑名单 / IP Ban 都不再封它 */
export interface FirewallWhiteEntry {
  ip: string
  comment?: string
  created_at?: number
}

/** IP Ban（fail2ban）设置 */
export interface BanSettings {
  enabled: boolean
  max_retry: number
  find_time: number
  ban_time: number
  interval: number
  ignore_private: boolean
  ignored_ips: string[]
  log_file: string
}

/** 当前生效的封禁 */
export interface BanRecord {
  ip: string
  reason: string
  hits: number
  created_at: number
  expires_at: number
  /** 剩余秒数（后端计算） */
  remaining: number
}

export interface BanStatus {
  settings: BanSettings
  bans: BanRecord[]
  /** 窗口内正在累计失败次数的 IP */
  watching: Array<{ ip: string; hits: number; last: number }>
  log_file: string
  last_run: number
  total_banned: number
  last_error: string
}

export interface FirewallRuleAddPayload {
  port: number
  proto?: 'tcp' | 'udp'
  action?: 'accept' | 'drop'
  /** 来源 IP / CIDR，留空表示不限来源 */
  source?: string
  comment?: string
}

export interface FirewallForwardAddPayload {
  proto?: 'tcp' | 'udp'
  listen_port: number
  target_addr: string
  target_port: number
  comment?: string
}

/** 导出文件结构（firewall.export 的 data） */
export interface FirewallExport {
  version: number
  backend?: string
  port_rules?: Array<{
    port: number
    proto?: string
    action?: string
    source?: string
    comment?: string
  }>
  forwards?: Array<{
    proto?: string
    listen_port: number
    target_addr: string
    target_port: number
    comment?: string
  }>
  blacklist?: Array<{ ip: string; comment?: string }>
}

/** 读取防火墙状态与规则列表（仅 admin） */
export function getFirewallStatus() {
  return http.get<ApiResponse<FirewallStatus>>('/system/config/firewall')
}

/** 新增放行 / 拒绝规则 */
export function addFirewallRule(data: FirewallRuleAddPayload) {
  return http.post<ApiResponse>('/system/config/firewall/rule/add', data)
}

/** 删除规则（id 来自状态接口） */
export function deleteFirewallRule(id: string) {
  return http.post<ApiResponse>('/system/config/firewall/rule/delete', { id })
}

/** 修改规则（先删后加，受保护规则不可改） */
export function updateFirewallRule(data: {
  id: string
  port: number
  proto: string
  action: string
  source: string
  comment: string
}) {
  return http.post<ApiResponse>('/system/config/firewall/rule/update', data)
}

/** 启停 / 开机自启：action = start | stop | enable | disable */
export function toggleFirewall(action: 'start' | 'stop' | 'enable' | 'disable') {
  return http.post<ApiResponse>('/system/config/firewall/toggle', { action })
}

/** 新增端口转发规则 */
export function addFirewallForward(data: FirewallForwardAddPayload) {
  return http.post<ApiResponse>('/system/config/firewall/forward/add', data)
}

/** 删除端口转发规则（id 来自状态接口） */
export function deleteFirewallForward(id: string) {
  return http.post<ApiResponse>('/system/config/firewall/forward/delete', { id })
}

/** IP 黑名单：封禁 IP / CIDR */
export function addFirewallBlacklist(ip: string, comment = '') {
  return http.post<ApiResponse>('/system/config/firewall/blacklist/add', { ip, comment })
}

/** 删除黑名单条目（id 来自状态接口） */
export function deleteFirewallBlacklist(id: string) {
  return http.post<ApiResponse>('/system/config/firewall/blacklist/delete', { id })
}

/** 导出全部规则（返回导出 JSON 结构） */
export function exportFirewall() {
  return http.get<ApiResponse<FirewallExport>>('/system/config/firewall/export')
}

/** 导入规则（传入导出 JSON 结构） */
export function importFirewall(data: FirewallExport) {
  return http.post<ApiResponse & { data?: { added: number; skipped: number; failed?: string[] } }>(
    '/system/config/firewall/import',
    data,
  )
}

/** 一键锁服：封禁全部入站，仅开放 SSH(22) / 80 / 443 / 面板端口 */
export function lockdownFirewall(unlock = false) {
  return http.post<ApiResponse>('/system/config/firewall/lockdown', { unlock })
}

// ── IP 白名单 ────────────────────────────────────────────────

/** 白名单列表（面板策略清单 + 防火墙后端实际 accept 规则） */
export function getFirewallWhitelist() {
  return http.get<ApiResponse<{ whitelist: FirewallWhiteEntry[]; rules: Array<{ id: string; ip: string }> }>>(
    '/system/config/firewall/whitelist',
  )
}

/** 加入白名单：同时写面板清单与防火墙 accept 规则 */
export function addFirewallWhitelist(ip: string, comment = '') {
  return http.post<ApiResponse>('/system/config/firewall/whitelist/add', { ip, comment })
}

/** 移出白名单 */
export function deleteFirewallWhitelist(ip: string) {
  return http.post<ApiResponse>('/system/config/firewall/whitelist/delete', { ip })
}

// ── IP Ban（内置 fail2ban）─────────────────────────────

/** IP Ban 状态：设置 + 生效封禁 + 正在计数的 IP */
export function getBanStatus() {
  return http.get<ApiResponse<BanStatus>>('/system/config/firewall/ban/status')
}

/** 保存 IP Ban 设置 */
export function saveBanSettings(data: BanSettings) {
  return http.post<ApiResponse>('/system/config/firewall/ban/settings', data)
}

/** 手动提前解封某个 IP */
export function unbanIp(ip: string) {
  return http.post<ApiResponse>('/system/config/firewall/ban/unban', { ip })
}
