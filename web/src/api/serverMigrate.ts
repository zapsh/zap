import { http } from '@/utils/request'

/** 待迁移用户候选 */
export interface MigrateCandidate {
  id: number
  username: string
  linux_user: string
  home_dir: string
  /** 站点数：按家目录统计（含共享该家目录的团队成员站点） */
  site_count: number
  /** 共享该家目录的面板账号数（含自己，>1 表示站长 + 团队成员） */
  share_count: number
}

export interface MigratePreview {
  src: string
  count: number
  candidates: MigrateCandidate[]
}

/** 迁移结果项 */
export interface MigrateOkItem {
  id: number
  username: string
  linux_user: string
  old_home: string
  new_home: string
  sites: number
  sites_synced: number
  site_errors: string[]
  /** true=该家目录由同组账号先搬过，本次仅复用结果并更新记录 */
  reused?: boolean
}

export interface MigrateFailItem {
  id: number
  username: string
  home_dir: string
  error: string
}

export interface MigrateResult {
  src: string
  dest: string
  mode: string
  ok: MigrateOkItem[]
  fail: MigrateFailItem[]
}

/** 列出位于源挂载点下、可迁移的用户（admin） */
export function getMigrateUsers(src?: string) {
  return http.get<{ code: number; message: string; data: MigratePreview }>(
    '/system/migrate/users',
    { params: { src } },
  )
}

/** 执行数据迁移：源挂载点 → 目标挂载点（user_ids 为空 = 全部候选用户） */
export function runMigrate(payload: { src?: string; dest: string; user_ids?: number[] }) {
  return http.post<{ code: number; message: string; data: MigrateResult }>(
    '/system/migrate/home',
    payload,
  )
}
