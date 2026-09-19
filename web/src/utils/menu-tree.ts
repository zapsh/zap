/** 后端菜单树节点（GET /system/menus/list 的返回结构） */
export interface MenuNode {
  id: number
  name: string
  type?: string
  status?: number
  meta?: { title?: string; hidden?: boolean; roles?: string[] }
  children?: MenuNode[]
}

/**
 * 把不可授予的菜单节点置灰：树形结构照常展示，但不能勾选。
 *
 * 适用于「父账号给成员分菜单」的场景 —— 只能把自己已经可见的菜单分出去
 * （范围由后端 `available_ids` 给出）。真正的拦截在后端：越权的 id 会被直接丢弃，
 * 这里置灰只是操作提示。
 */
export function disableUnavailable(nodes: MenuNode[], allowed: number[]): MenuNode[] {
  const set = new Set(allowed)
  return nodes.map((n) => {
    const children = n.children?.length ? disableUnavailable(n.children, allowed) : undefined
    return {
      ...n,
      disabled: !set.has(n.id),
      ...(children ? { children } : {}),
    }
  })
}
