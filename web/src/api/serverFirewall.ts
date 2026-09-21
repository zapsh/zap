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
}

export interface FirewallRuleAddPayload {
  port: number
  proto?: 'tcp' | 'udp'
  action?: 'accept' | 'drop'
  /** 来源 IP / CIDR，留空表示不限来源 */
  source?: string
  comment?: string
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

/** 启停 / 开机自启：action = start | stop | enable | disable */
export function toggleFirewall(action: 'start' | 'stop' | 'enable' | 'disable') {
  return http.post<ApiResponse>('/system/config/firewall/toggle', { action })
}
