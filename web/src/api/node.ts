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
  sampled_at: number | null
  trend: NodeTrendPoint[]
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

export interface NodeOverview {
  nodes: NodeOverviewItem[]
  summary: NodeSummary
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
}

export interface NodePayload {
  name: string
  base_url: string
  api_token?: string
  remark?: string
  insecure_tls?: boolean
  status?: number
}

export function listNodes() {
  return http.get<ApiResponse<NodeItem[]>>('/node/list')
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

/** 连通性测试；编辑态传 id 且 api_token 留空时复用库里已存的 Token */
export function testNode(data: {
  base_url: string
  api_token?: string
  insecure_tls?: boolean
  id?: number
}) {
  return http.post<ApiResponse<NodeSnapshot>>('/node/test', data)
}

/** 立刻采集一轮（新增节点后不必等下个周期） */
export function refreshNodes() {
  return http.post<ApiResponse<null>>('/node/refresh')
}

export function getNodesOverview() {
  return http.get<ApiResponse<NodeOverview>>('/node/overview')
}

/** 单节点历史曲线；range 与本机监控一致（live/1h/6h/24h/7d/30d） */
export function getNodeStatus(id: number, range?: string) {
  return http.get<ApiResponse<{ node_id: number; name: string; points: NodeStatPoint[] }>>(
    '/node/status',
    { params: range && range !== 'live' ? { id, range } : { id } },
  )
}
