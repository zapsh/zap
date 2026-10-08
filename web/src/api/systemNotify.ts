// SPDX-License-Identifier: AGPL-3.0-only
import { http } from '@/utils/request'

/** Mail 渠道配置（系统设置 → Zap 设置 → 通知设置，仅 admin） */
export interface MailSettings {
  provider: string
  from: string
  // ── SMTP ──
  host: string
  port: string
  encryption: string
  username: string
  /** 密码不回显，恒为空串 */
  password: string
  password_set?: boolean
  password_hint?: string
  // ── SendGrid ──
  sg_api_key: string
  sg_api_key_set?: boolean
  sg_api_key_hint?: string
  // ── 阿里云 DirectMail ──
  aliyun_access_key: string
  aliyun_access_secret: string
  aliyun_access_secret_set?: boolean
  aliyun_access_secret_hint?: string
  aliyun_region: string
  // ── 腾讯云 SES ──
  tencent_secret_id: string
  tencent_secret_key: string
  tencent_secret_key_set?: boolean
  tencent_secret_key_hint?: string
  tencent_region: string
}

/**
 * 读取通知设置。
 *
 * 后端 `/system/config/basic` 仍会返回 basic / mail / contact 三段（原「基础设置」页的
 * 遗留结构），面板这一侧只取 `mail`：建站默认网络与联系信息已下线，键值仍留在
 * `server_env.yaml` 里，这里不再读写。
 */
export function getMailSettings() {
  return http.get<{ code: number; message: string; data: { mail: MailSettings } }>(
    '/system/config/basic',
  )
}

/** 保存通知设置（按渠道部分提交；mail.password 留空=不改原密码） */
export function saveMailSettings(mail: Partial<MailSettings>) {
  return http.post<{ code: number; message: string }>('/system/config/basic', { mail })
}

/** 单个事件的邮件模板（主题 + 纯文本正文 + HTML 正文 + 是否 HTML） */
export interface MailTemplate {
  subject: string
  /** 纯文本正文（非 HTML 模式使用，亦作为 HTML 模式的纯文本兜底） */
  body_text: string
  /** HTML 正文（is_html=true 时使用） */
  body_html: string
  /** 是否为 HTML 格式 */
  is_html: boolean
}

/** 邮件模板读取结果 */
export interface MailTemplates {
  /** `global` = 管理员全局模板；`self` = 当前用户（reseller）私有模板 */
  scope: 'global' | 'self'
  events: Record<string, MailTemplate>
}

/** 读取当前用户可编辑作用域下的邮件通知模板 */
export function getMailTemplates() {
  return http.get<{ code: number; message: string; data: MailTemplates }>(
    '/system/mail/templates',
  )
}

/** 保存邮件通知模板（仅提交需要改的事件） */
export function saveMailTemplates(events: Record<string, MailTemplate>) {
  return http.post<{ code: number; message: string }>('/system/mail/templates', { events })
}

/** 发送测试邮件（仅管理员；to 为接收邮箱） */
export function sendTestMail(to: string) {
  return http.post<{ code: number; message: string }>('/system/mail/test', { to })
}

/** 群发通知（admin 全平台 / reseller 名下客户；可指定 user_ids 或全发）
 * channel: inbox=仅站内信，email=仅邮件，both=两者都发 */
export function broadcastMail(payload: {
  subject: string
  body_text: string
  body_html?: string
  is_html: boolean
  channel?: 'inbox' | 'email' | 'both'
  user_ids?: number[]
}) {
  return http.post<{
    code: number
    message: string
    data: {
      sent_email: number
      sent_inbox: number
      failed: Array<{ email?: string; username: string; error: string }>
    }
  }>('/system/mail/broadcast', payload)
}

/** 群发通知模板（用户自建命名模板） */
export interface BroadcastTemplate {
  id: number
  owner_id: number
  /** global=全局（仅管理员）；self=私有（创建者自己） */
  scope: 'global' | 'self'
  name: string
  subject: string
  body_text: string
  body_html: string
  is_html: boolean
}

/** 读取可用模板（全局 + 自己的私有） */
export function getBroadcastTemplates() {
  return http.get<{ code: number; message: string; data: { list: BroadcastTemplate[] } }>(
    '/system/broadcast/templates',
  )
}

/** 创建 / 更新群发通知模板（id 省略=创建） */
export function saveBroadcastTemplate(
  payload: {
    name: string
    scope?: 'global' | 'self'
    subject: string
    body_text: string
    body_html?: string
    is_html: boolean
  },
  id?: number,
) {
  const body = { ...payload, body_html: payload.body_html || undefined }
  return id
    ? http.put<{ code: number; message: string }>(`/system/broadcast/templates/${id}`, body)
    : http.post<{ code: number; message: string }>('/system/broadcast/templates', body)
}

/** 删除群发通知模板 */
export function deleteBroadcastTemplate(id: number) {
  return http.delete<{ code: number; message: string }>(`/system/broadcast/templates/${id}`)
}
