// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

export interface RoleItem {
  id: number
  name: string
  role_key: string
  description: string
  status: number
  created_at: number
  updated_at: number
}

export interface RoleListResponse {
  code: number
  data: RoleItem[]
  total: number
}

export function getRoleList() {
  return http.get<RoleListResponse>('/system/role/list')
}

export function createRole(data: { name: string; role_key: string; description?: string }) {
  return http.post<ApiResponse<{ id: number }>>('/system/role/add', data)
}

export function updateRole(data: {
  id: number
  name?: string
  role_key?: string
  description?: string
  status?: number
}) {
  return http.post<ApiResponse>('/system/role/update', data)
}

export function deleteRole(id: number) {
  return http.post<ApiResponse>('/system/role/delete', { id })
}

export interface RolePermissions {
  /** 菜单可见性（仅前端渲染，不是安全边界） */
  menu_ids: number[]
  /** 动作级权限点：`{ns}:view` / `{ns}:edit`，请求级鉴权依据 */
  permissions: string[]
}

export function getRolePermissions(roleId: number) {
  return http.get<ApiResponse<RolePermissions>>('/system/role/permissions', {
    params: { role_id: roleId },
  })
}

export function setRolePermissions(roleId: number, menuIds: number[], permissions: string[]) {
  return http.post<ApiResponse>('/system/role/permissions/set', {
    role_id: roleId,
    menu_ids: menuIds,
    permissions,
  })
}

export interface PermActionItem {
  key: string
  label: string
}

export interface PermGroupItem {
  ns: string
  label: string
  actions: PermActionItem[]
}

/** 权限点目录：与后端 access 权限矩阵同源，勾选即可生效 */
export function getPermissionCatalog() {
  return http.get<ApiResponse<{ groups: PermGroupItem[] }>>('/system/role/permission-catalog')
}
