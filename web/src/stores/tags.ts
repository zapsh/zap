/**
 * 顶部导航标签（Nav Bar 的标签页）。
 *
 * 几条规则，对应参考的那套顶部导航：
 *
 * 1. **只记一级 / 二级**：二级页的标签一律是「一级 › 二级」；两级标题同名时只写一次
 *    （「站点」「终端」这类），免得出现「站点 › 站点」。分类的默认落点往往就是某个具体
 *    子页（服务器状态的落点就是 Server Monitor），只写一级会让同一分类下的标签长得
 *    一模一样、认不出点的是哪个，所以落点页同样带二级标题。
 *    页内 nav pill、`?tab=` 这类同页切换不产生新标签（path 没变）。
 * 2. **每个分类只留一个「当前页」标签**：切换一级菜单时，把上一分类的「当前页」标签关掉；
 *    同一分类内再点别的二级菜单不新增标签，而是就地改写这唯一的标签 —— 一级标题不动，
 *    只换二级标题 / 图标 / path。所以普通页面在标签栏里始终只有一个。
 * 3. **保持存活的页面标签不关**：`ALIVE_PAGES`（终端 / 文件管理）里的页面各占一个独立标签，
 *    不会被上一条的就地改写吃掉，也不随切分类清除，一直留在标签栏（见 `NavTab.alive`），
 *    直到用户点 X 关闭 —— 关闭时才把它的缓存从 keep-alive 白名单里摘掉。
 *    于是标签栏通常是「仪表盘 + 存活的页面 + 当前页」。
 * 4. **只有仪表盘常驻**：只显示图标、不可关闭，任何分类下都在；标签全关光时它就是兜底。
 *    这里刻意不看 `meta.affix`：后端菜单表几乎给所有页面都标了 affix（历史遗留），
 *    真按它来标签会全部关不掉。
 * 5. **keep-alive 白名单**：`include` 按**组件 name** 匹配（不是路由 name —— 菜单下发的
 *    name 是 `files-index` 这种），所以常规页面其实命中不了，切页即重建。只有
 *    `ALIVE_PAGES` 里那几个页面显式声明了组件 name：只要它的标签还开着就挂在白名单上，
 *    切走再回来实例还在（文件管理的当前目录、终端的 SSH 会话都不用重来）；标签被关掉，
 *    白名单里也就没了，下次进来重新开始。
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

/**
 * 保持存活的页面：打开后标签常驻，切页、切分类都不销毁实例，切回来状态还在。
 *
 * key = 页面路径，value = **组件 name**（keep-alive 的 include 按组件 name 匹配，
 * 所以对应页面里要显式写 `export default { name: ... }`）。
 * 这些页面的状态丢了就得重来（重新选目录、重连 SSH），所以它们各占一个不会被就地改写的标签，
 * keep-alive 白名单也只在它的标签还开着时挂上，用户点 X 关掉标签即释放缓存。
 */
const ALIVE_PAGES: Record<string, string> = {
  '/files/index': 'FileManager',
  '/terminal/index': 'Terminal',
}

export interface NavTab {
  /** 唯一键，也是跳转目标：不含 query，同一页面带不同 query 视作同一个标签 */
  path: string
  /**
   * 路由 name，同时进 keep-alive 白名单。
   *
   * 注意：include 实际按**组件 name** 匹配，菜单下发的 name（`files-index` 这种）
   * 对不上，所以常规页面切页即重建；真正常驻的见 `ALIVE_PAGES`。
   */
  name?: string
  /** 标题原文（i18n key 或后端下发的中文），渲染时过 translateTitle */
  title: string
  /** 二级标签的父级标题，渲染成「一级 › 二级」 */
  parentTitle?: string
  icon?: string
  /** 所属主分类（`matched[0].path`），建标签时记下；存活页面标签跨分类保留 */
  group: string
  /** 是否可关闭：仅常驻标签为 false */
  closable: boolean
  /**
   * 是否为「保持存活」页面的标签（终端 / 文件管理）。
   *
   * 这类标签不会被自动关掉（切分类、切别的二级页都留着），只有用户点 X 才关闭；
   * 关掉的同时它的 keep-alive 缓存也随之释放。
   */
  alive?: boolean
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

  /** 刷新页面时临时摘出白名单的组件 name，见 suspendCache */
  const suspended = ref<string[]>([])

  /**
   * keep-alive 白名单：当前打开着的标签，加上存活页面标签对应的组件 name，
   * 再减掉刷新时临时摘掉的那个。
   *
   * 存活页面的缓存跟它的标签绑在一起：标签开着，白名单里就在；点 X 关掉标签，
   * 白名单里也就没了，缓存随之释放。
   */
  const cachedNames = computed(() => {
    const names = new Set<string>()
    tabs.value.forEach((t) => {
      if (t.name) names.add(t.name)
      // 存活页面：路由 name（菜单下发的 `files-index` 这种）和组件 name 对不上，
      // 所以显式补上组件 name，keep-alive 才认得出。
      const alive = ALIVE_PAGES[t.path]
      if (alive) names.add(alive)
    })
    suspended.value.forEach((n) => names.delete(n))
    return [...names]
  })

  /**
   * 刷新页面用：把某个页面的缓存实例从白名单里摘掉，返回「放回去」的函数。
   *
   * 常驻存活的页面必须走这套：直接跳 `/redirect` 会被 keep-alive 复用缓存实例，
   * 刷新点了等于没点。调用方顺序（见 TagsView.refresh）：
   * 摘掉 → 等 KeepAlive 销毁实例 → 跳中转页（老实例这时才真正卸载）→ 放回去
   * （放早了老实例会被重新缓存，放晚了新实例就不会进缓存）。
   */
  function suspendCache(path: string): () => void {
    const name = ALIVE_PAGES[path] ?? tabs.value.find((t) => t.path === path)?.name
    if (!name) return () => {}
    suspended.value = [...suspended.value, name]
    return () => {
      suspended.value = suspended.value.filter((n) => n !== name)
    }
  }

  /**
   * 按当前路由同步标签。
   *
   * - 换主分类：关掉上一分类的「当前页」标签，但保留仪表盘与存活页面的标签；
   * - 存活页面（终端 / 文件管理）：各占一个标签，不参与就地改写，只由用户 X 关闭；
   * - 普通页面：始终复用同一个「当前页」标签，就地改写路径 / 标题 / 图标。
   *
   * 已存在的标签只是激活切换，不改动。由布局层在每次导航后调用，是标签栏唯一的入口。
   */
  function sync(route: RouteLocationNormalizedLoaded) {
    const path = route.path
    // `/redirect/xxx` 是「刷新当前页」的中转路由，会立刻 replace 回去，不该留下标签
    if (path.startsWith('/redirect')) return

    const first = route.matched[0]
    const currentGroup = first?.path || path

    if (currentGroup !== group.value) {
      // 换主分类：上一分类的「当前页」标签关掉，但仪表盘和存活页面的标签都留着
      tabs.value = tabs.value.filter((t) => !t.closable || t.alive)
      group.value = currentGroup
    }

    // 已经开着这个标签了：只是激活切换，不改动
    if (tabs.value.some((t) => t.path === path)) return

    const isDashboard = path === DASHBOARD_PATH
    const isAlive = !isDashboard && !!ALIVE_PAGES[path]
    const titles = tabTitles((first?.meta?.title as string) || '', (route.meta.title as string) || '')
    const name = typeof route.name === 'string' ? route.name : undefined
    const icon = isDashboard ? DASHBOARD_ICON : (route.meta.icon as string | undefined)

    // 存活页面：独立标签，不参与就地改写，一直留在标签栏直到用户点 X 关闭
    if (isAlive) {
      tabs.value.push({
        path,
        name,
        title: titles.title,
        parentTitle: titles.parentTitle,
        icon,
        group: currentGroup,
        closable: true,
        alive: true,
      })
      sortTabs()
      return
    }

    // 普通页面：只留一个「当前页」标签，再点别的二级菜单时就地改写它，
    // 一级标题（parentTitle）保持不变，只变二级标题、图标和 path。
    const current = tabs.value.find((t) => t.closable && !t.alive)
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
      alive: false,
    })

    sortTabs()
  }

  /** 常驻标签永远排在最前（仪表盘在最左） */
  function sortTabs() {
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
    suspendCache,
    close,
    closeOthers,
    closeAll,
    reset,
  }
})
