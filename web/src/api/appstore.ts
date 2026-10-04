import { http } from '@/utils/request'
import { getToken } from '@/utils/auth'
import { wsUrl } from '@/utils/base'

export interface RepoSource {
  id: string
  name: string
  url: string
  builtin: boolean
  enabled: boolean
  version: string
  commit: string
  updated_at: number
  exists: boolean
}

/** 单个安装/升级选项（app.yaml options） */
export interface AppOption {
  /** 选项名；同时作为脚本环境变量名（建议全大写，如 MODULES） */
  name: string
  /** 展示标题，缺省用 name */
  label?: string
  /** string | number | bool | select | multiselect，缺省 string */
  type?: 'string' | 'number' | 'bool' | 'select' | 'multiselect'
  /** 默认值：标量或字符串（多选可传数组） */
  default?: unknown
  /** 必填（多选=至少选一个） */
  required?: boolean
  placeholder?: string
  desc?: string
  /** select/multiselect 候选；项可为字符串或 {label,value} */
  choices?: Array<string | { label: string; value: string }>
  /** 多选值拼接字符，缺省空格（脚本用 $NAME 展开时勿含空格于值中） */
  separator?: string
}

/** 某动作键下的一组选项：值可写为选项数组，或对象 { items, intro }。
 * intro：整组介绍 / 说明，Web 端展示在选项表单最下方（不参与提交与 env 注入）。 */
export interface OptionsGroup {
  items: AppOption[]
  intro?: string
}

export interface AppChoice {
  label: string
  value: string
}

/**
 * version_meta 中单个版本的元数据。任意包可声明（后端原样透传）：
 * - family：家族标识（如 mysql / mariadb / openresty…）。同包多家族时，版本下拉按它分组、
 *   且禁止跨家族直接升级（须先卸载再装目标家族版本）；
 * - label：家族短名（选项内标签 / 卡片选中态标签，如 MySQL、MariaDB），缺省回退 family 原文；
 * - group：家族下拉分组标题（如 MySQL Community Server），缺省回退 label / family。
 * 展示名无任何前端内置映射，全部由 version_meta 提供。
 */
export interface VersionMeta {
  family?: string
  label?: string
  group?: string
  [key: string]: unknown
}

export interface AppPackage {
  pkg_path: string
  category: string
  name: string
  title: string
  description: string
  version: string
  /** 全部可安装版本（来自 app.yaml version 数组），默认取首个 */
  versions: string[]
  /** 版本 → 附加元数据（app.yaml version_meta，如 MySQL / MariaDB 合并入口） */
  version_meta?: Record<string, VersionMeta> | null
  deps: string[]
  /** 依赖：映射（name: 版本/要求）或旧式数组 */
  dependencies: Record<string, string> | string[]
  /** 自定义操作按钮：动作键 -> 按钮文案（如 build: 编译安装） */
  actions: Record<string, string>
  /** 安装/升级可选项：动作键 -> 选项列表或 {items,intro}；顶层数组 = install 动作；缺省动作键时使用 install 键 */
  options?: Record<string, AppOption[] | OptionsGroup> | AppOption[]
  /** 是否允许多实例安装（已安装仍可安装其他版本） */
  allow_multiple_instances: boolean
  default_port: number | null
  scripts: any
  /** 是否提供升级脚本（无则升级按「卸载 → 安装」兜底执行，前端需提示数据备份） */
  has_upgrade?: boolean
  /** 可浏览/安装此包的角色白名单（app.yaml roles）；空/缺省 = 所有角色开放 */
  roles?: string[]
  source: 'official' | 'custom'
  repo_id?: string
  installed: boolean
  installed_version?: string
  installed_source?: string
  installed_at?: number
  upgraded_from?: string | null
  /** 全部已安装实例（多版本 / 多站点各一项）；操作按实例定位，不能只认包名 */
  installed_instances?: Array<{
    instance?: string
    instance_key?: string
    version?: string
    owner?: string | null
    site_id?: string | number | null
  }>
}

export interface RunItem {
  run_id: string
  action: string
  pkg: string
  username: string
  status: string
  exit_code: number
  log_path: string
  started_at: number
  finished_at: number
}

// ── Git 源（多源）───────────────────────────────────────────

export const getRepos = () => http.get<any>('/appstore/repos')

export const addRepo = (data: { name: string; url: string }) =>
  http.post<any>('/appstore/repos/add', data)

export const removeRepo = (data: { id: string }) =>
  http.post<any>('/appstore/repos/remove', data)

export const updateRepo = (data: { id: string }) =>
  http.post<any>('/appstore/repos/update', data)

// ── 包 ──────────────────────────────────────────────────────

export const getPackages = () => http.get<any>('/appstore/packages')

/** 表单归一后的选项：选项名 -> 字符串值（多选以 separator 拼接） */
export type FormOptions = Record<string, string>

export const installPackage = (data: {
  pkg_path: string
  source: string
  repo_id?: string
  version: string
  /** 用户点击的动作键（app.yaml actions），随请求透传到 shell 环境变量 ACTION */
  action?: string
  /** 安装表单选项 */
  options?: FormOptions
}) => http.post<any>('/appstore/install', data)

export const uninstallPackage = (data: {
  pkg_path: string
  /** 实例名（缺省 default；站点类 `site:<id>`）：多实例时指明卸掉哪一个 */
  instance?: string
  /** 卸载表单选项 */
  options?: FormOptions
}) => http.post<any>('/appstore/uninstall', data)

export const upgradePackage = (data: {
  pkg_path: string
  source: string
  repo_id?: string
  version: string
  /** 实例名（缺省 default；站点类 `site:<id>`）：多实例时指明升级哪一个 */
  instance?: string
  /** 用户点击的动作键（app.yaml actions），随请求透传到 shell 环境变量 ACTION */
  action?: string
  /** 升级表单选项 */
  options?: FormOptions
}) => http.post<any>('/appstore/upgrade', data)

// ── 脚本 ────────────────────────────────────────────────────

export const getScriptsTree = () => http.get<any>('/appstore/scripts/tree')

export const readScript = (path: string) =>
  http.get<any>('/appstore/script/read', { params: { path } })

export const writeScript = (data: { path: string; content: string }) =>
  http.post<any>('/appstore/script/write', data)

/** 删除自定义脚本或目录（仅限 scripts/ 下，脚本根目录不可删） */
export const deleteScript = (data: { path: string }) =>
  http.post<any>('/appstore/script/delete', data)

export const runScript = (data: { path: string }) => http.post<any>('/appstore/script/run', data)

export const stopScript = (data: { run_id: string }) => http.post<any>('/appstore/script/stop', data)

// ── 运行记录 / 日志 ─────────────────────────────────────────

export const getRuns = (params: { page?: number; page_size?: number }) =>
  http.get<any>('/appstore/runs', { params })

export const getRunLog = (runId: string, offset = 0) =>
  http.get<any>(`/appstore/log/${runId}`, { params: { offset } })

// ── 运行快照（失败后查看/编辑脚本并重跑）──────────────────────

export interface RunFileItem {
  path: string
  size: number
}

/** 列出一次运行的可编辑脚本快照文件树 */
export const getRunFiles = (runId: string) =>
  http.get<any>('/appstore/run/files', { params: { run_id: runId } })

/** 读取运行快照内文件内容 */
export const readRunFile = (runId: string, path: string) =>
  http.get<any>('/appstore/run/file/read', { params: { run_id: runId, path } })

/** 写运行快照内文件（修改脚本，仅管理员） */
export const writeRunFile = (data: { run_id: string; path: string; content: string }) =>
  http.post<any>('/appstore/run/file/write', data)

/** 重跑一次失败的运行（复用其快照，仅管理员） */
export const retryRun = (runId: string) =>
  http.post<any>('/appstore/run/retry', { run_id: runId })

export function wsLogUrl(runId: string): string {
  return wsUrl(`/api/appstore/ws/${runId}?token=${getToken()}`)
}

// ── 已安装应用（实例管理）───────────────────────────────────

export interface InstalledApp {
  pkg_path: string
  name: string
  version: string
  category: string
  source: 'official' | 'custom'
  repo_id?: string
  installed_at?: number
  upgraded_from?: string | null
  run_id?: string
  /** 实例标识（默认同包名，php74/php85 之类由脚本在 info.yaml 登记） */
  instance: string
  /** 实例的稳定主键 `<category>/<name>@<instance>`：多实例时 pkg_path 不唯一，定位请用这个 */
  instance_key?: string
  /** 归属面板用户（站点类应用有）：非管理员据此与其他用户的站点应用隔离 */
  owner?: string | null
  /** 站点 id（站点类应用有） */
  site_id?: string | number | null
  /** 动态状态：running / stopped / failed / starting / stopping / unknown */
  state: string
  /** 脚本登记的实例信息：install_dir / expose / port / pid_file / config_file / svc_name 等 */
  info: Record<string, any>
}

export const getInstalledApps = () => http.get<any>('/appstore/installed')

export const instanceAction = (data: {
  pkg_path: string
  /** 实例名（缺省 default）：多实例时指明操作哪一个 */
  instance?: string
  action: string
}) => http.post<any>('/appstore/instance/action', data)
