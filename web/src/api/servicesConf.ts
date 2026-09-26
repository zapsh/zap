// 通用服务配置 API（服务配置大类：php / mysql / mariadb / docker，均需管理员）
import { http } from '@/utils/request'

export interface ServiceConfStatus {
  installed?: boolean
  service?: string
  label?: string
  bin?: string
  version?: string
  /** 数据库引擎（仅 MySQL / MariaDB 服务返回：mysql / mariadb），由系统按安装自动识别 */
  engine?: 'mysql' | 'mariadb'
  /** 安装目录（仅 MySQL / MariaDB 服务返回，引擎不同目录不同，如 …/mysql-8.0 或 …/mariadb-10.11） */
  dir?: string | null
  unit?: string | null
  running?: boolean
  systemd?: boolean
  conf_file?: string | null
  conf_dir?: string | null
  main_exists?: boolean
  /** 探测主配置时尝试过的候选路径（用于排查"未检测到配置文件"） */
  conf_candidates?: string[]
}

export interface ServiceConfFile {
  path: string
  rel: string
  name: string
  is_main: boolean
  size: number
  mtime: number
  exists?: boolean
}

export interface ServiceConfListData {
  installed: boolean
  conf_file?: string | null
  conf_dir?: string | null
  main_exists?: boolean
  files: ServiceConfFile[]
}

export interface ServiceConfReadData {
  path: string
  is_main: boolean
  content: string
  size: number
  mtime: number
  missing?: boolean
}

export interface ServiceConfField {
  key: string
  label: string
  /** list：多行文本，一行一项（如 docker 镜像源 registry-mirrors，配置里是数组） */
  kind: 'text' | 'number' | 'select' | 'bool' | 'list'
  help: string
  section?: string | null
  options?: string[]
}

export interface ServiceConfKeysData {
  installed: boolean
  service?: string
  label?: string
  format?: 'ini' | 'json'
  main?: string | null
  main_exists?: boolean
  fields: ServiceConfField[]
  values: Record<string, string | number | boolean | null>
}

export interface ServiceConfResult {
  saved?: boolean
  path?: string
  reason?: string
}

export function getServiceConfStatus(service: string) {
  return http.get<{ code: number; message: string; data: ServiceConfStatus }>(
    '/system/service-conf/status',
    { params: { service } },
  )
}

export function getServiceConfList(service: string) {
  return http.get<{ code: number; message: string; data: ServiceConfListData }>(
    '/system/service-conf/list',
    { params: { service } },
  )
}

export function getServiceConfRead(service: string, path: string) {
  return http.get<{ code: number; message: string; data: ServiceConfReadData }>(
    '/system/service-conf/read',
    { params: { service, path } },
  )
}

export function saveServiceConf(service: string, path: string, content: string) {
  return http.post<{ code: number; message: string; data: ServiceConfResult }>(
    '/system/service-conf/save',
    { service, path, content },
  )
}

export function getServiceConfKeys(service: string) {
  return http.get<{ code: number; message: string; data: ServiceConfKeysData }>(
    '/system/service-conf/keys',
    { params: { service } },
  )
}

export function saveServiceConfKeys(service: string, keys: Record<string, string>) {
  return http.post<{ code: number; message: string; data: ServiceConfResult }>(
    '/system/service-conf/keys/save',
    { service, keys },
  )
}

export function controlServiceConf(service: string, action: string) {
  return http.post<{ code: number; message: string; data: ServiceConfResult }>(
    '/system/service-conf/control',
    { service, action },
  )
}

// ── 多版本实例（php74 / php81 …，目前 php 可用）─────────────────

export interface ServiceConfInstance {
  svc: string
  instance: string
  label?: string
  version: string
  /** 安装目录（如 /usr/local/apps/php-74） */
  dir?: string | null
  installed: boolean
  running: boolean
  bin?: string | null
  unit?: string | null
  systemd?: boolean
  conf_file?: string | null
  main_exists?: boolean
  /** 该实例是否为系统全局默认（/usr/local/bin/php 指向它） */
  is_default: boolean
}

export function getServiceConfInstances(service: string) {
  return http.get<{
    code: number
    message: string
    data: { instances: ServiceConfInstance[]; apps_dir?: string }
  }>('/system/service-conf/instances', { params: { service } })
}

export interface ServiceConfDefaultResult {
  enabled: boolean
  service?: string
  registered?: string[]
  removed?: string[]
}

/** 服务定义清单：来自 yaml（内置 + /etc/zap/services 覆盖），新增服务丢一份 yaml 即可 */
export interface ServiceConfDef {
  key: string
  label: string
  /** ini | json */
  format: string
  exts: string[]
  fields: number
}

export function getServiceConfDefs() {
  return http.get<{ code: number; message: string; data: { items: ServiceConfDef[] } }>(
    '/system/service-conf/defs',
  )
}

/** 设置 / 取消某实例的「全局默认访问」（注册到 /usr/local/bin） */
export function setServiceConfDefault(service: string, enable: boolean) {
  return http.post<{ code: number; message: string; data: ServiceConfDefaultResult }>(
    '/system/service-conf/default',
    { service, enable },
  )
}

// ── PHP 扩展管理（服务配置 → PHP → 扩展）────────────────────

export interface PhpExtItem {
  name: string
  version: string
  enabled: boolean
  /** 编译进解释器的扩展：没有 .so，不能禁用 / 卸载 */
  builtin: boolean
  so: string
  ini: string
  removable: boolean
}

export interface PhpExtListData {
  installed: boolean
  reason?: string
  service?: string
  bin?: string
  version?: string
  extension_dir?: string
  scan_dir?: string
  ini?: string
  /** 安装工具可用性，installer 是后端据此得出的选路结论 */
  tools?: { pie: boolean; pecl: boolean; phpize: boolean; php_config: boolean; gcc: boolean; make: boolean }
  installer?: 'pie' | 'pecl' | 'source' | 'none'
  installer_hint?: string
  extensions: PhpExtItem[]
}

export function getPhpExtList(service: string) {
  return http.get<{ code: number; message: string; data: PhpExtListData }>(
    '/system/service-conf/php-ext/list',
    { params: { service } },
  )
}

export function togglePhpExt(service: string, name: string, enable: boolean) {
  return http.post<{ code: number; message: string; data: { enabled: boolean; reload?: string } }>(
    '/system/service-conf/php-ext/toggle',
    { service, name, enable },
  )
}

/** 安装 / 卸载是长任务（要编译），返回 run_id 供日志抽屉流式查看 */
export interface PhpExtTaskResult {
  task_id: string
  run_id: string
  queued: boolean
  position?: number
}

export function installPhpExt(service: string, package_: string, version?: string) {
  return http.post<{ code: number; message: string; data: PhpExtTaskResult }>(
    '/system/service-conf/php-ext/install',
    { service, package: package_, version: version || '' },
  )
}

export function removePhpExt(service: string, name: string) {
  return http.post<{ code: number; message: string; data: PhpExtTaskResult }>(
    '/system/service-conf/php-ext/remove',
    { service, name },
  )
}
