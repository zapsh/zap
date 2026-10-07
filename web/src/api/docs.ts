// SPDX-License-Identifier: AGPL-3.0-only
// 「文档」菜单 API。
//
// 后端返回 HTML 片段（不是业务 JSON 包），所以 `getDocHtml` 走
// `responseType: 'text'` + `transformResponse` 自填 `{code:0, data}`
// 让统一响应拦截器把它当业务成功放过 —— 鉴权头、token 刷新都跟着
// `service` 走，调用方无需关心。

import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

// 后端 docs_list 返回的条目
export interface DocMeta {
  id: string
  /** 兜底中文标题（前端 i18n 优先） */
  title_zh: string
  /** 兜底英文标题 */
  title_en: string
}

// 注意：http 实例已带 baseURL（`/zap/api` 或 `/api`，见 utils/base.ts），
// 这里只写 `/docs/...`，不要再拼 `/api`，否则会请求到 `/api/api/docs/...`
// 落到 SPA fallback，拿到 index.html（HTTP 200 但没有文档内容）。

/** 列出所有可用文档（白名单 + 已放置的 md 文件） */
export function getDocsList() {
  return http.get<ApiResponse<DocMeta[]>>('/docs/list')
}

/**
 * 读取单个文档并返回渲染好的 HTML 字符串。
 *
 * 后端响应 Content-Type 是 `text/html`，默认 axios 会尝试 JSON.parse 失败。
 * 用 `transformResponse` 兜住，把原始字符串装进 `{code:0, data}`，
 * 业务拦截器照样放行；调用方拿到 `resp.data` 即为可 `v-html` 的 HTML 片段。
 *
 * `locale`（可选）会作为 `?lang=` 传给后端，触发多语言 md 回退：
 * `<FILE>_<lang>.md` → `<FILE>_<short>.md` → `<FILE>.md`。
 * 不传时直接走默认 md。
 */
export function getDocHtml(name: string, locale?: string) {
  const params = locale && locale.trim() ? { lang: locale.trim() } : undefined
  return http.get<ApiResponse<string>>(`/docs/${encodeURIComponent(name)}`, {
    params,
    responseType: 'text',
    // 成功时后端直接吐 HTML 片段（Content-Type: text/html），不是业务 JSON 包，
    // 这里补壳 `{code:0}` 让统一响应拦截器放行，鉴权头 / token 刷新照常工作。
    //
    // 但 ZapError 的 HTTP 状态码也是 200，只靠 body 里的 `code` 区分成败
    // （见 zapd/src/zap/mod.rs）。若无条件包成 code:0，后端「文档不存在 / lang
    // 非法 / 读取失败」等错误会被当成成功内容渲染，问题被静默吞掉。
    // 因此：body 形如 JSON 且含 code 字段时原样还原，交由拦截器 reject。
    transformResponse: [
      (data: unknown) => {
        const raw = String(data ?? '')
        const s = raw.trim()
        if (s.startsWith('{') && s.endsWith('}')) {
          try {
            const parsed = JSON.parse(s)
            if (parsed && typeof parsed === 'object' && 'code' in parsed) return parsed
          } catch {
            /* 不是合法 JSON，按 HTML 处理 */
          }
        }
        return { code: 0, message: 'OK', data: raw }
      },
    ],
  })
}