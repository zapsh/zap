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
  /** 精确引用 `repo:tag`（悬空镜像为 ID）：删除 / 运行都用它，避免误删同一镜像的其它名字 */
  Ref?: string
  /** 同一个镜像 ID 共有几个名字；> 1 说明列表里还有它的别名行 */
  SharedTags?: number
  Digest?: string
  Size: string
  CreatedAt: string
  CreatedSince?: string
  Containers: string
  /** 前端补的唯一行键（同 ID 多 tag 展开后 ID 不再唯一） */
  Key?: string
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

// ── 镜像详情 / 构建 / 快启容器 ──────────────────────────────

/** 构建历史的一层（对应 docker 的一条构建指令） */
export interface DockerImageHistoryItem {
  Id: string
  Created: number
  CreatedAt: string
  CreatedBy: string
  Tags: string
  Size: number
  SizeText: string
  Comment: string
}

/** 镜像摘要（后端从 inspect 里挑出来的常用字段，供 UI 直接绑定） */
export interface DockerImageSummary {
  id: string
  tags: string[]
  size: number
  sizeText: string
  created: string
  architecture: string
  os: string
  dockerVersion: string
  author: string
  comment: string
  cmd: string
  entrypoint: string
  workdir: string
  env: string[]
  exposedPorts: Record<string, unknown>
  labels: Record<string, string>
  layers: number
}

export interface DockerImageInspect {
  /** inspect 原文，UI 按 JSON 展示 / 复制 */
  inspect: Record<string, unknown>
  history: DockerImageHistoryItem[]
  summary: DockerImageSummary
}

export function imageInspect(id: string) {
  return http.get<Api<DockerImageInspect>>('/docker/image/inspect', { params: { id } })
}

/** 构建参数 `--build-arg KEY=VALUE` */
export interface DockerBuildArgPayload {
  key: string
  value: string
}

export interface DockerImageBuildPayload {
  /** 构建上下文目录（绝对路径） */
  context_dir: string
  /** Containerfile 绝对路径；留空由后端在上下文里找 Dockerfile / Containerfile */
  containerfile?: string
  /** 目标镜像名（可带 tag）；非管理员会被后端加上 `<用户名>/` 前缀 */
  name: string
  tags?: string[]
  build_args?: DockerBuildArgPayload[]
  /** 目标平台，如 linux/amd64；空 = 跟随宿主架构 */
  platform?: string
  no_cache?: boolean
  pull?: boolean
}

export interface DockerImageBuildResult {
  run_id: string
  tags: string[]
  log: string
  context_dir: string
  containerfile: string
}

/**
 * 构建镜像（长任务）。
 *
 * 后端登记运行记录后立刻返回 `run_id`，真正干活的是后台 `docker build`；
 * 实时日志复用 appstore 的 WebSocket（`/appstore/ws/{run_id}`）查看。
 */
export function imageBuild(payload: DockerImageBuildPayload) {
  return http.post<Api<DockerImageBuildResult>>('/docker/image/build', payload)
}

/**
 * 从镜像快启容器（等价 `docker run -d`）。
 *
 * `ports` 支持 `[IP:]宿主机端口:容器端口[/udp]`；`name` / `restart` 留空则用 docker 默认值。
 */
export function containerRun(payload: {
  image: string
  name?: string
  ports?: string[]
  restart?: string
}) {
  return http.post<Api<{ id: string; name: string; image: string }>>('/docker/container/run', payload)
}

// ── 数据卷 ───────────────────────────────────────────────

export function listVolumes() {
  return http.get<Api<{ items: DockerVolume[] }>>('/docker/volumes')
}

/**
 * create / remove / prune。
 *
 * `location` 只在 create 时有效：
 * - `home`（默认）：数据落在当前账号家目录下的 `volumes/<name>`，进配额、随 home 备份；
 * - `default`：交给 Docker，落在 `/var/lib/docker/volumes/<name>/_data`，由 daemon 托管。
 */
export function volumeAction(
  name: string,
  action: string,
  location?: 'home' | 'default',
) {
  return http.post<Api<DockerActionResult>>('/docker/volume/action', { name, action, location })
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
