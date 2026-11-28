// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 集群（受管节点）接口。
 *
 * 凭据模型：每台节点存一个**在该节点上签发的 API Token**（`zap_` 开头），
 * 后端加密落库、任何接口都不会回传原文，因此编辑时 Token 留空 = 不修改。
 * 采集只打节点的 `/system/info`（普通用户即可），不需要在节点上开管理员。
 */
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

export interface NodeDisk {
  name: string
  mount_point: string
  file_system: string
  total_space: number
  available_space: number
}

export interface NodeItem {
  id: number
  name: string
  base_url: string
  remark: string
  insecure_tls: boolean
  /** 1 = 启用采集，0 = 停用 */
  status: number
  online: boolean
  last_seen: number
  last_error: string
  os_name: string
  host_name: string
  version: string
  public_ip: string
  uptime: string
  cpu_num: number
  memory_total_b: number
  disk_total_b: number
  disks: NodeDisk[]
  group_id: number
  group_name: string | null
  owner_id: number
  /** Token 签发时间 / 到期时间（0 = 永久或未知） */
  cred_issued_at: number
  cred_expires_at: number
  cred_issued_by: string
  /** valid / expired / invalid / unknown */
  cred_status: string
  created_at: number
  updated_at: number
}

export interface NodeTrendPoint {
  t: number
  cpu: number
  mem: number
  load: number
}

/** 大屏用节点：最新一次采样值 + 近 30 分钟趋势 */
export interface NodeOverviewItem extends NodeItem {
  cpu_usage: number | null
  memory_usage: number | null
  swap_usage: number | null
  disk_usage: number | null
  loadavg_one: number | null
  loadavg_five: number | null
  loadavg_fifteen: number | null
  net_rx_bps: number | null
  net_tx_bps: number | null
  sampled_at: number | null
  trend: NodeTrendPoint[]
}

export interface NodeProcess {
  pid: number
  name: string
  cpu: number
  memory: number
}

export interface NodeSummary {
  total: number
  online: number
  offline: number
  avg_cpu_usage: number
  avg_load: number
  max_load: number
  memory_total_b: number
  memory_used_b: number
  disk_total_b: number
  disk_used_b: number
}

export interface NodeGroupSummary {
  group_id: number
  name: string
  total: number
  online: number
  offline: number
  avg_cpu_usage: number
}

export interface NodeOverview {
  nodes: NodeOverviewItem[]
  summary: NodeSummary
  group_summary: NodeGroupSummary[]
  /** 后端采集间隔（秒），前端据此设轮询节奏 */
  interval: number
}

export interface NodeSnapshot {
  os_name: string
  host_name: string
  version: string
  cpu_num: number
  memory_total_b: number
  uptime: string
  public_ip: string
}

export interface NodeStatPoint {
  created_at: number
  cpu_usage: number | null
  memory_usage: number | null
  swap_usage: number | null
  disk_usage: number | null
  loadavg_one: number | null
  loadavg_five: number | null
  loadavg_fifteen: number | null
  net_rx_bps: number | null
  net_tx_bps: number | null
}

export interface NodePayload {
  name: string
  base_url: string
  api_token?: string
  /** 引导凭据：api_token 留空时用它登录节点并自动签发 Token；后端不落库 */
  username?: string
  password?: string
  /** 节点账号开了两步验证时的 6 位验证码 */
  totp_code?: string
  remark?: string
  insecure_tls?: boolean
  status?: number
  group_id?: number
  owner_id?: number
  /** 账号密码签发时 Token 的有效天数，0 / 不填 = 节点默认 */
  cred_expire_days?: number
}

export interface NodeListParams {
  keyword?: string
  group_id?: number
  status?: number
  online?: number
  sort?: 'id' | 'name' | 'cpu' | 'memory' | 'online' | 'last_seen'
}

export function listNodes(params?: NodeListParams) {
  return http.get<ApiResponse<NodeItem[]>>('/node/list', { params })
}

export interface NodeGroupItem {
  id: number
  name: string
  sort_order: number
  count: number
}

export function listNodeGroups() {
  return http.get<ApiResponse<NodeGroupItem[]>>('/node/group/list')
}

export function addNodeGroup(name: string, sortOrder = 0) {
  return http.post<ApiResponse<{ id: number }>>('/node/group/add', {
    name,
    sort_order: sortOrder,
  })
}

export function updateNodeGroup(id: number, name: string, sortOrder = 0) {
  return http.post<ApiResponse<null>>('/node/group/update', {
    id,
    name,
    sort_order: sortOrder,
  })
}

export function deleteNodeGroup(id: number) {
  return http.post<ApiResponse<null>>('/node/group/delete', { id })
}

/** 可归属的用户（管理员全部，经销商自己的成员） */
export function listNodeOwners() {
  return http.get<ApiResponse<{ id: number; username: string }[]>>('/node/owners')
}

export function batchUpdateNodes(data: { ids: number[]; status?: number; group_id?: number }) {
  return http.post<ApiResponse<{ updated: number }>>('/node/batch/update', data)
}

/** 批量下发节点上已有的应用商店脚本（不执行任意命令） */
export function batchRunScript(ids: number[], path: string) {
  return http.post<ApiResponse<{ results: { id: number; ok: boolean; message: string }[] }>>(
    '/node/batch/script',
    { ids, path },
  )
}

export interface AlertConfig {
  cpu: number
  memory: number
  disk: number
  offline_seconds: number
  cooldown_seconds: number
  email: boolean
}

export function getAlertConfig() {
  return http.get<ApiResponse<AlertConfig>>('/node/alert/config')
}

export function setAlertConfig(data: AlertConfig) {
  return http.post<ApiResponse<null>>('/node/alert/config', data)
}

export interface NodeAlertEvent {
  id: number
  node_id: number
  node_name: string
  kind: string
  message: string
  created_at: number
}

export function listAlertEvents() {
  return http.get<ApiResponse<NodeAlertEvent[]>>('/node/alert/events')
}

export function addNode(data: NodePayload) {
  return http.post<ApiResponse<{ id: number }>>('/node/add', data)
}

/** 后端按 COALESCE 更新：只传要改的字段，api_token 省略即保持原值 */
export function updateNode(data: { id: number } & Partial<NodePayload>) {
  return http.post<ApiResponse<null>>('/node/update', data)
}

export function deleteNodes(ids: number[]) {
  return http.post<ApiResponse<{ deleted: number }>>('/node/delete', { ids })
}

/**
 * 连通性测试；编辑态传 id 且 api_token 留空时复用库里已存的 Token。
 * 账号密码模式只验证能否登录、不签发 Token，成功时 data 为 null。
 */
export function testNode(data: {
  base_url: string
  api_token?: string
  username?: string
  password?: string
  totp_code?: string
  insecure_tls?: boolean
  id?: number
}) {
  return http.post<ApiResponse<NodeSnapshot | null>>('/node/test', data)
}

/** 立刻采集一轮（新增节点后不必等下个周期） */
export function refreshNodes() {
  return http.post<ApiResponse<null>>('/node/refresh')
}

export function getNodesOverview(group_id?: number) {
  return http.get<ApiResponse<NodeOverview>>('/node/overview', {
    params: group_id ? { group_id } : undefined,
  })
}

/** 单节点历史曲线；range 与本机监控一致（live/1h/6h/24h/7d/30d） */
export function getNodeStatus(id: number, range?: string) {
  return http.get<
    ApiResponse<{
      node_id: number
      name: string
      range: string
      points: NodeStatPoint[]
      top_processes: NodeProcess[]
    }>
  >('/node/status', { params: range && range !== 'live' ? { id, range } : { id } })
}
