import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

/**
 * 获取菜单树s
 */
export async function getSystemInfo() {
  return http.get<ApiResponse>('/system/info')
}


/**
 * 服务器状态（含 CPU / 内存 / 负载 / 网络历史曲线数据）。
 *
 * @param range `live`（默认，近 5 分钟原始点 + 前端高频轮询）或
 *              `1h / 6h / 24h / 7d / 30d`（后端按时间桶降采样，~120-180 点）
 */
export async function getRTStatus(params?: { range?: string }) {
  return http.get<ApiResponse>('/system/status', { params })
}

export async function getSystemOverview() {
  return http.get<ApiResponse>('/system/overview')
}

export async function getSystemAbout() {
  return http.get<ApiResponse>('/system/about')
}

/** 仪表盘统计卡片：按当前角色可见范围返回用户 / 站点 / 数据库数量 */
export interface DashboardCounts {
  users: number
  sites: number
  databases: number
}

export async function getDashboardCounts() {
  return http.get<ApiResponse<DashboardCounts>>('/dashboard/counts')
}