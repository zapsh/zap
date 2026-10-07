// SPDX-License-Identifier: AGPL-3.0-only
import axios from 'axios'
import type { AxiosInstance, AxiosRequestConfig } from 'axios'
import { ElMessage } from 'element-plus'
import { getToken, getTokenExpire, removeToken, setToken, setTokenExpire } from './auth'
import { API_BASE, withBase } from './base'
// 非组件环境（拦截器）用全局 t：取调用瞬间的语言，无需响应式
import { t } from '@/i18n'

// 是否正在刷新 token（避免并发刷新）
let isRefreshing = false
let refreshSubscribers: ((token: string) => void)[] = []

function onTokenRefreshed(token: string) {
  refreshSubscribers.forEach((cb) => cb(token))
  refreshSubscribers = []
}

function addRefreshSubscriber(cb: (token: string) => void) {
  refreshSubscribers.push(cb)
}

const service: AxiosInstance = axios.create({
  // 优先用后端注入的前缀（zap.yaml 的 server.url_prefix），开发环境回退 .env
  baseURL: API_BASE || import.meta.env.VITE_API_URL,
  timeout: 15000,
})

// ── 请求拦截器 ──────────────────────────────────────────────
service.interceptors.request.use(
  async (config) => {
    const token = getToken()
    if (token && config.headers) {
      // 检查 token 是否即将过期，提前刷新
      const expire = getTokenExpire()
      if (expire && Date.now() > expire && !isRefreshing) {
        isRefreshing = true
        try {
          const newToken = await refreshToken()
          config.headers['Authorization'] = `Bearer ${newToken}`
          onTokenRefreshed(newToken)
        } catch {
          // 刷新失败：唤醒并发等待的请求（否则这些 Promise 永远 pending），
          // 让它们带着旧 token 继续，由 401 分支统一处理
          config.headers['Authorization'] = `Bearer ${token}`
          onTokenRefreshed(token)
        } finally {
          isRefreshing = false
        }
      } else if (expire && Date.now() > expire && isRefreshing) {
        // 等待正在进行的刷新
        return new Promise((resolve) => {
          addRefreshSubscriber((newToken: string) => {
            config.headers!['Authorization'] = `Bearer ${newToken}`
            resolve(config)
          })
        })
      } else {
        config.headers['Authorization'] = `Bearer ${token}`
      }
    }
    // FormData 请求（上传）让浏览器/axios 自动设置 Content-Type（含 multipart boundary），
    // 手动写 'multipart/form-data' 会导致 boundary 丢失，后端 Multipart 解析失败（400）。
    if (!(config.data instanceof FormData)) {
      config.headers['Content-Type'] = config.headers['Content-Type'] || 'application/json'
    }
    return config
  },
  (error) => Promise.reject(error),
)

// ── 响应拦截器 ──────────────────────────────────────────────
service.interceptors.response.use(
  (response) => {
    const res = response.data
    // 业务错误码：只 reject，由调用方自行处理 UI 提示（错误对象携带 code，供两步验证等场景判断）
    if (res.code !== 0) {
      // 后端 message 优先（业务错误自带文案），缺失时用前端兜底文案
      const err: Error & { code?: number } = new Error(res.message || t('error.system'))
      err.code = res.code
      return Promise.reject(err)
    }
    return res
  },
  async (error) => {
    // HTTP 状态码错误（基础设施级，统一弹窗）
    if (error.response) {
      const { status, data } = error.response

      switch (status) {
        case 401:
          if (!isRefreshing) {
            isRefreshing = true
            try {
              const newToken = await refreshToken()
              onTokenRefreshed(newToken)
              error.config.headers['Authorization'] = `Bearer ${newToken}`
              return service(error.config)
            } catch {
              handleAuthExpired()
            } finally {
              isRefreshing = false
            }
          } else {
            return new Promise((resolve) => {
              addRefreshSubscriber((token: string) => {
                error.config.headers['Authorization'] = `Bearer ${token}`
                resolve(service(error.config))
              })
            })
          }
          break

        case 403:
          ElMessage({
            message: data?.message || t('error.forbidden'),
            type: 'error',
            duration: 5000,
            grouping: true,
          })
          break

        case 404:
          ElMessage({ message: t('error.notFound'), type: 'error', duration: 5000, grouping: true })
          break

        case 500:
          ElMessage({ message: t('error.server'), type: 'error', duration: 5000, grouping: true })
          break

        default:
          // 本地已无凭据却仍请求受保护接口（如会话被清理后的残留请求）：
          // 与 401 同等看待，引导重新登录，而不是悬着一个英文报错
          if (
            status === 400 &&
            data?.message &&
            /Missing credentials|Invalid token/i.test(String(data.message))
          ) {
            handleAuthExpired(t('error.credentialsMissing'))
            break
          }
          ElMessage({
            message: data?.message || t('error.requestFailed', { status }),
            type: 'error',
            duration: 5000,
            grouping: true,
          })
      }
    } else if (error.message?.includes('Network Error')) {
      ElMessage({
        message: t('error.network'),
        type: 'error',
        duration: 5000,
        grouping: true,
      })
    } else if (error.message?.includes('timeout')) {
      ElMessage({ message: t('error.timeout'), type: 'error', duration: 5000, grouping: true })
    }
    // 其他错误（如业务错误 reject 的 Error）不弹窗，由调用方处理

    return Promise.reject(error)
  },
)

// ── Token 刷新 ──────────────────────────────────────────────

/**
 * 认证失效统一处理：清空本地凭据并引导重新登录。
 * 避免反复收到 "Missing credentials / Invalid token" 等英文报错却停留在页面上。
 */
function handleAuthExpired(message = t('error.sessionExpired')) {
  removeToken()
  ElMessage({ message, type: 'error', duration: 5000, grouping: true })
  setTimeout(() => {
    window.location.href = withBase('/login')
  }, 1500)
}

/**
 * 从 JWT 里解析 exp（Unix 秒），拿不到返回 null。
 * 后端刷新接口已返回 expire_in，这里只作兜底。
 */
function tokenExpireFromJwt(token: string): number | null {
  try {
    const payload = token.split('.')[1]
    if (!payload) return null
    const b64 = payload.replace(/-/g, '+').replace(/_/g, '/')
    const json = decodeURIComponent(
      atob(b64)
        .split('')
        .map((c) => '%' + ('00' + c.charCodeAt(0).toString(16)).slice(-2))
        .join(''),
    )
    const exp = JSON.parse(json)?.exp
    return typeof exp === 'number' ? exp : null
  } catch {
    return null
  }
}

async function refreshToken(): Promise<string> {
  // reflash_token 要求携带仍有效的 Bearer 凭据（过期前 60 秒的缓冲窗口内刷新）
  const token = getToken()
  const resp = await axios.post(
    `${API_BASE || import.meta.env.VITE_API_URL}/auth/reflash_token`,
    {},
    {
      headers: {
        'Content-Type': 'application/json',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
    },
  )
  if (resp.data?.access_token) {
    setToken(resp.data.access_token)
    // 必须同步刷新本地过期时间：否则过期时间永远停在登录那一刻，
    // 之后每个请求都会被判定为「已过期」，于是轮询接口（如 /system/status，
    // 每 5 秒一次）每次都伴随一次无意义的 reflash_token。
    const exp = tokenExpireFromJwt(resp.data.access_token)
    const seconds =
      Number(resp.data.expire_in) ||
      (exp ? exp - Math.floor(Date.now() / 1000) : 0)
    if (seconds > 0) setTokenExpire(seconds)
    return resp.data.access_token
  }
  throw new Error(t('error.refreshFailed'))
}

// ── 封装 HTTP 方法 ──────────────────────────────────────────
// axios 1.20+ 返回类型推断为 AxiosResponseResult,与业务约定的 Promise<T>
// (响应拦截器已解包 data)不一致,统一显式断言。
export const http = {
  get<T = any>(url: string, config?: AxiosRequestConfig): Promise<T> {
    return service.get(url, config) as Promise<T>
  },

  post<T = any>(url: string, data?: any, config?: AxiosRequestConfig): Promise<T> {
    return service.post(url, data, config) as Promise<T>
  },

  put<T = any>(url: string, data?: any, config?: AxiosRequestConfig): Promise<T> {
    return service.put(url, data, config) as Promise<T>
  },

  delete<T = any>(url: string, config?: AxiosRequestConfig): Promise<T> {
    return service.delete(url, config) as Promise<T>
  },

  upload<T = any>(url: string, file: File, config?: AxiosRequestConfig): Promise<T> {
    const formData = new FormData()
    formData.append('file', file)
    return service.post(url, formData, config) as Promise<T>
  },

  download(url: string, config?: AxiosRequestConfig): Promise<Blob> {
    return service.get(url, { responseType: 'blob', ...config })
  },
}
