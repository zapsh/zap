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
}

const NODE_CTX_KEY = 'zap_node_context'

function readNodeContext(): NodeContext | null {
  try {
    const raw = sessionStorage.getItem(NODE_CTX_KEY)
    if (!raw) return null
    const v = JSON.parse(raw)
    return typeof v?.id === 'number' ? { id: v.id, name: String(v.name ?? '') } : null
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

/** 构造同前缀下的 WebSocket 地址（相对路径 WebSocket 在部分场景下不可靠，这里拼全 URL） */
export function wsUrl(path: string): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const seg = nodeSegment()
  if (seg) {
    // path 形如 `/api/terminal/ws/...`，插入节点段变成 `/api/nodes/3/terminal/ws/...`。
    // 主控当前会显式拒绝 WS 升级（远端终端 / 实时日志在后续版本开放），
    // 失败要发生在「远端」而不是悄悄连回本机。
    const rest = path.startsWith('/api') ? path.slice(4) : path
    return `${proto}//${window.location.host}${API_BASE}${seg}${rest.startsWith('/') ? '' : '/'}${rest}`
  }
  return `${proto}//${window.location.host}${withBase(path)}`
}
