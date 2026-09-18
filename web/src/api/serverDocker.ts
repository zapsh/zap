// Docker 服务配置专用 API（daemon.json：状态 / 关键项 / 文件编辑 / 服务控制）。
//
// Docker 的配置是 JSON（存在嵌套对象与数组），与 php.ini / my.cnf 的 INI 关键项形态差异较大，
// 页面因此独立于通用服务配置组件；这里把通用 service-conf 通道（service=docker）封装成
// docker 语义的接口，页面不再直接依赖 servicesConf.ts。
import { http } from '@/utils/request'

/** Docker 服务运行状态 */
export interface DockerStatus {
  installed?: boolean
  service?: string
  label?: string
  bin?: string
  version?: string
  unit?: string | null
  running?: boolean
  systemd?: boolean
  conf_file?: string | null
  conf_dir?: string | null
  main_exists?: boolean
  /** 探测主配置时尝试过的候选路径 */
  conf_candidates?: string[]
}

/** daemon.json 关键项定义 */
export interface DockerField {
  key: string
  label: string
  /** list：配置里是数组，表单里一行一项（镜像源 / 私有仓库 / DNS / exec-opts） */
  kind: 'text' | 'number' | 'select' | 'bool' | 'list'
  help: string
  options?: string[]
}

export interface DockerKeysData {
  installed: boolean
  service?: string
  label?: string
  format?: 'ini' | 'json'
  main?: string | null
  main_exists?: boolean
  fields: DockerField[]
  values: Record<string, string | number | boolean | null>
}

export interface DockerConfFile {
  path: string
  rel: string
  name: string
  is_main: boolean
  size: number
  mtime: number
  exists?: boolean
}

export interface DockerConfListData {
  installed: boolean
  conf_file?: string | null
  conf_dir?: string | null
  main_exists?: boolean
  files: DockerConfFile[]
}

export interface DockerConfContent {
  path: string
  is_main: boolean
  content: string
  size: number
  mtime: number
  /** 文件在磁盘上不存在（尚未创建） */
  missing?: boolean
}

export interface DockerResult {
  saved?: boolean
  path?: string
  reason?: string
}

/** 服务状态：版本 / 运行态 / daemon.json 路径 */
export function getDockerStatus() {
  return http.get<{ code: number; message: string; data: DockerStatus }>(
    '/system/service-conf/status',
    { params: { service: 'docker' } },
  )
}

/** 关键项表单定义与当前值 */
export function getDockerKeys() {
  return http.get<{ code: number; message: string; data: DockerKeysData }>(
    '/system/service-conf/keys',
    { params: { service: 'docker' } },
  )
}

/** 保存关键项（列表字段传换行分隔文本，留空表示删除该键） */
export function saveDockerKeys(keys: Record<string, string>) {
  return http.post<{ code: number; message: string; data: DockerResult }>(
    '/system/service-conf/keys/save',
    { service: 'docker', keys },
  )
}

/** 可编辑配置文件（/etc/docker 下的 json） */
export function listDockerConfs() {
  return http.get<{ code: number; message: string; data: DockerConfListData }>(
    '/system/service-conf/list',
    { params: { service: 'docker' } },
  )
}

export function readDockerConf(path: string) {
  return http.get<{ code: number; message: string; data: DockerConfContent }>(
    '/system/service-conf/read',
    { params: { service: 'docker', path } },
  )
}

export function saveDockerConf(path: string, content: string) {
  return http.post<{ code: number; message: string; data: DockerResult }>(
    '/system/service-conf/save',
    { service: 'docker', path, content },
  )
}

/** 服务控制：start / restart（dockerd 无 reload，改 daemon.json 只能重启） */
export function controlDocker(action: 'start' | 'restart') {
  return http.post<{ code: number; message: string; data: DockerResult }>(
    '/system/service-conf/control',
    { service: 'docker', action },
  )
}
