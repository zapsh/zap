// SPDX-License-Identifier: AGPL-3.0-only

/**
 * API 响应基础接口
 */
export interface ApiResponse<T = Record<string,any>> {
    code: number
    data: T
    message: string
}
