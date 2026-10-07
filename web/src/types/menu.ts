// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 菜单项接口
 */
export interface MenuItem {
  id: string
  parentId: string
  name: string
  path: string
  component: string
  redirect?: string
  meta: {
    title: string
    icon?: string
    hidden?: boolean
    roles?: string[]
    keepAlive?: boolean
    affix?: boolean
  }
  children?: MenuItem[]
  order: number
  type: 'dir' | 'menu' | 'button' // dir: 目录, menu: 菜单, button: 按钮
  status: 0 | 1 // 0: 禁用, 1: 启用
  createTime?: string
  updateTime?: string
}

/**
 * 菜单表单接口
 */
export interface MenuForm {
  parentId: string
  name: string
  path: string
  component: string
  redirect?: string
  meta: {
    title: string
    icon?: string
    hidden?: boolean
    roles?: string[]
    keepAlive?: boolean
    affix?: boolean
  }
  order: number
  type: 'dir' | 'menu' | 'button'
  status: 0 | 1
}
