// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'

export interface AuditLogItem {
  id: number
  user_id: number
  username: string
  action: string
  target: string
  detail: string
  ip: string
  created_at: number
}

export interface AuditListResponse {
  code: number
  data: AuditLogItem[]
  total: number
}

/** 查询审计日志（分页，按操作 / 用户模糊过滤，仅 admin） */
export function getAuditLogList(params?: {
  page?: number
  page_size?: number
  action?: string
  username?: string
}) {
  return http.get<AuditListResponse>('/system/audit/list', { params })
}
