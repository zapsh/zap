// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 角色状态类型
 */
export type RoleStatus = '0' | '1'

/**
 * 角色对象接口
 */
export interface Role {
  id: number | string
  roleName: string
  roleKey: string
  description?: string
  status: RoleStatus
  createTime?: string
  updateTime?: string
}

/**
 * 角色表单接口
 */
export interface RoleForm {
  roleName: string
  roleKey: string
  description?: string
  status: RoleStatus
}

/**
 * 角色查询参数接口
 */
export interface RoleQuery {
  roleName?: string
  status?: RoleStatus
  pageNum?: number
  pageSize?: number
}

/**
 * 角色列表响应接口
 */
export interface RoleListResult {
  total: number
  list: Role[]
}

/**
 * 角色权限设置参数接口
 */
export interface RolePermissionParams {
  roleId: number | string
  menuIds: (number | string)[]
}
