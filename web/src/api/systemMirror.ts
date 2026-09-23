import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

/** 内置源预设：国内 / Cloudflare */
export interface MirrorPreset {
  label: string
  value: string
}

export interface MirrorData {
  /** 当前生效的下载源：http(s) 镜像，或 file:// 开头的本地目录 */
  pkgMirror: string
  updatedAt: number
  /** 配置文件路径（{data}/mirror.yaml），排障时给人看 */
  path: string
  /** true = 本地目录源（离线环境） */
  isLocal: boolean
  presets: MirrorPreset[]
}

/** 读取下载源配置（系统设置 → 下载源，仅 admin） */
export function getMirror() {
  return http.get<ApiResponse<MirrorData>>('/system/config/mirror')
}

/**
 * 保存下载源。
 *
 * 后端会校验：http(s) 地址，或**已存在**的本地目录（绝对路径 / file:// 前缀）。
 * 改完对**下一次**应用商店安装生效，正在跑的任务不受影响。
 */
export function saveMirror(pkgMirror: string) {
  return http.post<ApiResponse<{ pkgMirror: string }>>('/system/config/mirror', {
    pkg_mirror: pkgMirror,
  })
}
