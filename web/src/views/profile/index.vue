<script setup lang="ts">
import { computed, onMounted, ref, reactive } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import QRCode from 'qrcode'
import { useUserStore } from '@/stores/user'
import TeamPanel from '@/views/team/TeamPanel.vue'
import {
  updateUser,
  totpSetup,
  totpVerify,
  totpDisable,
  totpStatus,
  getMyPrefs,
  saveMyPrefs,
} from '@/api/user'
import type { NoticePrefs } from '@/api/user'
import { roleLabel } from '@/utils/role'

const { t } = useI18n()
const userStore = useUserStore()
const { userInfo } = userStore

const activeTab = ref('info')

// ── 顶部页面切换：个人中心 / 团队成员 ────────────────────────
const route = useRoute()
const router = useRouter()

/**
 * 团队成员沿用原侧边栏菜单的可见角色（admin / user / reseller）。
 * demo 之类未获授权的角色不给入口，避免并入个人中心后凭空多出可见范围。
 */
const canSeeTeam = computed(() =>
  ((userInfo.roles ?? []) as string[]).some((r) => ['admin', 'user', 'reseller'].includes(r)),
)

const pages = computed(() => [
  { key: 'profile', label: t('profilePage.title') },
  ...(canSeeTeam.value ? [{ key: 'team', label: t('team.title') }] : []),
])

/** `?tab=team` 让旧链接（原 /team）与跳转直达团队面板 */
const activePage = ref(canSeeTeam.value && route.query.tab === 'team' ? 'team' : 'profile')

function switchPage(key: string) {
  if (activePage.value === key) return
  activePage.value = key
  router.replace({ query: key === 'team' ? { tab: 'team' } : {} })
}

// ── 基本资料 ───────────────────────────────────────────────
const infoForm = reactive({
  nickname: '',
  email: '',
  phone: '',
})

onMounted(async () => {
  // 回填当前用户信息
  infoForm.nickname = userInfo.nickname
  infoForm.email = userInfo.email
  infoForm.phone = userInfo.phone
  await loadTotpStatus()
  await loadPrefs()
})

async function saveInfo() {
  const phone = infoForm.phone.trim()
  if (phone && !/^1[3-9]\d{9}$/.test(phone)) {
    ElMessage.warning(t('profilePage.phoneInvalid'))
    return
  }
  try {
    await updateUser({
      id: userInfo.id,
      nickname: infoForm.nickname,
      email: infoForm.email,
      phone,
    })
    ElMessage.success(t('profilePage.infoSaved'))
    await userStore.getInfoAction()
  } catch {
    // 拦截器已弹窗
  }
}

// ── 修改密码 ───────────────────────────────────────────────
const pwdForm = reactive({
  newPassword: '',
  confirmPassword: '',
})
const pwdLoading = ref(false)

async function changePassword() {
  if (!pwdForm.newPassword) {
    ElMessage.warning(t('profilePage.pwdRequired'))
    return
  }
  if (pwdForm.newPassword.length < 6) {
    ElMessage.warning(t('profilePage.pwdTooShort'))
    return
  }
  if (pwdForm.newPassword !== pwdForm.confirmPassword) {
    ElMessage.warning(t('profilePage.pwdMismatch'))
    return
  }

  pwdLoading.value = true
  try {
    await updateUser({ id: userInfo.id, password: pwdForm.newPassword })
    ElMessage.success(t('profilePage.pwdChanged'))
    pwdForm.newPassword = ''
    pwdForm.confirmPassword = ''
  } catch {
    // 拦截器已弹窗
  } finally {
    pwdLoading.value = false
  }
}

// ── 两步验证 ───────────────────────────────────────────────
const totpEnabled = ref(false)
const totpSetupData = ref<{ secret: string; otpauth_url: string } | null>(null)
const qrCodeUrl = ref('')
const totpCode = ref('')
const totpActionLoading = ref(false)

async function loadTotpStatus() {
  try {
    const res = await totpStatus()
    totpEnabled.value = res.data?.enabled ?? false
  } catch {
    // 拦截器已弹窗
  }
}

async function startTotpSetup() {
  try {
    const res = await totpSetup()
    totpSetupData.value = res.data
    totpCode.value = ''
    QRCode.toDataURL(res.data.otpauth_url, { width: 180, margin: 1 })
      .then((url) => {
        qrCodeUrl.value = url
      })
      .catch(() => {
        qrCodeUrl.value = ''
      })
  } catch {
    // 拦截器已弹窗
  }
}

async function submitTotpVerify() {
  if (!/^\d{6}$/.test(totpCode.value)) {
    ElMessage.warning(t('profilePage.codeInvalid'))
    return
  }
  totpActionLoading.value = true
  try {
    await totpVerify(totpCode.value)
    ElMessage.success(t('profilePage.totpEnableSuccess'))
    totpEnabled.value = true
    totpSetupData.value = null
    qrCodeUrl.value = ''
    totpCode.value = ''
  } catch {
    // 拦截器已弹窗
  } finally {
    totpActionLoading.value = false
  }
}

async function disableTotp() {
  try {
    const { value } = await ElMessageBox.prompt(
      t('profilePage.disablePrompt'),
      t('profilePage.disableTitle'),
      {
        inputPlaceholder: t('profilePage.disableCodePlaceholder'),
        inputPattern: /^\d{6}$/,
        inputErrorMessage: t('profilePage.codeInvalid'),
        confirmButtonText: t('profilePage.disableConfirm'),
        cancelButtonText: t('common.cancel'),
      },
    )
    totpActionLoading.value = true
    try {
      await totpDisable(value)
      ElMessage.success(t('profilePage.disableSuccess'))
      totpEnabled.value = false
      totpCode.value = ''
    } catch {
      // 拦截器已弹窗
    } finally {
      totpActionLoading.value = false
    }
  } catch {
    // 用户取消
  }
}

async function copySecret() {
  if (!totpSetupData.value) return
  try {
    await navigator.clipboard.writeText(totpSetupData.value.secret)
    ElMessage.success(t('profilePage.secretCopied'))
  } catch {
    ElMessage.warning(t('profilePage.copyFailed'))
  }
}

// ── 偏好设置 ───────────────────────────────────────────────
const prefs = reactive<NoticePrefs>({
  notify_disk_quota: true,
  notify_bandwidth: true,
  notify_ssl_expiry: true,
  notify_password_change: true,
  password_change_disable: false,
  notify_login: false,
  login_disable: false,
  autossl_notify_mode: 'deferrals',
})
const prefsLoading = ref(false)
const prefsSaving = ref(false)

async function loadPrefs() {
  prefsLoading.value = true
  try {
    const res = await getMyPrefs()
    if (res.data) Object.assign(prefs, res.data)
  } catch {
    // 拦截器已弹窗
  } finally {
    prefsLoading.value = false
  }
}

async function savePrefs() {
  prefsSaving.value = true
  try {
    const res = await saveMyPrefs({ ...prefs })
    ElMessage.success(res.message || t('profilePage.prefsSaved'))
  } catch {
    // 拦截器已弹窗
  } finally {
    prefsSaving.value = false
  }
}
</script>

<template>
  <div class="app-container">
    <!-- 顶部 nav pill：个人中心 / 团队成员（团队成员已从左侧菜单移入此处） -->
    <nav class="page-pills">
      <button
        v-for="p in pages"
        :key="p.key"
        type="button"
        class="page-pill"
        :class="{ 'is-active': activePage === p.key }"
        @click="switchPage(p.key)"
      >
        {{ p.label }}
      </button>
    </nav>

    <TeamPanel v-if="activePage === 'team'" />

    <el-card v-else>
      <el-tabs v-model="activeTab">
        <!-- 基本资料 -->
        <el-tab-pane :label="t('profilePage.tabInfo')" name="info">
          <el-form label-width="80px" style="max-width: 440px" @submit.prevent>
            <el-form-item :label="t('profilePage.username')">
              <el-input :model-value="userInfo.username" disabled />
            </el-form-item>
            <el-form-item :label="t('profilePage.role')">
              <el-tag v-for="r in userInfo.roles" :key="r" style="margin-right: 6px">
                {{ roleLabel(r) }}
              </el-tag>
              <span v-if="!userInfo.roles?.length" style="color: var(--el-text-color-secondary)"
                >-</span
              >
            </el-form-item>
            <el-form-item :label="t('profilePage.nickname')">
              <el-input
                v-model="infoForm.nickname"
                :placeholder="t('profilePage.nicknamePlaceholder')"
              />
            </el-form-item>
            <el-form-item :label="t('profilePage.email')">
              <el-input v-model="infoForm.email" :placeholder="t('profilePage.emailPlaceholder')" />
            </el-form-item>
            <el-form-item :label="t('profilePage.phone')">
              <el-input
                v-model="infoForm.phone"
                :placeholder="t('profilePage.phonePlaceholder')"
                maxlength="11"
              />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" @click="saveInfo">{{
                t('profilePage.saveChanges')
              }}</el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>

        <!-- 修改密码 -->
        <el-tab-pane :label="t('profilePage.tabPassword')" name="password">
          <el-form label-width="90px" style="max-width: 400px" @submit.prevent>
            <el-form-item :label="t('profilePage.newPassword')">
              <el-input
                v-model="pwdForm.newPassword"
                type="password"
                show-password
                :placeholder="t('profilePage.passwordPlaceholder')"
              />
            </el-form-item>
            <el-form-item :label="t('profilePage.confirmPassword')">
              <el-input
                v-model="pwdForm.confirmPassword"
                type="password"
                show-password
                :placeholder="t('profilePage.confirmPlaceholder')"
              />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="pwdLoading" @click="changePassword">
                {{ t('profilePage.changePassword') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>

        <!-- 两步验证 -->
        <el-tab-pane :label="t('profilePage.tabTotp')" name="totp">
          <div class="totp-panel">
            <template v-if="totpEnabled">
              <el-tag type="success" size="large" effect="dark">{{
                t('profilePage.totpEnabledTag')
              }}</el-tag>
              <p class="totp-tip">
                {{ t('profilePage.totpEnabledTip') }}
              </p>
              <el-button type="danger" plain :loading="totpActionLoading" @click="disableTotp">
                {{ t('profilePage.totpDisableBtn') }}
              </el-button>
            </template>

            <template v-else-if="!totpSetupData">
              <p class="totp-tip">
                {{ t('profilePage.totpIntro') }}
              </p>
              <el-button type="primary" @click="startTotpSetup">{{
                t('profilePage.totpEnableBtn')
              }}</el-button>
            </template>

            <template v-else>
              <div class="totp-setup">
                <img
                  v-if="qrCodeUrl"
                  :src="qrCodeUrl"
                  :alt="t('profilePage.qrAlt')"
                  class="totp-qr"
                />
                <div class="totp-secret">
                  <span class="totp-secret-label">{{ t('profilePage.secretLabel') }}</span>
                  <code>{{ totpSetupData.secret }}</code>
                  <el-button link type="primary" @click="copySecret">{{
                    t('profilePage.copy')
                  }}</el-button>
                </div>
                <p class="totp-tip">
                  {{ t('profilePage.totpScanTip') }}
                </p>
                <el-input
                  v-model="totpCode"
                  :placeholder="t('profilePage.codePlaceholder')"
                  maxlength="6"
                  style="max-width: 220px"
                />
                <div style="margin-top: 12px">
                  <el-button type="primary" :loading="totpActionLoading" @click="submitTotpVerify">
                    {{ t('profilePage.confirmEnable') }}
                  </el-button>
                  <el-button @click="totpSetupData = null">{{ t('common.cancel') }}</el-button>
                </div>
              </div>
            </template>
          </div>
        </el-tab-pane>

        <!-- 偏好设置 -->
        <el-tab-pane :label="t('profilePage.tabPrefs')" name="prefs">
          <div class="prefs-panel" v-loading="prefsLoading">
            <h4 class="prefs-title">{{ t('profilePage.prefsTitle') }}</h4>
            <p class="prefs-desc">{{ t('profilePage.prefsDesc') }}</p>

            <el-checkbox v-model="prefs.notify_disk_quota" class="prefs-item">
              {{ t('profilePage.prefDiskQuota') }}
            </el-checkbox>

            <el-checkbox v-model="prefs.notify_bandwidth" class="prefs-item">
              {{ t('profilePage.prefBandwidth') }}
            </el-checkbox>

            <el-checkbox v-model="prefs.notify_ssl_expiry" class="prefs-item">
              {{ t('profilePage.prefSslExpiry') }}
            </el-checkbox>

            <el-checkbox v-model="prefs.notify_password_change" class="prefs-item">
              {{ t('profilePage.prefPasswordChange') }}
            </el-checkbox>
            <div v-if="prefs.notify_password_change" class="prefs-sub">
              <el-checkbox v-model="prefs.password_change_disable">{{
                t('profilePage.prefNoNotify')
              }}</el-checkbox>
            </div>

            <el-checkbox v-model="prefs.notify_login" class="prefs-item">
              {{ t('profilePage.prefLogin') }}
            </el-checkbox>
            <div v-if="prefs.notify_login" class="prefs-sub">
              <el-checkbox v-model="prefs.login_disable">{{
                t('profilePage.prefNoNotify')
              }}</el-checkbox>
            </div>

            <h4 class="prefs-title prefs-group-title">{{ t('profilePage.autosslTitle') }}</h4>
            <el-radio-group v-model="prefs.autossl_notify_mode" class="prefs-radio-group">
              <el-radio value="deferrals" class="prefs-radio">{{
                t('profilePage.autosslDeferrals')
              }}</el-radio>
              <el-radio value="failures" class="prefs-radio">{{
                t('profilePage.autosslFailures')
              }}</el-radio>
              <el-radio value="disabled" class="prefs-radio">{{
                t('profilePage.autosslDisabled')
              }}</el-radio>
            </el-radio-group>

            <div style="margin-top: 28px">
              <el-button type="primary" :loading="prefsSaving" @click="savePrefs">{{
                t('profilePage.saveChanges')
              }}</el-button>
            </div>
          </div>
        </el-tab-pane>
      </el-tabs>
    </el-card>
  </div>
</template>

<style scoped>
.app-container {
  padding: 20px;
}

/* 顶部 nav pill 分段导航（与 Docker 页同一套视觉） */
.page-pills {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px;
  margin-bottom: 12px;
  background: var(--el-fill-color-light);
  border-radius: 10px;
  overflow-x: auto;
}

.page-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 16px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--el-text-color-regular);
  font-size: 13px;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.2s, color 0.2s;
}

.page-pill:hover {
  color: var(--el-color-primary);
}

.page-pill.is-active {
  background: var(--el-color-primary);
  color: #fff;
  font-weight: 500;
}

.totp-panel {
  max-width: 440px;
}

.totp-tip {
  color: var(--el-text-color-secondary);
  font-size: 13px;
  line-height: 1.7;
  margin: 12px 0;
}

.totp-setup {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.totp-qr {
  width: 180px;
  height: 180px;
  border: 1px solid var(--el-border-color-light);
  border-radius: 4px;
}

.totp-secret-label {
  color: var(--el-text-color-regular);
  font-size: 13px;
}

.totp-secret code {
  background: var(--el-fill-color);
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 13px;
  word-break: break-all;
}

/* ── 偏好设置 ── */
.prefs-panel {
  max-width: 680px;
}
.prefs-title {
  margin: 0 0 8px;
  font-size: 15px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.prefs-group-title {
  margin-top: 28px;
}
.prefs-desc {
  margin: 0 0 16px;
  color: var(--el-text-color-regular);
  font-size: 13px;
}
.prefs-item {
  display: flex;
  align-items: flex-start;
  margin: 0 0 10px;
  white-space: normal;
  line-height: 1.6;
}
.prefs-sub {
  margin: 0 0 12px 32px;
  color: var(--el-text-color-secondary);
}
.prefs-radio-group {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
}
.prefs-radio {
  margin: 0 0 10px;
}
</style>
