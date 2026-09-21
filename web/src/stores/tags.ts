/**
 * 顶部导航标签（Nav Bar 的标签页）。
 *
 * 三条规则，对应参考的那套顶部导航：
 *
 * 1. **只记一级 / 二级**：二级页的标签一律是「一级 › 二级」；两级标题同名时只写一次
 *    （「站点」「终端」这类），免得出现「站点 › 站点」。分类的默认落点往往就是某个具体
 *    子页（服务器状态的落点就是 Server Monitor），只写一级会让同一分类下的标签长得
 *    一模一样、认不出点的是哪个，所以落点页同样带二级标题。
 *    页内 nav pill、`?tab=` 这类同页切换不产生新标签（path 没变）。
 * 2. **按主分类隔离，且分类内只留一个**：切换一级菜单时，把上一分类的标签整组关掉，
 *    只留常驻的仪表盘；同一分类内再点别的二级菜单不新增标签，而是就地改写这唯一的标签
 *    —— 一级标题不动，只换二级标题 / 图标 / path。标签栏因此最多是「仪表盘 + 当前页」。
 * 3. **只有仪表盘常驻**：只显示图标、不可关闭，任何分类下都在；标签全关光时它就是兜底。
 *    这里刻意不看 `meta.affix`：后端菜单表几乎给所有页面都标了 affix（历史遗留），
 *    真按它来标签会全部关不掉。
 *
 * 状态只活在内存里：刷新页面回到「仪表盘 + 当前页」，不做跨会话持久化。
 */
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import type { RouteLocationNormalizedLoaded } from 'vue-router'

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
 * 标签标题：一级（父）标题 + 二级（自身）标题。
 *
 * 同级同名时只留一份（`站点 › 站点` 这种纯噪声），自身标题缺失时退回一级标题。
 */
function tabTitles(groupTitle: string, ownTitle: string): { title: string; parentTitle?: string } {
  if (!ownTitle) return { title: groupTitle }
  if (!groupTitle || groupTitle === ownTitle) return { title: ownTitle }
  return { title: ownTitle, parentTitle: groupTitle }
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
   * 按当前路由同步标签：换分类先清空；分类内换页就地改写那唯一的标签；
   * 其它情况（首次进分类、回仪表盘）才追加，已存在则只是激活切换。
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
    const titles = tabTitles((first?.meta?.title as string) || '', (route.meta.title as string) || '')
    const name = typeof route.name === 'string' ? route.name : undefined
    const icon = isDashboard ? DASHBOARD_ICON : (route.meta.icon as string | undefined)

    // 分类内只留一个标签：再点别的二级菜单时不新增，直接把现有标签改成新页面，
    // 一级标题（parentTitle）保持不变，只变二级标题、图标和 path。
    const current = tabs.value.find((t) => t.closable && t.group === currentGroup)
    if (!isDashboard && current) {
      current.path = path
      current.name = name
      current.title = titles.title
      current.parentTitle = titles.parentTitle
      current.icon = icon
      return
    }

    tabs.value.push({
      path,
      name,
      title: isDashboard ? DASHBOARD_TITLE : titles.title,
      parentTitle: isDashboard ? undefined : titles.parentTitle,
      icon,
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
