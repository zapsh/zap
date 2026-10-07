// SPDX-License-Identifier: AGPL-3.0-only
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
  const trimmed = String(raw || '').trim().replace(/^\/+|\/+$/g, '')
  return trimmed ? `/${trimmed}` : ''
}

/** API 基础路径：`/zap/api`（无前缀时为 `/api`） */
export const API_BASE: string = `${BASE}/api`

/** 给路径加上前缀：`/login` → `/zap/login` */
export function withBase(path: string): string {
  if (!BASE) return path
  return `${BASE}${path.startsWith('/') ? '' : '/'}${path}`
}

/** 构造同前缀下的 WebSocket 地址（相对路径 WebSocket 在部分场景下不可靠，这里拼全 URL） */
export function wsUrl(path: string): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${window.location.host}${withBase(path)}`
}
