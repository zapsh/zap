import { http } from '@/utils/request'

/** Mail 渠道配置（系统设置 → Zap 设置 → 通知设置，仅 admin） */
export interface MailSettings {
  host: string
  port: string
  encryption: string
  from: string
  username: string
  /** 密码不回显，恒为空串 */
  password: string
  /** 是否已保存过密码 */
  password_set?: boolean
  /** 已保存密码的掩码提示，如 ab****yz */
  password_hint?: string
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
