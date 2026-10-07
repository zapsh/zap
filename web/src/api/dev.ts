// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

export interface ApiTokenItem {
  id: number
  name: string
  prefix: string
  status: number
  expires_at: number
  last_used_at: number
  created_at: number
}

/** 创建 API Token（data.token 仅本次返回） */
export interface ApiTokenCreated {
  id: number
  token: string
  prefix: string
  expires_at: number
  created_at: number
}

export function getApiTokenList() {
  return http.get<ApiResponse<ApiTokenItem[]>>('/dev/api-token/list')
}

export function createApiToken(data: { name?: string; expire_days?: number }) {
  return http.post<ApiResponse<ApiTokenCreated>>('/dev/api-token/create', data)
}

export function updateApiToken(data: { id: number; name?: string; status?: number }) {
  return http.post<ApiResponse>('/dev/api-token/update', data)
}

export function deleteApiToken(id: number) {
  return http.post<ApiResponse>('/dev/api-token/delete', { id })
}

// ── API 文档 ────────────────────────────────────────────────

export interface ApiDocParam {
  name: string
  type: string
  required: boolean
  desc: string
}

export interface ApiDocEndpoint {
  method: string
  path: string
  summary: string
  params?: ApiDocParam[]
  note?: string
}

export interface ApiDocGroup {
  name: string
  description: string
  endpoints: ApiDocEndpoint[]
}

export interface ApiDocsData {
  title: string
  version: string
  base_path: string
  auth_intro: string[]
  groups: ApiDocGroup[]
}

export function getApiDocs() {
  return http.get<ApiResponse<ApiDocsData>>('/dev/api-docs')
}
