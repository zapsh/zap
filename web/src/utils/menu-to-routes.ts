import type { RouteRecordRaw } from 'vue-router'
import type { MenuItem } from '@/types/menu'

// 预加载所有视图组件
const modules = import.meta.glob('../views/**/*.vue')

/**
 * 动态加载组件
 * 组件文件不存在时返回 null（例如数据库里残留了已删除页面的菜单记录），
 * 由调用方跳过该菜单，避免中断整个动态路由注册。
 * @param component 组件路径
 */
function loadComponent(component: string) {
  // 处理布局组件
  if (component === 'Layout') {
    return () => import('@/layout/index.vue')
  }

  // 处理其他组件
  // 1. 移除开头的斜杠
  const path = component.replace(/^\//, '')
  // 2. 确保组件路径以 .vue 结尾
  const componentPath = path.endsWith('.vue') ? path : `${path}.vue`
  // 3. 构造完整的组件路径
  const fullPath = `../views/${componentPath}`

  // 检查组件是否存在
  if (!modules[fullPath]) {
    console.error(`组件不存在: ${fullPath}`)
    return null
  }

  return modules[fullPath]
}

/**
 * 将MenuItem转换为RouteRecordRaw
 * 菜单无效（缺 name / 组件不存在）时返回 null，由调用方跳过
 * @param menuItem 菜单项
 */
export function menuToRoute(menuItem: MenuItem): RouteRecordRaw | null {
  // 确保基础属性存在
  if (!menuItem.name) {
    console.error('菜单项必须包含name属性，已跳过:', menuItem)
    return null
  }
  // 兼容空 path：子菜单使用 'index' 作为默认路径
  const itemPath = menuItem.path || 'index'
  const component = menuItem.component ? loadComponent(menuItem.component) : undefined
  if (menuItem.component && !component) {
    // 组件未找到：整条菜单不注册
    return null
  }
  const route: RouteRecordRaw = {
    path: itemPath,
    name: menuItem.name,
    meta: {
      title: menuItem.meta?.title || '',
      icon: menuItem.meta?.icon,
      roles: menuItem.meta?.roles,
      // hidden 有两个来源：后端下发的 meta.hidden（迁移走的入口，如「已安装应用」）
      // 与 status=0（禁用）。两者都要保留，否则子菜单会把该隐藏的入口照样画出来。
      hidden: menuItem.meta?.hidden === true || menuItem.status === 0,
    },
    // 明确设置可能的属性
    component,
    redirect: menuItem.redirect,
    children: [],
  }

  // 处理子菜单
  if (menuItem.children?.length) {
    route.children = menuItem.children
      .filter((child) => child.type !== 'button')
      .map((child) => menuToRoute(child))
      .filter((child): child is RouteRecordRaw => child !== null)
  }

  // 清理未定义的属性
  if (!route.component) delete route.component
  if (!route.redirect) delete route.redirect

  return route
}

/**
 * 将菜单树转换为路由配置（自动跳过无效菜单）
 * @param menuTree 菜单树
 */
export function menuTreeToRoutes(menuTree: MenuItem[]): RouteRecordRaw[] {
  return menuTree
    .filter((menu) => menu.type !== 'button') // 过滤掉按钮类型
    .filter((menu) => menu.status !== 0) // 过滤掉禁用的菜单
    .filter((menu) => !menu.meta.hidden) // 过滤掉隐藏的菜单
    .map((menu) => menuToRoute(menu))
    .filter((route): route is RouteRecordRaw => route !== null)
}
