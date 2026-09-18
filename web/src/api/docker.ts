import { http } from '@/utils/request'

// ── 数据模型 ─────────────────────────────────────────────
// 以下字段直接来自 `docker ... --format '{{json .}}'` 输出，首字母大写是 docker 的规范，
// 后端原样透传（避免二次映射导致版本兼容问题）。

/** 容器行（`docker container ls`） */
export interface DockerContainer {
  ID: string
  Image: string
  Names: string
  State: string
  Status: string
  Ports: string
  CreatedAt: string
  RunningFor: string
  Size: string
  Labels: string
  /** 后端从 compose label 解析出的项目名称（可能为空串） */
  project: string
}

/** 资源快照行（`docker stats --no-stream`） */
export interface DockerStat {
  ID: string
  Name: string
  CPUPerc: string
  MemUsage: string
  MemPerc: string
  NetIO: string
  BlockIO: string
  PIDs: string
}

/** 镜像行（`docker image ls`） */
export interface DockerImage {
  ID: string
  Repository: string
  Tag: string
  Digest: string
  Size: string
  CreatedAt: string
  CreatedSince: string
  Containers: string
}

/** 数据卷行（`docker volume ls`） */
export interface DockerVolume {
  Name: string
  Driver: string
  Mountpoint: string
  Scope: string
  Labels: string
}

/** 网络行（`docker network ls`） */
export interface DockerNetwork {
  ID: string
  Name: string
  Driver: string
  Scope: string
  IPv6: string
  Internal: string
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
