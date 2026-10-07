// SPDX-License-Identifier: AGPL-3.0-only
const TOKEN_KEY = 'Zap-Admin-Token'

/**
 * 使用 sessionStorage 替代 localStorage。
 * sessionStorage 在关闭标签页时自动清除，降低 XSS 令牌泄露风险。
 */

export function getToken(): string {
  return sessionStorage.getItem(TOKEN_KEY) || ''
}

export function setToken(token: string): void {
  sessionStorage.setItem(TOKEN_KEY, token)
}

export function setTokenExpire(expire: number): void {
  // 提前 60 秒过期，给刷新留出缓冲
  const expireTime = Date.now() + expire * 1000 - 60_000
  sessionStorage.setItem(`${TOKEN_KEY}-expire`, expireTime.toString())
}

export function getTokenExpire(): number | null {
  const expire = sessionStorage.getItem(`${TOKEN_KEY}-expire`)
  return expire ? Number(expire) : null
}

export function removeToken(): void {
  sessionStorage.removeItem(TOKEN_KEY)
  sessionStorage.removeItem(`${TOKEN_KEY}-expire`)
}
