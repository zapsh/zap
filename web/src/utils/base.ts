// SPDX-License-Identifier: AGPL-3.0-only
import { ref } from 'vue'

/// 应用基础路径（URL 前缀）。
///
/// 由后端在 `index.html` 里注入：读取 `zap.yaml` 的 `server.url_prefix`，
/// 形如 `/zap`；未配置前缀时为空串，行为与原来完全一致。
///
/// 用途：
/// - axios `baseURL`（`/zap/api`）
/// - vue-router history base（页面路由 `/zap/dashboard`）
/// - WebSocket 地址（`ws://host/zap/api/terminal/ws/...`）
/// - 登录过期等硬跳转（`window.location.href`）
///
/// 开发环境（vite dev server）没有后端注入，回退到环境变量 `VITE_BASE_URL`，
/// 再回退到空串。
export const BASE: string = readBase()

function readBase(): string {
  const injected = (window as any).__ZAP_BASE__
  if (typeof injected === 'string' && injected.length > 0) {
    return normalize(injected)
  }
  return normalize(import.meta.env.VITE_BASE_URL || '')
}

/** 统一成 `/zap` 或 `''`：去掉首尾斜杠后再补一个前导斜杠 */
function normalize(raw: string): string {
  const trimmed = String(raw || '')
    .trim()
    .replace(/^\/+|\/+$/g, '')
  return trimmed ? `/${trimmed}` : ''
}

/** API 基础路径：`/zap/api`（无前缀时为 `/api`） */
export const API_BASE: string = `${BASE}/api`

/** 给路径加上前缀：`/login` → `/zap/login` */
export function withBase(path: string): string {
  if (!BASE) return path
  return `${BASE}${path.startsWith('/') ? '' : '/'}${path}`
}

// ── 节点视图上下文（多节点主控代理）─────────────────────────────
//
// 这里是节点视图的两个 URL 收敛点之一（apiBase / wsUrl）：
// nodeContext 激活后，HTTP 与 WS 地址统一指向 `/api/nodes/{id}/**`
// 主控白名单代理，业务代码零改动。

export interface NodeContext {
  id: number
  name: string
  /**
   * 节点自己的面板地址（含 url_prefix，如 `https://10.0.0.5:2663/zap`）。
   * 仅用于「必须由外部直接访问」的链接（插件回调 URL 等）——这类地址的调用方
   * 没有主控会话，走 `/api/nodes/{id}/…` 代理会被 401 挡掉。
   */
  baseUrl?: string
}

const NODE_CTX_KEY = 'zap_node_context'

function readNodeContext(): NodeContext | null {
  try {
    const raw = sessionStorage.getItem(NODE_CTX_KEY)
    if (!raw) return null
    const v = JSON.parse(raw)
    if (typeof v?.id !== 'number') return null
    return {
      id: v.id,
      name: String(v.name ?? ''),
      baseUrl: typeof v.baseUrl === 'string' && v.baseUrl ? v.baseUrl : undefined,
    }
  } catch {
    return null
  }
}

/**
 * 当前查看的受管节点；null = 本机。
 * 存 sessionStorage：新标签页 / 重开浏览器自然回到本机，
 * 避免用户忘了退出而把误操作打到远端节点上。
 */
export const nodeContext = ref<NodeContext | null>(readNodeContext())

export function setNodeContext(ctx: NodeContext | null) {
  nodeContext.value = ctx
  if (ctx) sessionStorage.setItem(NODE_CTX_KEY, JSON.stringify(ctx))
  else sessionStorage.removeItem(NODE_CTX_KEY)
}

/** 节点代理路径段：`/nodes/3`；本机视图为空串 */
function nodeSegment(): string {
  return nodeContext.value ? `/nodes/${nodeContext.value.id}` : ''
}

/** API 基础路径（节点感知）：`/zap/api` 或 `/zap/api/nodes/3` */
export function apiBase(): string {
  return `${API_BASE}${nodeSegment()}`
}

/**
 * 指向「当前这台机器自己」的完整 API 地址，**不经代理**。
 *
 * 给需要被外部系统直接调用的链接用（插件回调 URL、节点上的定时器）：接管状态下
 * 页面展示的是节点的数据，这类地址必须落在节点自己的面板上，而不是主控的代理路径
 * ——外部调用方没有主控会话，走代理只会拿到 401。
 *
 * 注意 `base_url` 已含 url 前缀但不含 `/api`（后端 node_url 拼的也是 `{base}/api/…`），
 * 所以这里接 `/api` 而不是 API_BASE，否则带前缀的部署会出现 `/zap/zap/api`。
 */
export function nodeSelfApiBase(): string {
  const raw = nodeContext.value?.baseUrl
  if (raw) return `${raw.replace(/\/+$/, '')}/api`
  return `${window.location.origin}${API_BASE}`
}

// ── 节点视图下的请求归属 ──────────────────────────────────────
//
// 「接管整台机器」= 除下面几类外，所有请求（含写操作与 WebSocket）都改走主控的节点代理。
// 分类口径必须与后端 `zapd/src/zap/node.rs::PROXY_DENY_ROOTS` 一致：
// 两边任一处改动都要同步，否则前端把请求送到代理、后端再拒掉，
// 用户看到的是莫名其妙的 403。

/**
 * 主控自有接口：只有这几类即使在节点视图里也仍打主控。
 *
 * - `/auth`：登录 / 登出 / 刷新 / 验证码。会话属于主控，节点没有这个会话，
 *   代理过去等于拿远端身份去刷新本机会话；
 * - `/node`、`/nodes`：主控的节点台账与跨节点曲线（`/nodes/{id}/history`），
 *   代理过去就是 A→B→A 的递归代理；
 * - `/dev`：API Token 控制台，管的是「谁在调用这个面板」；
 * - `/docs`、`/health`：主控自己的文档与探针，两台机器上是同一份。
 *
 * 刻意**不含** `/user` 与 `/system/menus`：接管要「像直接登录那台机器」，
 * 身份（roles / permissions）和菜单树必须来自节点，前端才会按远端 Token
 * 归属账号的权限渲染菜单与按钮，而不是「主控 admin 看得见按钮、点下去节点报 403」。
 * 进入 / 退出节点视图都整页重载，就是为了让这两份数据重新拉一次。
 */
const MASTER_OWNED = ['/auth', '/node', '/nodes', '/dev', '/docs', '/health']

/**
 * 节点视图下这条 API 的去向：`master` 仍打主控，`proxy` 经 `/api/nodes/{id}` 转发。
 * 本机视图返回 null（不参与判定）。传入的是 axios 的相对路径（不含 baseURL）。
 */
export type NodeViewRoute = 'master' | 'proxy'

export function nodeViewRoute(url?: string): NodeViewRoute | null {
  if (nodeContext.value == null) return null
  const u = (url || '').split('?')[0]
  return MASTER_OWNED.some((p) => u === p || u.startsWith(`${p}/`)) ? 'master' : 'proxy'
}

/**
 * 切换接管目标时用的**整页跳转地址**：把 SPA 路由解析成带 history 前缀的完整 href，
 * 交给 `window.location.href`。
 *
 * 必须整页重载而不是 `router.push`：接管后的身份（`/user/info`）与菜单
 * （`/system/menus`）现在都来自节点，而这两份数据在应用启动时只拉一次。
 * SPA 内跳路由会沿用主控的菜单，用户看到的仍是主控的权限。
 */
export function nodeViewPath(
  router: { resolve: (to: string) => { href: string } },
  target = '/dashboard',
): string {
  return router.resolve(target).href
}

/** 构造同前缀下的 WebSocket 地址（相对路径 WebSocket 在部分场景下不可靠，这里拼全 URL） */
export function wsUrl(path: string): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const seg = nodeSegment()
  if (seg) {
    // path 形如 `/api/terminal/ws/...`，插入节点段变成 `/api/nodes/3/terminal/ws/...`，
    // 由主控与节点之间再建一条上游 WS 并双向转发（见 routers/node.rs 的升级分支）。
    const rest = path.startsWith('/api') ? path.slice(4) : path
    return `${proto}//${window.location.host}${API_BASE}${seg}${rest.startsWith('/') ? '' : '/'}${rest}`
  }
  return `${proto}//${window.location.host}${withBase(path)}`
}
