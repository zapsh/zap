import { http } from '@/utils/request'

// ── 节点 ────────────────────────────────────────────────
// state: 0 待接入 / 1 已接入 / 2 已拒绝 / 3 已停用
// online: 0 未知 / 1 在线 / 2 离线
export interface ClusterNode {
  id: number
  node_id: string
  name: string
  hostname: string
  os: string
  version: string
  state: number
  token_id: number
  node_prefix: string
  ctrl_addr: string
  enroll_code_id: number
  enroll_ip: string
  online: number
  last_report_at: number
  last_error: string
  cpu_usage: number
  mem_usage: number
  disk_usage: number
  load1: number
  site_count: number
  approved_at: number
  created_at: number
  /** 命令通道（长连）是否已建立：true 才能被主控推送命令 */
  channel?: boolean
  /** 长连建立时刻（时间戳秒，未连接为 null） */
  channelSince?: number | null
}

/** 注册口令（列表项：不含明文，明文只在生成那一次返回） */
export interface EnrollCode {
  id: number
  name: string
  code_prefix: string
  expires_at: number
  max_uses: number
  used_count: number
  ip_allow: string
  status: number
  created_by: number
  last_used_at: number
  created_at: number
}

/** 授权用量（列表页顶部 Banner） */
export interface ClusterUsage {
  used: number
  max: number | null
  plan: string
  machineId: string
  expiresAt: number | null
  /** 当前已建立命令通道的节点数 */
  channels: number
}

/** 本机接入状态（受管视图） */
export interface ClusterSelf {
  joined: boolean
  joinUrl: string
  /** 主控机器码后 8 位：只够核对「我归哪台主控管」 */
  ctrlMachineIdTail: string
  state: string
  lastReportAt: number
  lastError: string
  enrolledAt: number
  tlsMode: number
  tokenPrefix: string
  hasToken: boolean
  /** 命令通道：1=已连上主控，可被推命令；0=未连接 */
  wsState: number
  wsSince: number
  /** 主控授权摘要（report 响应下发，仅展示用） */
  ctrlLicense: { expiresAt?: number | null; maxNodes?: number | null; usedNodes?: number }
}

/** 命令通道（v1.1）：顺着长连给节点下发命令 */
export type ClusterCommand = 'ping' | 'report' | 'rejoin'

export function commandNodes(data: { ids: number[]; cmd: ClusterCommand; url?: string; insecure?: boolean }) {
  return http.post('/pro/cluster/nodes/command', data)
}

/** 让选中的节点改用新地址重连：有长连的直接推，推不到的返回人工命令 */
export function rejoinNodes(ids: number[]) {
  return http.post('/pro/cluster/nodes/rejoin', { ids })
}

/** 导出逐台重连命令（rejoin.sh） */
export function getRejoinScript(ids?: number[]) {
  return http.get('/pro/cluster/nodes/rejoin-script', {
    params: ids && ids.length ? { ids: ids.join(',') } : {}
  })
}

export function getNodes(state?: number) {
  return http.get('/pro/cluster/nodes', { params: state === undefined ? {} : { state } })
}

export function updateNode(data: { id: number; name?: string; ctrlAddr?: string }) {
  return http.put('/pro/cluster/nodes', data)
}

export function deleteNode(id: number) {
  return http.delete('/pro/cluster/nodes', { params: { id } })
}

export function approveNodes(ids: number[]) {
  return http.post('/pro/cluster/nodes/approve', { ids })
}

export function rejectNode(id: number) {
  return http.post('/pro/cluster/nodes/reject', { id })
}

export function disableNode(id: number) {
  return http.post('/pro/cluster/nodes/disable', { id })
}

export function enableNode(id: number) {
  return http.post('/pro/cluster/nodes/enable', { id })
}

export function rotateNode(id: number) {
  return http.post('/pro/cluster/nodes/rotate', { id })
}

// ── 注册口令 ────────────────────────────────────────────
export function getEnrollCodes() {
  return http.get('/pro/cluster/codes')
}

/** 生成口令：明文只在这一次返回，之后只能看到前缀 */
export function createEnrollCode(data: {
  name?: string
  ttlSec?: number
  maxUses?: number
  ipAllow?: string
}) {
  return http.post('/pro/cluster/codes', data)
}

export function revokeEnrollCode(id: number) {
  return http.delete('/pro/cluster/codes', { params: { id } })
}

// ── 用量与本机接入 ──────────────────────────────────────
export function getClusterUsage() {
  return http.get('/pro/cluster/usage')
}

export function getClusterSelf() {
  return http.get('/pro/cluster/self')
}

/** 接入 / 重新接入主控（受控端「一键重连」） */
export function rejoinCluster(data: { url: string; token?: string; insecure?: boolean }) {
  return http.post('/pro/cluster/self/rejoin', data)
}

/** 断开接入 */
export function unjoinCluster() {
  return http.delete('/pro/cluster/self')
}
