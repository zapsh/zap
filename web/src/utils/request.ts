// SPDX-License-Identifier: AGPL-3.0-only
import axios from 'axios'
import type { AxiosInstance, AxiosRequestConfig } from 'axios'
import { ElMessage } from 'element-plus'
import { getToken, getTokenExpire, removeToken, setToken, setTokenExpire } from './auth'
import { API_BASE, apiBase, nodeContext, nodeViewRoute, setNodeContext, withBase } from './base'
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

/**
 * 后端错误文案本地化：优先用 key 命中前端 locale；key 缺失（未翻译 / 老接口无 key）
 * 时回退到后端的中文 message。避免 key 漏翻时把 key 字符串直接显示给用户。
 */
function terr(key: string | undefined, fallback: string): string {
  if (!key) return fallback
  const s = t(key)
  return s && s !== key ? s : fallback
}

const service: AxiosInstance = axios.create({
  // 优先用后端注入的前缀（zap.yaml 的 server.url_prefix），开发环境回退 .env
  baseURL: API_BASE || import.meta.env.VITE_API_URL,
  timeout: 15000,
})

// ── 节点视图：请求改走主控代理 /api/nodes/{id}/** ──────────────
//
// 分类口径必须与后端 `zapd/src/zap/node.rs`（PROXY_DENY_ROOTS）对齐：
// 这里决定请求发到哪，那里决定代理放不放行。两边任一处漂移，
// 用户看到的就是莫名其妙的 403。

/**
 * 这条错误响应**确实由节点应答**，而不是主控在同一条代理 URL 上自己发的。
 *
 * 只看 URL 判不开：接管状态下 `/api/nodes/{id}/user/info` 的 401，既可能是节点
 * 拒绝了它的 API Token，也可能是主控拒绝了这个浏览器会话（代理路由本身挂在主控
 * 门禁内）——两者的正确处置完全相反：前者只能去「节点管理」换凭据，后者必须照常
 * 刷新 / 重新登录。把后者误判成前者，用户就会卡在登录页外反复 401 出不来。
 * 主控代理只给真正由节点回来的响应打 `x-zap-node-proxy` 头，以此为准。
 */
function fromNodeProxy(error: any): boolean {
  return Boolean(error?.response?.headers?.['x-zap-node-proxy'])
}

// ── 请求拦截器 ──────────────────────────────────────────────
service.interceptors.request.use(
  async (config) => {
    if (nodeViewRoute(config.url) === 'proxy') {
      // 除主控自有面外全部转给代理，**写操作与 WebSocket 也一样**。
      // 「能不能执行」不在前端判：节点按它自己的权限矩阵校验转发过去的
      // API Token，主控只负责「绝不落到本机」与「记审计」。
      // 前端不再按方法名猜一遍读写，免得两处口径漂移。
      config.baseURL = apiBase() || import.meta.env.VITE_API_URL
      // 跨节点多一跳 RTT，远端执行本身也更慢：接管状态下把超时下限放宽到 60s。
      // 已显式设置更长超时的调用（上传 / 导出 / 镜像构建）保持原值不变。
      config.timeout = Math.max(config.timeout ?? 0, 60_000)
    }
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
      const err: Error & { code?: number } = new Error(
        terr(res.key, res.message) || t('error.system'),
      )
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
          // 由节点应答的 401 = 那台机器的 API Token 失效（被删 / 过期）：
          // 与本机会话无关，绝不能走本地刷新 / 登出流程。
          // 反之主控在这条代理 URL 上发的 401 是**本机会话**过期，必须照常
          // 刷新——否则受控端一改 Token，用户就会被永久挡在登录页外。
          if (fromNodeProxy(error)) {
            ElMessage({
              message: terr(data?.key, data?.message) || t('cluster.remoteAuthFailed'),
              type: 'error',
              duration: 5000,
              grouping: true,
            })
            break
          }
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
          // 403 有三种来源，下一步动作完全不同，必须分开说：
          // ① 主控没代理这个接口（x-zap-node-denied: master_owned）；
          // ② 主控账号没有 node:edit，压根没资格接管（perm）；
          // ③ 远端节点拒绝——是那个节点 API Token 的权限不够，与主控角色无关。
          const denied = error.response?.headers?.['x-zap-node-denied']
          if (denied) {
            ElMessage({
              // 'master_owned' 与旧版主控统一发的 'policy' 都归为「这个接口不经代理」
              message:
                denied === 'perm' ? t('cluster.nodeNeedEdit') : t('cluster.nodeBlockedWrite'),
              type: 'warning',
              duration: 5000,
              grouping: true,
            })
            break
          }
          if (fromNodeProxy(error)) {
            ElMessage({
              message: t('cluster.remoteForbiddenFailed', {
                name: nodeContext.value?.name || '',
                detail: terr(data?.key, data?.message) || t('error.forbidden'),
              }),
              type: 'error',
              duration: 5000,
              grouping: true,
            })
            break
          }
          ElMessage({
            message: terr(data?.key, data?.message) || t('error.forbidden'),
            type: 'error',
            duration: 5000,
            grouping: true,
          })
          break

        case 404:
          // 节点视图下的 404 几乎都是「节点版本比面板旧、没有这个接口」，
          // 说清是哪台机器，否则用户会以为主控坏了。
          ElMessage({
            message: fromNodeProxy(error)
              ? t('cluster.remoteNotFoundFailed', { name: nodeContext.value?.name || '' })
              : t('error.notFound'),
            type: 'error',
            duration: 5000,
            grouping: true,
          })
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
            message: terr(data?.key, data?.message) || t('error.requestFailed', { status }),
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
 *
 * 幂等：一次页面会话内只触发一次重定向。多个并发请求（401 + 缺凭据的 400）
 * 都会走到这里，若不幂等会各自 setTimeout 调度一次 `window.location.href`，
 * 导致登录页渲染后又被第二次赋值触发额外整页刷新（表现为「显示登录页后又刷新一下」）。
 */
let authExpiredHandled = false
function handleAuthExpired(message = t('error.sessionExpired')) {
  if (authExpiredHandled) return
  authExpiredHandled = true
  // 接管态依附于主控会话：会话都没了，「正在操作那台机器」也就无从谈起。
  // 不清的话，重新登录时 `/user/info`、`/system/menus` 仍会走节点代理，
  // 受控端一旦换了 Token 就变成「登录成功 → 401 → 踢回登录页」的死循环。
  setNodeContext(null)
  removeToken()
  ElMessage({ message, type: 'error', duration: 5000, grouping: true })
  setTimeout(() => {
    const loginPath = withBase('/login')
    // 已在登录页（理论上本函数就是靠这次重载进入的）则不再重载，避免额外刷新
    if (window.location.pathname !== loginPath) {
      window.location.href = loginPath
    }
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
    const seconds = Number(resp.data.expire_in) || (exp ? exp - Math.floor(Date.now() / 1000) : 0)
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
