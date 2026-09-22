import { http } from '@/utils/request'

/**
 * Zap Pro 授权状态
 * mode: licensed 本机已授权 / managed 集群受管节点（功能由主控授权覆盖）/ none 未授权
 */
export interface LicenseStatus {
  active: boolean
  reason: string
  licensee: string
  plan: string
  expiresAt: number | null
  maxNodes: number | null
  usedNodes: number
  mode: 'licensed' | 'managed' | 'none'
  machineId: string
  version: string
  /** 接入状态（受管视图用，未接入时为全零值） */
  managed: {
    joined: boolean
    joinUrl: string
    ctrlMachineIdTail: string
    state: string
    lastReportAt: number
    lastError: string
    enrolledAt: number
    tlsMode: number
    tokenPrefix: string
    hasToken: boolean
    ctrlLicense: { expiresAt?: number | null; maxNodes?: number | null; usedNodes?: number }
  }
}

export function getLicenseStatus() {
  return http.get('/pro/license/status')
}

export function activateLicense(data: { license: string }) {
  return http.post('/pro/license/activate', data)
}

export function removeLicense() {
  return http.delete('/pro/license/remove')
}
