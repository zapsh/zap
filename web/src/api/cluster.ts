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

/** 节点概览（仪表盘用，随用量一起下发） */
export interface ClusterNodeStats {
  /** 全部记录数（含待接入 / 已停用 / 已拒绝） */
  total: number
  /** 已接入（占授权名额） */
  approved: number
  /** 已接入且在线 */
  online: number
  /** 已接入但离线 / 尚未上报 */
  offline: number
  /** 待接入（等审批） */
  pending: number
  disabled: number
  rejected: number
}

/** 授权用量（列表页顶部 Banner / 仪表盘） */
export interface ClusterUsage {
  used: number
  max: number | null
  plan: string
  machineId: string
  expiresAt: number | null
  /** 当前已建立命令通道的节点数 */
  channels: number
  nodes: ClusterNodeStats
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

// ── 主控反向批量装机（v1.1，§11）───────────────────────
// 主控持 SSH 凭据远程跑 install.sh + join：只装主控，剩下的机器由它推过去。

export interface DeployHostReq {
  host: string
  port?: number
  /** 传给 --join-name 的节点名，留空用目标机主机名 */
  name?: string
}

/** 一次批量装机任务（不含任何凭据明文：密码/口令都不回传） */
export interface DeployTask {
  id: number
  join_url: string
  insecure: number
  pro: number
  version: string
  install_url: string
  enroll_code_id: number
  auth_user: string
  auth_type: string
  ssh_key_name: string
  push_key: number
  owner_id: number
  host_key_policy: string
  concurrency: number
  timeout_sec: number
  /** running | done | stopped */
  state: string
  total: number
  ok_count: number
  fail_count: number
  created_at: number
}

/** 单台机器的装机结果 */
export interface DeployHost {
  id: number
  task_id: number
  host: string
  port: number
  name: string
  /** pending | running | ok | failed | skipped */
  state: string
  message: string
  /** 远程输出尾部（排障用） */
  output: string
  /** TOFU 记下的主机密钥指纹 */
  host_key_fp: string
  started_at: number
  finished_at: number
  created_at: number
  /** 失败时才带：可复制的人工执行命令（口令不在响应里） */
  command?: string
}

export interface DeployPayload {
  hosts: DeployHostReq[]
  auth: {
    username: string
    authType: 'password' | 'key'
    password?: string
    sshKeyName?: string
  }
  joinUrl?: string
  insecure?: boolean
  /** 自动生成一枚本次专用注册口令（装完自动通过） */
  autoCode?: boolean
  /** autoCode = false 时用它；留空则不带口令（装完进待审批） */
  code?: string
  version?: string
  pro?: boolean
  installUrl?: string
  /** 顺带把主控公钥写进目标机 authorized_keys（以后免密） */
  pushKey?: boolean
  concurrency?: number
  timeoutSec?: number
  /** tofu（默认）| any */
  hostKeyPolicy?: string
}

export function createDeploy(data: DeployPayload) {
  return http.post('/pro/cluster/deploy', data)
}

export function getDeploy(id: number) {
  return http.get('/pro/cluster/deploy', { params: { id } })
}

export function listDeploys() {
  return http.get('/pro/cluster/deploy/list')
}

export function retryDeploy(data: { id: number; ids?: number[] }) {
  return http.post('/pro/cluster/deploy/retry', data)
}

export function deleteDeploy(id: number) {
  return http.delete('/pro/cluster/deploy', { params: { id } })
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
