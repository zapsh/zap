import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'
import type { MenuItem as MenuTreeNode } from '@/types/menu'

export interface MenuItem {
  id: number
  name: string
  path: string
  component: string
  redirect?: string
  type: string
  meta: {
    title: string
    icon?: string
    hidden?: boolean
    keepAlive?: boolean
    affix?: boolean
    roles?: string[]
  }
  children?: MenuItem[]
  order: number
  status: number
}

export interface MenuForm {
  parent_id?: number
  name: string
  path: string
  component?: string
  redirect?: string
  type?: string
  title?: string
  icon?: string
  hidden?: number
  keep_alive?: number
  affix?: number
  roles?: string
  sort_order?: number
  status?: number
}

/** Get menu tree for sidebar rendering */
export function getMenuTree() {
  return http.get<ApiResponse<MenuTreeNode[]>>('/system/menus/tree')
}

/**
 * 可见菜单集合的指纹。
 *
 * 侧栏依赖的**环境能力**会变（例如后台刚装上 Docker），但菜单只在登录时拉一次，
 * 于是拿这个轻量指纹做比对：变了就重建菜单，用户不必手动刷新浏览器。
 */
export function getMenusRevision() {
  return http.get<ApiResponse<{ revision: string; total: number }>>('/system/menus/revision')
}

/** Get menu tree for admin management */
export function getMenuList() {
  return http.get<ApiResponse<MenuItem[]>>('/system/menus/list')
}

export function createMenu(data: MenuForm) {
  return http.post<ApiResponse<{ id: number }>>('/system/menus/add', data)
}

export function updateMenu(data: { id: number } & Partial<MenuForm>) {
  return http.post<ApiResponse>('/system/menus/update', data)
}

export function deleteMenu(id: number) {
  return http.post<ApiResponse>('/system/menus/delete', { id })
}

export function toggleMenuStatus(id: number, status: number) {
  return http.post<ApiResponse>('/system/menus/status', { id, status })
}
