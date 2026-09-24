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
  /** 节点公钥指纹（SPKI sha256 前 32 位 hex），空=未登记公钥 */
  key_fp?: string
  /** v3 上报方式：0=节点主动上报（要能出网），1=主控主动拉取（给不能出网的机器） */
  report_mode?: number
  /** v4 批量编排：分组标签（空 = 未分组） */
  node_group?: string
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
  /** 命令通道：1=已连上主控，可被推命令；0=未连接 */
  wsState: number
  wsSince: number
  /** 主控授权摘要（report 响应下发，仅展示用） */
  ctrlLicense: { expiresAt?: number | null; maxNodes?: number | null; usedNodes?: number }
  /** 本机 Ed25519 密钥指纹（主控侧登记的就是它） */
  keyFp: string
  /** 主控证书 SPKI 钉（前 16 位），钉住后换证书一律拒绝 */
  ctrlPin: string
  /** v3：本机面板地址 —— 主控「一键 SSH」就是直连这个地址 */
  selfAddr: string
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

/**
 * v3 一键 SSH：向这台节点要一枚 5 分钟的终端票据。
 *
 * 票据由**被控端自己**签发（主控拿不到它的 JWT 密钥），浏览器随后直连返回的
 * `wsUrl` 开终端 —— 全程不经过主控转发。
 */
export interface SshTicket {
  wsUrl: string
  token: string
  connId: number
  ttlSec: number
}

export function sshTicket(id: number) {
  return http.post<{ code: number; message?: string; data: SshTicket }>(
    '/pro/cluster/nodes/ssh',
    { id },
  )
}

/**
 * v3 pull：切上报方式。`pull` = 主控主动去拉（节点出不了网时用它），
 * 切换后会立刻拉一次，拉不通当场把原因带回来。
 */
export interface NodeModeResult {
  mode: string
  reportMode: number
  /** 切到 pull 时是否立刻拉到了快照 */
  pulled?: boolean
  error?: string
}

export function setNodeMode(id: number, mode: 'push' | 'pull') {
  return http.post<{ code: number; message?: string; data: NodeModeResult }>(
    '/pro/cluster/nodes/mode',
    { id, mode },
  )
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

// ── v2：聚合大屏 / 快照历史 / 告警规则 / Webhook ─────────
// 快照历史与告警事件存在**独立时序库**，接口形状由后端拼好，前端不需要关心。

export interface ClusterTopItem {
  id: number
  name: string
  value: number
  metric: string
}

export interface ClusterOverview {
  counts: {
    total: number
    approved: number
    online: number
    offline: number
    pending: number
    disabled: number
  }
  /** 机队均值：只统计在线节点，离线节点的旧值不算进来 */
  fleet: {
    cpuAvg: number
    memAvg: number
    diskAvg: number
    loadAvg: number
    sites: number
    sampled: number
  }
  top: { cpu: ClusterTopItem[]; mem: ClusterTopItem[]; disk: ClusterTopItem[] }
  /** 未恢复告警数 */
  firing: number
  recent: AlertEvent[]
  storage: { rawDays: number; rollup5Days: number; rollup1hDays: number }
}

export interface AlertEvent {
  id: number
  rule_id: number
  rule_name: string
  node_key: string
  node_name: string
  metric: string
  value: number
  threshold: number
  /** 触发时刻；resolved_at = 0 表示还在告警中 */
  fired_at: number
  resolved_at: number
}

export interface HistoryPoint {
  ts: number
  avg: number
  max: number
}

export interface AlertRule {
  id: number
  name: string
  /** cpu | mem | disk | load1 | offline */
  metric: string
  /** gt | lt */
  op: string
  threshold: number
  durationSec: number
  /** 作用节点 id；空 = 全部已接入节点 */
  scope: number[]
  webhookIds: number[]
  enabled: boolean
  silenceUntil: number
  silenced: boolean
  createdAt: number
  updatedAt: number
}

export interface ClusterWebhook {
  id: number
  name: string
  url: string
  /** 只回显后 4 位，够确认是哪个、不够拿去伪造请求 */
  secretTail: string
  hasSecret: boolean
  enabled: boolean
  createdAt: number
  updatedAt: number
}

export interface NotifyLog {
  id: number
  kind: string
  target: string
  ok: number
  detail: string
  created_at: number
}

export function getClusterOverview() {
  return http.get('/pro/cluster/overview')
}

/** 单节点历史曲线。hours > 6 自动读聚合点（后端降采样到 ~360 个点） */
export function getNodeHistory(params: { id: number; metric?: string; hours?: number }) {
  return http.get('/pro/cluster/nodes/history', { params })
}

export function getAlerts(params?: { limit?: number; openOnly?: boolean }) {
  return http.get('/pro/cluster/alerts', {
    params: { limit: params?.limit ?? 50, openOnly: params?.openOnly ? 1 : 0 }
  })
}

/** 投递日志：排查「为什么没收到通知」 */
export function getNotifyLogs(limit = 50) {
  return http.get('/pro/cluster/notify-logs', { params: { limit } })
}

export function getAlertRules() {
  return http.get('/pro/cluster/rules')
}

export function saveAlertRule(data: Partial<AlertRule> & { name: string; metric: string }) {
  return data.id ? http.put('/pro/cluster/rules', data) : http.post('/pro/cluster/rules', data)
}

export function setAlertRuleState(data: { id: number; enabled?: boolean; silenceMin?: number }) {
  return http.post('/pro/cluster/rules/state', data)
}

export function deleteAlertRule(id: number) {
  return http.delete('/pro/cluster/rules', { params: { id } })
}

export function getWebhooks() {
  return http.get('/pro/cluster/webhooks')
}

export function saveWebhook(data: {
  id?: number
  name: string
  url: string
  secret?: string
  enabled?: boolean
}) {
  return data.id ? http.put('/pro/cluster/webhooks', data) : http.post('/pro/cluster/webhooks', data)
}

export function deleteWebhook(id: number) {
  return http.delete('/pro/cluster/webhooks', { params: { id } })
}

/** 发一条测试载荷，验证地址/签名对不对 */
export function testWebhook(id: number) {
  return http.post('/pro/cluster/webhooks/test', { id })
}

// ── v4 批量命令编排：分组 / 任务库 / 执行批次 ─────────────

export function setNodeGroup(data: { ids: number[]; group: string }) {
  return http.post('/pro/cluster/nodes/group', data)
}

export function getNodeGroups() {
  return http.get('/pro/cluster/nodes/groups')
}

/** 预存任务：常用命令存成任务，对分组 / 选中节点一键复用 */
export interface CmdTask {
  id: number
  name: string
  command: string
  timeout: number
  note: string
  created_by: number
  created_at: number
}

export function getCmdTasks() {
  return http.get('/pro/cluster/cmd/tasks')
}

export function saveCmdTask(data: {
  id?: number
  name: string
  command: string
  timeout?: number
  note?: string
}) {
  return data.id ? http.put('/pro/cluster/cmd/tasks', data) : http.post('/pro/cluster/cmd/tasks', data)
}

export function deleteCmdTask(id: number) {
  return http.delete(`/pro/cluster/cmd/tasks/${id}`)
}

/** 一次批量执行的批次（目标在发起时快照成 id 列表，分组变化不影响已发批次） */
export interface CmdRun {
  id: number
  task_id: number
  name: string
  command: string
  timeout: number
  targets: string
  total: number
  ok_count: number
  fail_count: number
  /** 0=执行中 1=已结束 */
  status: number
  created_by: number
  created_at: number
  finished_at: number
}

export interface CmdResult {
  id: number
  run_id: number
  node_id: number
  name: string
  /** 0=待执行 1=成功 2=失败 */
  status: number
  exit_code: number
  message: string
  output: string
  duration_ms: number
}

export function createCmdRun(data: {
  taskId?: number
  command?: string
  timeout?: number
  ids?: number[]
  group?: string
  all?: boolean
}) {
  return http.post('/pro/cluster/cmd/runs', data)
}

export function getCmdRuns(limit = 20) {
  return http.get('/pro/cluster/cmd/runs', { params: { limit } })
}

export function getCmdRun(id: number) {
  return http.get(`/pro/cluster/cmd/runs/${id}`)
}

export function deleteCmdRun(id: number) {
  return http.delete(`/pro/cluster/cmd/runs/${id}`)
}
