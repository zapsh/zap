import { http } from '@/utils/request'
import type { LoginForm, UserInfo } from '@/types/user'
import type { ApiResponse } from '@/types/api_response'

// ── 认证 ───────────────────────────────────────────────────

export function login(data: LoginForm) {
  return http.post('/auth/login', data)
}

export function getUserInfo() {
  return http.get<ApiResponse<UserInfo>>('/user/info')
}

export function logout() {
  return http.get<ApiResponse>('/auth/logout')
}

// ── 用户管理 CRUD ─────────────────────────────────────────

export interface UserListItem {
  id: number
  username: string
  email: string
  phone: string
  nickname: string
  /** 用户家目录，如 /home/foo；老库回填前可能为空串 */
  home_dir: string
  /** Linux 系统账号名（每个面板用户一个账号）；空 = 未创建/未派生 */
  linux_user: string
  /** 该用户 PHP-FPM pool 规格 JSON；空 = 使用面板默认规格 */
  fpm_pool: string
  /**
   * PHP-FPM 规格引用：''=面板默认 / 'inherit'=继承 owner(reseller) 名下默认 /
   * 具体模板名（见运行环境页的 FPM 规格模板库）
   */
  fpm_spec_ref: string
  last_login_ip: string
  last_login_time: number
  status: number
  roles: string[]
  permissions: string[]
  owner_id: number
  /** 归属用户名（owner_id 对应用户；0 或已删除时为空串） */
  owner_username: string
  /** 用户类型：0=客户（独立家目录与系统账号）/ 1=成员（共享父账号） */
  user_kind: number
  /** 父账号对该成员收紧（取消）的权限点；仅成员有值 */
  perm_deny: string[]
  /** 绑定的套餐 id；0 = 未绑定套餐 */
  package_id: number
  /** 套餐名（未绑定时为空串） */
  package_name: string
  created_at: number
  updated_at: number
}

export interface UserListResponse {
  code: number
  message: string
  data: UserListItem[]
  total: number
}

/** 获取用户列表 */
export function getUserList(params?: { username?: string; status?: string }) {
  return http.get<UserListResponse>('/system/user/list', { params })
}

export interface ResellerItem {
  id: number
  username: string
  nickname: string
}

/** 获取经销商列表（admin 用于分配客户归属） */
export function getResellerList() {
  return http.get<ApiResponse<ResellerItem[]>>('/system/user/resellers')
}

export interface CreateUserPayload {
  username: string
  password: string
  email: string
  phone?: string
  nickname?: string
  roles?: string
  owner_id?: number
  /** 该用户 PHP-FPM pool 规格 JSON；缺省用面板默认（admin） */
  fpm_pool?: string
  /**
   * PHP-FPM 规格引用：''=面板默认 / 'inherit'=继承 owner(reseller) 名下默认 / 模板名
   * （与 fpm_pool 互斥，提交其一即可）
   */
  fpm_spec_ref?: string
  /** 套餐 id；0 / 缺省 = 不绑定套餐（不继承套餐限制） */
  package_id?: number
  /** 个人附加权限点：在角色权限之外单独授予（只做加法，admin） */
  permissions?: string[]
  /** 用户类型：0=客户（默认） / 1=成员（子账号，共享归属用户的家目录与系统账号） */
  user_kind?: number
  /** 父账号对该成员收紧（取消）的权限点；仅成员生效 */
  perm_deny?: string[]
}

/** 新增用户（返回 id / 家目录 / Linux 账号） */
export function createUser(data: CreateUserPayload) {
  return http.post<ApiResponse<{ id: number; home_dir: string; linux_user: string }>>(
    '/system/user/add',
    data,
  )
}

export interface UpdateUserPayload {
  id: number
  email?: string
  phone?: string
  nickname?: string
  roles?: string
  status?: number
  password?: string
  /** 该用户 PHP-FPM pool 规格 JSON；空 = 恢复面板默认（admin） */
  fpm_pool?: string
  /**
   * PHP-FPM 规格引用：''=面板默认 / 'inherit'=继承 owner(reseller) 名下默认 / 模板名；
   * 与 fpm_pool 互斥，提交引用时后端会清空旧的自定义 JSON（reseller 仅限自己名下模板）
   */
  fpm_spec_ref?: string
  /** 套餐 id；0 = 解除套餐绑定（变更后会重新下发磁盘配额） */
  package_id?: number
  /** 个人附加权限点；传空数组 = 清空附加权限（admin） */
  permissions?: string[]
  /** 父账号对该成员收紧（取消）的权限点；传空数组 = 取消全部收紧 */
  perm_deny?: string[]
}

/** 更新用户结果 */
export interface UpdateUserResult extends ApiResponse {}

/** 更新用户（管理员编辑或用户自己改密码） */
export function updateUser(data: UpdateUserPayload) {
  return http.post<UpdateUserResult>('/system/user/update', data)
}

/** 删除用户 */
export function deleteUser(id: number) {
  return http.post<ApiResponse>('/system/user/delete', { id })
}

/** 修改当前用户密码 */
export function changeMyPassword(newPassword: string) {
  return http.post<ApiResponse>('/system/user/update', { password: newPassword })
}

// ── 团队成员（子账号）───────────────────────────────────────
//
// 成员共享父账号的家目录与 Linux 系统账号（同一个 uid），权限默认继承父账号的
// 生效权限，父账号可通过 perm_deny 再收紧。层级固定一层：成员不能再建成员。

/** 团队成员（子账号）列表项 */
export interface TeamMemberItem {
  id: number
  username: string
  nickname: string
  email: string
  phone: string
  status: number
  roles: string[]
  permissions: string[]
  /** 父账号收紧（收回）的权限点 */
  perm_deny: string[]
  /** 共享的家目录与系统账号（取自父账号，成员不单独建） */
  home_dir: string
  linux_user: string
  last_login_time: number
  last_login_ip: string
  created_at: number
  updated_at: number
}

/** 新增成员参数 */
export interface TeamAddPayload {
  username: string
  password: string
  email: string
  phone?: string
  nickname?: string
  perm_deny?: string[]
}

/** 修改成员参数 */
export interface TeamUpdatePayload {
  id: number
  email?: string
  phone?: string
  nickname?: string
  status?: number
  password?: string
  perm_deny?: string[]
}

/** 团队成员列表（当前用户名下的成员） */
export function getTeamList() {
  return http.get<ApiResponse<TeamMemberItem[]>>('/user/team/list')
}

/** 新增成员 */
export function createTeamMember(data: TeamAddPayload) {
  return http.post<ApiResponse<{ id: number }>>('/user/team/add', data)
}

/** 修改成员（基本信息 / 状态 / 密码 / 收紧权限） */
export function updateTeamMember(data: TeamUpdatePayload) {
  return http.post<ApiResponse>('/user/team/update', data)
}

/** 删除成员（只删面板账号，不动共享的系统账号） */
export function deleteTeamMember(id: number) {
  return http.post<ApiResponse>('/user/team/delete', { id })
}

// ── 家目录 / 运行实体同步 ──────────────────────────────────

export interface HomeSyncOkItem {
  id: number
  username: string
  home_dir: string
  linux_user: string
  mode: string
}

export interface HomeSyncFailItem extends HomeSyncOkItem {
  error: string
}

export interface HomeSyncResult {
  ok: HomeSyncOkItem[]
  fail: HomeSyncFailItem[]
  mode: string
}

/** 批量补齐用户运行实体（每个用户：Linux 账号 nologin + 家目录赋权，admin only） */
export function userHomeSync() {
  return http.post<ApiResponse<HomeSyncResult>>('/system/user/home_sync', undefined, {
    timeout: 60000,
  })
}

// ── TOTP 两步验证 ───────────────────────────────────────────

export interface TotpSetupResult {
  secret: string
  otpauth_url: string
}

/** 生成两步验证密钥与 otpauth URL */
export function totpSetup() {
  return http.get<ApiResponse<TotpSetupResult>>('/auth/totp/setup')
}

/** 提交验证码启用两步验证 */
export function totpVerify(code: string) {
  return http.post<ApiResponse>('/auth/totp/verify', { code })
}

/** 校验验证码后关闭两步验证 */
export function totpDisable(code: string) {
  return http.post<ApiResponse>('/auth/totp/disable', { code })
}

/** 查询两步验证开启状态 */
export function totpStatus() {
  return http.get<ApiResponse<{ enabled: boolean }>>('/auth/totp/status')
}

// ── 个人中心 → 偏好设置 ─────────────────────────────────────

/** 当前用户通知偏好（个人中心 → 偏好设置） */
export interface NoticePrefs {
  /** 账户接近磁盘配额 */
  notify_disk_quota: boolean
  /** 账户接近带宽限制 */
  notify_bandwidth: boolean
  /** SSL 证书即将过期 */
  notify_ssl_expiry: boolean
  /** 账户密码变化 */
  notify_password_change: boolean
  password_change_disable: boolean
  /** 有人登录我的账户（成功登录通知） */
  notify_login: boolean
  login_disable: boolean
  /** AutoSSL 通知模式：deferrals=失败及延后 / failures=仅失败 / disabled=禁用 */
  autossl_notify_mode: 'deferrals' | 'failures' | 'disabled'
}

/** 读取当前用户偏好设置 */
export function getMyPrefs() {
  return http.get<ApiResponse<NoticePrefs>>('/user/prefs')
}

/** 保存当前用户偏好设置 */
export function saveMyPrefs(data: NoticePrefs) {
  return http.post<ApiResponse<NoticePrefs>>('/user/prefs', data)
}

// ── 家目录备份 ────────────────────────────────────────────

export interface HomeBackupData {
  /** 后台任务 id，用 `/appstore/runs` 轮询成败 */
  run_id: string
  username: string
  /** 产出压缩包路径：`{home}/backups/home_backup_<时间戳>.tar.gz` */
  path: string
  log: string
}

/**
 * 把家目录打包进 `{home}/backups/`。
 *
 * 打包可能耗时很久，故只返回 run_id（任务在服务端后台跑），调用方轮询
 * 运行记录判断成败；`username` 留空即备份当前登录用户自己。
 */
export function backupHome(username?: string) {
  return http.post<ApiResponse<HomeBackupData>>('/system/user/backup-home', {
    ...(username ? { username } : {}),
  })
}
