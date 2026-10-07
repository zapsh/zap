// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

// ── 站内信（通知中心）────────────────────────────────────────

export interface NoticeMessage {
  id: number
  /** 通知类型（事件名）：login_success / password_change 等 */
  type: string
  title: string
  body: string
  /** 0=未读 1=已读 */
  is_read: number
  created_at: number
}

export interface NoticeListData {
  list: NoticeMessage[]
  total: number
  unread_count: number
}

/** 分页获取本人站内信（附未读数） */
export function getNotices(params?: { page?: number; page_size?: number }) {
  return http.get<ApiResponse<NoticeListData>>('/user/notices', { params })
}

/** 未读数（顶栏铃铛轮询） */
export function getUnreadCount() {
  return http.get<ApiResponse<{ unread: number }>>('/user/notices/unread')
}

/** 标记单条已读 */
export function readNotice(id: number) {
  return http.post<ApiResponse>('/user/notices/read', { id })
}

/** 全部标记已读 */
export function readAllNotices() {
  return http.post<ApiResponse>('/user/notices/read_all')
}

/** 删除消息（批量） */
export function deleteNotices(ids: number[]) {
  return http.post<ApiResponse>('/user/notices/delete', { ids })
}
