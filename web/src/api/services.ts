// 服务配置 → 总览：应用商店已安装应用中「登记了 systemd 服务」的实例（均需管理员）
import { http } from '@/utils/request'

/** 运行状态：与 zapexec 的 normalize_state 一致 */
export type ServiceState = 'running' | 'stopped' | 'starting' | 'stopping' | 'failed' | 'unknown'

export interface ServiceItem {
  /** 稳定主键 `<category>/<name>@<instance>` */
  key: string
  /** 包名（appstore 目录名，如 nginx / php / mysql） */
  pkg: string
  /** 应用显示名 */
  name: string
  version: string
  category: string
  /** 实例名（多实例应用如 php 会重复出现） */
  instance: string
  /** systemd unit 名（启停 / 自启动都用它） */
  svc: string
  state: ServiceState
  /** 是否开机自启 */
  enabled: boolean
  /** unit 是否已注册（未注册说明装了但服务没登记好） */
  exists: boolean
}

export interface ServicesOverview {
  items: ServiceItem[]
  /** 平台是否支持 systemd（false 时整页降级为「需手动操作」） */
  systemd: boolean
}

export const getServicesOverview = () =>
  http.get<{ code: number; message: string; data: ServicesOverview }>('/system/services/overview')

export const controlService = (svc: string, action: 'start' | 'stop' | 'restart' | 'reload') =>
  http.post('/system/services/control', { svc, action })

export const setServiceBoot = (svc: string, enable: boolean) =>
  http.post('/system/services/boot', { svc, enable })
