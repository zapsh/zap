import { http } from '@/utils/request'

/** 自动探测层：os / hostname / webserver / php / databases / tools */
export interface EnvPayload {
  os: {
    id: string
    name: string
    version: string
    arch: string
    kernel: string
  }
  hostname: string
  webserver: {
    flavor: 'nginx' | 'openresty' | 'none' | string
    version: string
    binary: string
    conf: string
    sites_dir: string
    running: boolean
  }
  php: {
    default: string
    instances: Array<{
      version: string
      binary: string
      socket: string
      running: boolean
      default: boolean
    }>
  }
  databases: Array<{ name: string; version: string; binary: string; running: boolean }>
  /** Node.js 运行时（fnm 全局管理） */
  nodejs?: {
    fnm: boolean
    fnm_path: string
    fnm_version: string
    fnm_dir: string
    default: string
    /** /usr/local/bin/node 软链是否存在（其它用户直接可用） */
    global_link: boolean
    versions: Array<{
      version: string
      path: string
      source: 'fnm' | 'system'
      installed: boolean
    }>
  }
  /** Python 运行时（uv 管理的 + 系统自带的解释器） */
  python?: {
    uv: boolean
    uv_path: string
    uv_version: string
    /** 当前生效的 PyPI 源 */
    index_url: string
    versions: Array<{ version: string; path: string; source: 'uv' | 'system' }>
  }
  tools: Array<{ name: string; version: string }>
}

/** conf 层：面板默认配置（管理员维护） */
export interface EnvConf {
  webserver: string
  php_default: string
  /** 全局默认 Python 版本（uv 管理），应用部署未指定版本时用它 */
  python_default: string
  /** 全局默认 Node 版本（fnm 管理），应用部署未指定版本时用它 */
  node_default: string
  /** 下载镜像：official（默认）/ china */
  download_mirror: string
  database: string
  /** PHP-FPM 默认 pool 规格（JSON 字符串；用户未自定义时的兜底） */
  fpm_pool_defaults: string
  /** 用户家目录默认挂载点（如 /home /home2），新建用户时 home_dir 前缀 */
  user_home_root: string
  /** 容器运行时：auto（跟随探测）/ docker / podman */
  container_runtime: string
}

export interface EnvData {
  payload: EnvPayload | null
  conf: EnvConf
  /** 探测时间（unix 秒） */
  detected_at: number
  /** 本次请求是否触发过自动刷新 */
  refreshed: boolean
  /** 探测失败原因（有缓存时展示） */
  error: string | null
}

export interface EnvDefaultsPayload {
  webserver?: string
  php_default?: string
  /** 全局默认 Python 版本 */
  python_default?: string
  /** 全局默认 Node 版本 */
  node_default?: string
  /** 下载镜像：official（默认）/ china */
  download_mirror?: string
  database?: string
  fpm_pool_defaults?: string
  user_home_root?: string
  /** 容器运行时：auto（跟随探测）/ docker / podman */
  container_runtime?: string
}

export const getServerEnv = () =>
  http.get<{ code: number; message: string; data: EnvData }>('/system/env')

export const refreshServerEnv = () =>
  http.post<{ code: number; message: string; data: EnvData }>('/system/env/refresh')

export const saveServerEnvDefaults = (data: EnvDefaultsPayload) =>
  http.post<{ code: number; message: string; data: EnvConf }>('/system/env/defaults', data)

// ── Python / Node.js 运行时 ──────────────────────────────

/**
 * 安装运行时可能要下载几十 MB，前端默认 15s 超时会直接弹「请求超时」，
 * 这里单独放宽到 10 分钟。
 */
const LONG_TIMEOUT = { timeout: 10 * 60 * 1000 }

/** 一键安装 uv（装到 /usr/local/bin，所有用户可用） */
export const installEnvUv = () => http.post('/system/env/python/uv', {}, LONG_TIMEOUT)

/** 切换 PyPI 源（系统级 uv + pip 配置）；传官方地址即回到官方源 */
export const setPythonIndex = (index_url: string) =>
  http.post('/system/env/python/index', { index_url }, LONG_TIMEOUT)

/** 安装 / 卸载一个 Python 版本（uv） */
export const installPythonVersion = (version: string) =>
  http.post('/system/env/python/install', { version }, LONG_TIMEOUT)

export const removePythonVersion = (version: string) =>
  http.post('/system/env/python/remove', { version }, LONG_TIMEOUT)

/** 一键安装 fnm（装到 /usr/local/fnm，所有用户可用） */
export const installEnvFnm = (mirror?: string) =>
  http.post('/system/env/nodejs/fnm', { mirror: mirror || '' }, LONG_TIMEOUT)

/** Node 版本操作：install | default | uninstall */
export const nodejsAction = (action: string, version: string, mirror?: string) =>
  http.post(
    '/system/env/nodejs/action',
    { action, version, mirror: mirror || '' },
    LONG_TIMEOUT,
  )

/** 设置 npm registry（写 /etc/npmrc，所有用户生效） */
export const setNodeRegistry = (registry: string) =>
  http.post('/system/env/nodejs/registry', { registry }, LONG_TIMEOUT)

// ── PHP-FPM 规格模板库（admin 维护） ─────────────────────────

/** FPM 规格模板项 */
export interface FpmSpecItem {
  id: number
  /**
   * 模板名：以 `{用户名}_` 开头视为归该用户名下（其名下客户可选用 / 可被继承）；
   * 其它名字为全局通用模板（所有人添加用户时都可见可选）。
   * 归某用户名下的默认模板建议命名为 `{用户名}_default`。
   */
  name: string
  /** 规格 JSON 字符串（与 fpm_pool_defaults 同字段集，覆盖于全局默认之上） */
  spec: string
  remark: string
  /** 归属用户名；null = 全局通用 */
  owner: string | null
  created_at: number
  updated_at: number
}

/** 模板列表（admin 全量；reseller 仅自己名下 + 全局通用） */
export const getFpmSpecs = () =>
  http.get<{ code: number; message: string; data: FpmSpecItem[] }>('/system/fpm-specs/list')

/** 新增模板（admin） */
export const addFpmSpec = (data: { name: string; spec: string; remark?: string }) =>
  http.post<{ code: number; message: string; data: { id: number } }>('/system/fpm-specs/add', data)

/** 修改模板（admin） */
export const updateFpmSpec = (data: {
  id: number
  name?: string
  spec?: string
  remark?: string
}) => http.post<{ code: number; message: string }>('/system/fpm-specs/update', data)

/** 删除模板（admin） */
export const deleteFpmSpec = (id: number) =>
  http.post<{ code: number; message: string }>('/system/fpm-specs/delete', { id })
