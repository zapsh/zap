/**
 * 顶部导航标签（Nav Bar 的标签页）。
 *
 * 三条规则，对应参考的那套顶部导航：
 *
 * 1. **只记一级 / 二级**：一级菜单落到的是「分类入口」标签（显示一级标题），二级页是
 *    「一级 › 二级」标签。页内 nav pill、`?tab=` 这类同页切换不产生新标签（path 没变）。
 * 2. **按主分类隔离**：切换一级菜单时，把上一分类的标签整组关掉，只留常驻的仪表盘。
 *    否则逛一圈回来，标签栏会攒下几十个互不相干的页面。
 * 3. **只有仪表盘常驻**：只显示图标、不可关闭，任何分类下都在；标签全关光时它就是兜底。
 *    这里刻意不看 `meta.affix`：后端菜单表几乎给所有页面都标了 affix（历史遗留），
 *    真按它来标签会全部关不掉。
 *
 * 状态只活在内存里：刷新页面回到「仪表盘 + 当前页」，不做跨会话持久化。
 */
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import type { RouteLocationNormalizedLoaded, RouteRecordNormalized } from 'vue-router'

/** 常驻标签：仪表盘 */
export const DASHBOARD_PATH = '/dashboard'
/** 仪表盘标题用 i18n key，渲染时由 translateTitle 处理 */
const DASHBOARD_TITLE = 'menu.dashboard'
const DASHBOARD_ICON = 'material-symbols:home'
const DASHBOARD_NAME = 'Dashboard'

export interface NavTab {
  /** 唯一键，也是跳转目标：不含 query，同一页面带不同 query 视作同一个标签 */
  path: string
  /** 路由 name：keep-alive 的 include 用它 */
  name?: string
  /** 标题原文（i18n key 或后端下发的中文），渲染时过 translateTitle */
  title: string
  /** 二级标签的父级标题，渲染成「一级 › 二级」 */
  parentTitle?: string
  icon?: string
  /** 所属主分类（`matched[0].path`），切换分类时整组替换 */
  group: string
  /** 是否可关闭：仅常驻标签为 false */
  closable: boolean
}

function dashboardTab(): NavTab {
  return {
    path: DASHBOARD_PATH,
    name: DASHBOARD_NAME,
    title: DASHBOARD_TITLE,
    icon: DASHBOARD_ICON,
    group: '/',
    closable: false,
  }
}

/**
 * 主分类的默认落点（一级菜单的 redirect）。
 *
 * 拿不到、或它指向别的分类（如 `/docs` 把旧链接重定向到 `/system/about`）时返回空串，
 * 此时调用方按「当前页就是入口标签」处理，不再凭空造一个标签。
 */
function entryPathOf(first: RouteRecordNormalized | undefined, group: string): string {
  const target = first?.redirect
  if (typeof target !== 'string' || !target) return ''
  if (group === '/' || target === group || target.startsWith(`${group}/`)) return target
  return ''
}

export const useTagsStore = defineStore('tags', () => {
  const tabs = ref<NavTab[]>([dashboardTab()])
  /** 当前主分类（`matched[0].path`）；空串表示还没进过任何分类 */
  const group = ref('')

  /** keep-alive 白名单：关闭标签即丢弃对应页面缓存 */
  const cachedNames = computed(() =>
    tabs.value.map((t) => t.name).filter((n): n is string => !!n),
  )

  /**
   * 按当前路由同步标签：换分类先清空，再追加（已存在则为激活切换，不改动）。
   *
   * 由布局层在每次导航后调用，是标签栏唯一的入口。
   */
  function sync(route: RouteLocationNormalizedLoaded) {
    const path = route.path
    // `/redirect/xxx` 是「刷新当前页」的中转路由，会立刻 replace 回去，不该留下标签
    if (path.startsWith('/redirect')) return

    const first = route.matched[0]
    const currentGroup = first?.path || path

    if (currentGroup !== group.value) {
      // 换主分类：上一分类的标签整组关掉，只留常驻标签
      tabs.value = tabs.value.filter((t) => !t.closable)
      group.value = currentGroup
    }

    if (tabs.value.some((t) => t.path === path)) return

    const isDashboard = path === DASHBOARD_PATH
    const ownTitle = (route.meta.title as string) || ''
    const groupTitle = (first?.meta?.title as string) || ''
    // 命中分类默认落点的页 = 分类入口标签，显示一级标题；其余二级页显示自身标题
    const entryPath = entryPathOf(first, currentGroup)
    const isEntry = isDashboard || (!!entryPath && path === entryPath)

    tabs.value.push({
      path,
      name: typeof route.name === 'string' ? route.name : undefined,
      title: isDashboard ? DASHBOARD_TITLE : isEntry ? groupTitle || ownTitle : ownTitle,
      parentTitle: !isEntry && groupTitle ? groupTitle : undefined,
      icon: isDashboard ? DASHBOARD_ICON : (route.meta.icon as string | undefined),
      group: currentGroup,
      closable: !isDashboard,
    })

    // 常驻标签永远排在最前（仪表盘在最左）
    tabs.value.sort((a, b) => Number(a.closable) - Number(b.closable))
  }

  /**
   * 关闭标签。
   *
   * 关掉的若是当前页，返回应该跳转过去的目标（相邻标签 → 常驻标签），
   * 由调用方执行跳转 —— store 不碰 router，免得和路由守卫绕成循环依赖。
   */
  function close(path: string, activePath: string): string | null {
    const idx = tabs.value.findIndex((t) => t.path === path)
    if (idx < 0 || !tabs.value[idx].closable) return null

    tabs.value.splice(idx, 1)
    if (path !== activePath) return null

    const next = tabs.value[idx] || tabs.value[idx - 1] || tabs.value[0]
    return next ? next.path : DASHBOARD_PATH
  }

  function closeOthers(path: string) {
    tabs.value = tabs.value.filter((t) => !t.closable || t.path === path)
  }

  /** 关闭所有可关标签，返回兜底跳转目标 */
  function closeAll(): string {
    tabs.value = tabs.value.filter((t) => !t.closable)
    return DASHBOARD_PATH
  }

  /** 登出 / 换账号：回到只有常驻标签的初始态 */
  function reset() {
    tabs.value = [dashboardTab()]
    group.value = ''
  }

  return {
    tabs,
    group,
    cachedNames,
    sync,
    close,
    closeOthers,
    closeAll,
    reset,
  }
})
