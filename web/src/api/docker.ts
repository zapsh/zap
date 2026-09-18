import { http } from '@/utils/request'

// ── 数据模型 ─────────────────────────────────────────────
// 后端走 Docker Engine API，字段名沿用 docker CLI 的大写风格（前端已按此绑定）。
// 标 `?` 的字段取决于 API 版本 / 查询参数，可能缺席，UI 上不做强依赖。

/** 容器行 */
export interface DockerContainer {
  ID: string
  Image: string
  Names: string
  State: string
  Status: string
  Ports: string
  CreatedAt: string
  RunningFor?: string
  Size?: string
  Labels?: string
  /** 后端从 compose label 解析出的项目名称（可能为空串） */
  project: string
}

/** 资源快照行 */
export interface DockerStat {
  ID?: string
  Name: string
  CPUPerc: string
  MemUsage: string
  MemPerc: string
  NetIO?: string
  BlockIO?: string
  PIDs?: string
}

/** 镜像行 */
export interface DockerImage {
  ID: string
  Repository: string
  Tag: string
  Digest?: string
  Size: string
  CreatedAt: string
  CreatedSince?: string
  Containers: string
}

/** 数据卷行 */
export interface DockerVolume {
  Name: string
  Driver: string
  Mountpoint: string
  /**
   * 数据真正所在的宿主机目录：bind mount 卷取 `Options.device`，其余同 Mountpoint。
   * local 驱动建 bind 卷时 Mountpoint 仍指向 daemon 默认目录，面板要展示的是这里。
   */
  DataDir?: string
  Scope: string
  Labels?: string
  CreatedAt?: string
}

/** 网络行 */
export interface DockerNetwork {
  ID: string
  Name: string
  Driver: string
  Scope: string
  /** Engine API 直接给布尔值 */
  IPv6: boolean
  Internal?: boolean
  CreatedAt?: string
}

/** Compose 项目（`docker compose ls`） */
export interface DockerComposeProject {
  Name: string
  Status: string
  ConfigFiles: string
}

export interface DockerEnvStatus {
  installed: boolean
  daemon: boolean
  version: string
  compose: boolean
  error: string
}

/** 批量操作的失败明细 */
export interface DockerFailedItem {
  id: string
  error: string
}

export interface DockerActionResult {
  output?: string
  succeeded?: number
  failed?: DockerFailedItem[]
}

export interface DockerLogsResult {
  log: string
}

type Api<T> = { code: number; message: string; data: T }

// ── 环境探测 ─────────────────────────────────────────────

export function dockerStatus() {
  return http.get<Api<DockerEnvStatus>>('/docker/status')
}

// ── 容器 ─────────────────────────────────────────────────

export function listContainers(all = true) {
  return http.get<Api<{ items: DockerContainer[] }>>('/docker/containers', {
    params: { all },
  })
}

export function containerStats() {
  return http.get<Api<{ items: DockerStat[] }>>('/docker/stats')
}

export function containerInspect(id: string) {
  return http.get<Api<Record<string, unknown>>>(`/docker/container/inspect`, {
    params: { id },
  })
}

export function containerLogs(params: {
  id: string
  tail?: number
  since?: string
  timestamps?: boolean
}) {
  return http.get<Api<DockerLogsResult>>('/docker/container/logs', { params })
}

/** 批量容器动作：start / stop / restart / pause / unpause / kill / remove */
export function containerAction(ids: string[], action: string) {
  return http.post<Api<DockerActionResult>>('/docker/container/action', { ids, action })
}

// ── 镜像 ─────────────────────────────────────────────────

export function listImages() {
  return http.get<Api<{ items: DockerImage[] }>>('/docker/images')
}

/** pull（id 传仓库引用）/ remove / prune */
export function imageAction(id: string, action: string) {
  return http.post<Api<DockerActionResult>>('/docker/image/action', { id, action })
}

// ── 数据卷 ───────────────────────────────────────────────

export function listVolumes() {
  return http.get<Api<{ items: DockerVolume[] }>>('/docker/volumes')
}

/** create / remove / prune */
export function volumeAction(name: string, action: string) {
  return http.post<Api<DockerActionResult>>('/docker/volume/action', { name, action })
}

// ── 网络 ─────────────────────────────────────────────────

export function listNetworks() {
  return http.get<Api<{ items: DockerNetwork[] }>>('/docker/networks')
}

/** create（可带 driver）/ remove / prune */
export function networkAction(name: string, action: string, driver?: string) {
  return http.post<Api<DockerActionResult>>('/docker/network/action', { name, action, driver })
}

// ── Compose ──────────────────────────────────────────────

export function listComposeProjects() {
  return http.get<Api<{ items: DockerComposeProject[] }>>('/docker/compose')
}

/** up / down / start / stop / restart / pull */
export function composeAction(project: string, action: string) {
  return http.post<Api<DockerActionResult>>('/docker/compose/action', { project, action })
}

// ── 容器终端（WebSocket）─────────────────────────────────

/**
 * 后端白名单内的 shell（与 zapd 的 `ALLOWED_SHELLS` 一致）。
 * 传别的后端直接 400——容器里跑什么解释器不能被前端随意指定。
 */
export const DOCKER_SHELLS = ['sh', 'bash', 'ash', 'zsh'] as const

export type DockerShell = (typeof DOCKER_SHELLS)[number]
