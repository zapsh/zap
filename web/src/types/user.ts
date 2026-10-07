// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 登录表单接口
 */
export interface LoginForm {
  username: string
  password: string
  captcha?: string
  rememberMe?: boolean
  /** 两步验证动态码（启用 TOTP 后第二步输入） */
  totp_code?: string
}


/**
 * 登录响应数据接口
 */
export interface LoginResponse {
  access_token: string
  code: number
  message: string
  token_type: string
  expire_in: number
}

/**
 * 用户信息接口
 */
export interface UserInfo {
  userId: string
  username: string
  nickname: string
  avatar: string
  email?: string
  phone?: string
  introduction?: string
  roles: string[]
  permissions: string[]
  [key: string]: any
}
