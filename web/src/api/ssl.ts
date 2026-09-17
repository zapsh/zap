import { http } from '@/utils/request'
import type { ApiResponse } from '@/types/api_response'

export type SslCertType = 'upload' | 'self-signed' | 'letsencrypt' | 'letsencrypt-staging'

export interface SslCertItem {
  id: number
  /** 证书归属用户 id（0 = 历史系统证书，仅管理员可见） */
  user_id?: number
  /** 归属用户登录名（系统证书 / 用户已删除时为空） */
  owner_name?: string
  name: string
  domains: string
  cert_type: SslCertType
  not_before: number
  not_after: number
  status: number
  remark: string
  created_at: number
  updated_at: number
}

export interface OwnerOption {
  id: number
  username: string
  nickname: string
}

export interface SslCertDetail extends SslCertItem {
  cert_content: string
  key_content: string
  ca_bundle: string
  csr: string
}

export interface SslCertUpsertData {
  id?: number
  /** 归属用户：仅管理员 / 经销商创建、编辑时可指定（默认当前用户） */
  user_id?: number
  name: string
  domains?: string
  cert_content?: string
  key_content?: string
  ca_bundle?: string
  csr?: string
  remark?: string
  status?: number
}

export interface SslCertParseResult {
  /** cert：X.509 证书；csr：证书签名请求 */
  kind: 'cert' | 'csr'
  domains: string[]
  domains_str: string
  common_name: string
  subject: string
  issuer: string
  not_before: number
  not_after: number
  serial: string
  fingerprint: string
  key_type: string
  key_bits: number
  /** SAN 中解析出的域名数量（0 表示取的是 CN） */
  sans_count: number
  /** PEM 中的证书数量（>1 说明是含中间链的 fullchain） */
  cert_count: number
  /** 与私钥的匹配结果（仅当调用时传了 keyPem 才有值） */
  key_match?: boolean | null
  /** 无法完成匹配校验的原因（如私钥格式错误 / 带密码） */
  key_error?: string
}

export function getCertList() {
  return http.get<ApiResponse<SslCertItem[]>>('/ssl/cert/list')
}

export function getCertDetail(id: number) {
  return http.get<ApiResponse<SslCertDetail>>('/ssl/cert/detail', { params: { id } })
}

/** 解析证书 / CSR，自动读取域名等信息；传 keyPem 时一并校验证书与私钥是否匹配 */
export function parseCert(pem: string, keyPem?: string) {
  return http.post<ApiResponse<SslCertParseResult>>('/ssl/cert/parse', { pem, key_pem: keyPem ?? '' })
}

export function addCert(data: SslCertUpsertData) {
  return http.post<ApiResponse>('/ssl/cert/add', data)
}

export function updateCert(data: SslCertUpsertData) {
  return http.post<ApiResponse>('/ssl/cert/update', data)
}

export function deleteCert(id: number) {
  return http.post<ApiResponse>('/ssl/cert/delete', { id })
}

export function selfSignCert(data: {
  name: string
  domains: string
  days?: number
  remark?: string
  user_id?: number
}) {
  return http.post<ApiResponse>('/ssl/cert/self-sign', data)
}

// ── Let's Encrypt 异步订单（ACME）────────────────────────────

/** 订单状态：pending=待处理 / processing=校验签发中 / issued=已签发 / failed=失败 / cancelled=已取消 */
export type AcmeOrderStatus =
  | 'pending'
  | 'processing'
  | 'issued'
  | 'failed'
  | 'cancelled'
  | 'expired'

/** 单个域名的验证明细：http-01 看 token/key_auth，dns-01 看 dns_host/dns_value */
export interface AcmeChallengeItem {
  domain: string
  status: string
  challenge_url: string
  token: string
  key_auth: string
  dns_host: string
  dns_value: string
  dns_record_id: string
  propagated: boolean
}

export interface AcmeOrder {
  id: number
  status: AcmeOrderStatus
  /** 后端给出的当前阶段说明，可直接展示 */
  stage: string
  cert_id: number
  domains: string[]
  challenge_type: 'http-01' | 'dns-01'
  dns_mode: 'manual' | 'auto'
  /** DNS-01 手动模式下用户需要添加的 TXT 记录 */
  todos: AcmeChallengeItem[]
  error: string
  expires_at: number
}

export interface LetsEncryptApplyData {
  email: string
  domains: string
  /** 验证方式：http=HTTP-01（自动写验证文件） / dns=DNS-01（默认，支持泛域名） */
  validation?: 'http' | 'dns'
  /** DNS-01 子模式：manual=手动解析（默认） / auto=调 DNS 服务商 API */
  dns_mode?: 'manual' | 'auto'
  dns_provider_id?: number
  name?: string
  staging?: boolean
  remark?: string
  user_id?: number
}

/** 下单创建 ACME 订单（dns-01 手动模式会返回待添加 TXT 记录） */
export function letsEncryptCert(data: LetsEncryptApplyData) {
  return http.post<ApiResponse<AcmeOrder>>('/ssl/letsencrypt', data)
}

/** 未完成的订单列表：刷新页面后可继续跟踪 */
export function getAcmeOrders() {
  return http.get<ApiResponse<AcmeOrder[]>>('/ssl/letsencrypt/orders')
}

/** 轮询订单状态；dns-01 手动模式下会连同传播检测结果一起返回 */
export function getAcmeOrderStatus(orderId: number) {
  return http.get<ApiResponse<AcmeOrder>>('/ssl/letsencrypt/status', {
    params: { order_id: orderId },
  })
}

/** 触发域名验证（用户完成解析后点击；http / 自动模式幂等） */
export function verifyAcmeOrder(orderId: number) {
  return http.post<ApiResponse<AcmeOrder>>('/ssl/letsencrypt/verify', { order_id: orderId })
}

/** 取消订单并清理已产生的验证素材 */
export function cancelAcmeOrder(orderId: number) {
  return http.post<ApiResponse<AcmeOrder>>('/ssl/letsencrypt/cancel', { order_id: orderId })
}

// ── ACME 的 DNS 服务商凭据（DNS-01 自动模式）────────────────

/** 服务商元信息：前端按 fields 动态渲染凭据表单，后端不再硬编码字段 */
export interface AcmeDnsProviderMeta {
  /** 存库标识，如 cloudflare / dnspod */
  kind: string
  /** 下拉展示名 */
  label: string
  fields: Array<{
    /** 提交给后端的字段名 */
    key: string
    label: string
    /** true = 敏感值（密码框，列表脱敏） */
    secret: boolean
    /** 取值说明，渲染为输入框下方提示 */
    hint: string
  }>
}

export interface AcmeDnsProviderItem {
  id: number
  user_id: number
  name: string
  provider: string
  /** 脱敏后的凭据字段 */
  credentials: Record<string, string>
  remark: string
  status: number
}

export function getAcmeDnsProviders() {
  return http.get<ApiResponse<AcmeDnsProviderMeta[]>>('/ssl/acme/dns/providers')
}

export function getAcmeDnsList() {
  return http.get<ApiResponse<AcmeDnsProviderItem[]>>('/ssl/acme/dns/list')
}

export interface AcmeDnsSaveData {
  /** 传 id 表示更新：留空的凭据字段保持不变 */
  id?: number
  name: string
  provider: string
  credentials?: Record<string, string>
  remark?: string
  user_id?: number
}

export function saveAcmeDnsProvider(data: AcmeDnsSaveData) {
  return http.post<ApiResponse<{ id: number }>>('/ssl/acme/dns/save', data)
}

export function deleteAcmeDnsProvider(id: number) {
  return http.post<ApiResponse>('/ssl/acme/dns/delete', { id })
}

/** 凭据连通性测试：传 id 时用库里已保存的密钥测试（密钥本身不下发前端） */
export function testAcmeDnsProvider(
  provider: string,
  credentials: Record<string, string>,
  id?: number,
) {
  return http.post<ApiResponse<{ message: string }>>('/ssl/acme/dns/test', {
    provider,
    credentials,
    ...(id ? { id } : {}),
  })
}
