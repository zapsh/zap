<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { ref, reactive, computed, nextTick, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { useUserStore } from '@/stores/user'
import { useLocale } from '@/composables/useLocale'
import type { FormInstance, FormRules, InputOtpInstance } from 'element-plus'
import type { LoginForm } from '@/types/user'
import { ElectricBoltOutline, Lock, User } from '@/icons'

const router = useRouter()
const userStore = useUserStore()
const { t } = useI18n()

// 登录页也要能切语言：英文用户第一次进来不该只看到中文
const { locale, options: localeOptions, change: changeLocale } = useLocale()

// 登录表单
const loginForm = reactive<LoginForm>({
  username: '',
  password: '',
  totp_code: '',
})

// 两步验证（TOTP）第二步是否可见
const showTotp = ref(false)
const totpInputRef = ref<InputOtpInstance>()

// 密码强度计算
const passwordStrength = computed(() => {
  const pwd = loginForm.password
  if (!pwd) return { level: 0, text: '', color: '' }

  let score = 0
  if (pwd.length >= 8) score++
  if (pwd.length >= 12) score++
  if (/[a-z]/.test(pwd) && /[A-Z]/.test(pwd)) score++
  if (/\d/.test(pwd)) score++
  if (/[^a-zA-Z0-9]/.test(pwd)) score++

  if (score <= 1) return { level: 1, text: t('login.strengthWeak'), color: '#F56C6C' }
  if (score <= 2) return { level: 2, text: t('login.strengthFair'), color: '#E6A23C' }
  if (score <= 3) return { level: 3, text: t('login.strengthMedium'), color: '#409EFF' }
  if (score <= 4) return { level: 4, text: t('login.strengthStrong'), color: '#67C23A' }
  return { level: 5, text: t('login.strengthVeryStrong'), color: '#67C23A' }
})

// 表单校验规则
const validatePassword = (_rule: any, value: string, callback: any) => {
  if (!value) {
    callback(new Error(t('login.passwordRequired')))
  } else if (value.length < 6) {
    callback(new Error(t('login.passwordTooShort')))
  } else if (value.length > 128) {
    callback(new Error(t('login.passwordTooLong')))
  } else {
    callback()
  }
}

// 用 computed：切换语言时校验提示跟着变（reactive 只在 setup 时求值一次）
const loginRules = computed<FormRules>(() => ({
  username: [
    { required: true, message: t('login.usernameRequired'), trigger: 'blur' },
    { min: 2, max: 50, message: t('login.usernameLength'), trigger: 'blur' },
  ],
  password: [{ required: true, validator: validatePassword, trigger: 'blur' }],
  totp_code: [{ pattern: /^\d{6}$/, message: t('login.totpRequired'), trigger: 'blur' }],
}))

const loginFormRef = ref<FormInstance>()
const loading = ref(false)

// 进入登录页自动聚焦用户名（登录过期跳转/刷新后重新挂载时，方便直接输入）
const usernameInputRef = ref()
onMounted(() => {
  nextTick(() => usernameInputRef.value?.focus())
})

// 登录方法（两步验证：密码正确但未提交验证码时进入第二步）
const handleLogin = async (formEl: FormInstance | undefined) => {
  if (!formEl || loading.value) return

  // 已进入第二步时必须先填写验证码
  if (showTotp.value && !/^\d{6}$/.test(loginForm.totp_code || '')) {
    ElMessage.warning(t('login.totpRequired'))
    totpInputRef.value?.focus()
    return
  }

  await formEl.validate(async (valid) => {
    if (valid) {
      loading.value = true
      try {
        await userStore.login(loginForm)

        // 获取用户信息（包含角色和权限）
        await userStore.getInfoAction()

        ElMessage.success(t('login.success'))
        router.push({ path: '/' })
      } catch (error: any) {
        // 密码正确但账号已启用两步验证 → 展示验证码输入框进入第二步
        if (error?.code === 1002) {
          showTotp.value = true
          loginForm.totp_code = ''
          nextTick(() => totpInputRef.value?.focus())
          return
        }
        ElMessage.error(error.message || t('login.failed'))
        if (showTotp.value) {
          // 第二步失败（如验证码错误）：保留验证码框，仅清空验证码
          loginForm.totp_code = ''
          totpInputRef.value?.focus()
        } else {
          loginForm.password = ''
          if (formEl) {
            formEl.clearValidate('password')
          }
        }
      } finally {
        loading.value = false
      }
    }
  })
}
</script>

<template>
  <div class="login-page">
    <!-- 居中登录区 -->
    <main class="login-main">
      <section class="login-card">
        <span class="card-logo">
          <el-icon :size="24"><ElectricBoltOutline /></el-icon>
        </span>
        <h3 class="title">{{ t('login.welcome') }}</h3>

        <el-form
          ref="loginFormRef"
          :model="loginForm"
          :rules="loginRules"
          class="login-form"
          autocomplete="on"
          label-position="top"
          @submit.prevent
        >
          <el-form-item prop="username">
            <el-input
              ref="usernameInputRef"
              v-model="loginForm.username"
              :placeholder="t('login.username')"
              type="text"
              size="large"
              tabindex="1"
              autocomplete="username"
            >
              <template #prefix>
                <el-icon><User /></el-icon>
              </template>
            </el-input>
          </el-form-item>

          <el-form-item prop="password">
            <el-input
              v-model="loginForm.password"
              :placeholder="t('login.password')"
              type="password"
              size="large"
              tabindex="2"
              autocomplete="current-password"
              show-password
              @keyup.enter="handleLogin(loginFormRef)"
            >
              <template #prefix>
                <el-icon><Lock /></el-icon>
              </template>
            </el-input>
            <!-- 密码强度指示器 -->
            <div v-if="loginForm.password" class="password-strength">
              <span class="strength-label">{{ t('login.strengthLabel') }}</span>
              <span :style="{ color: passwordStrength.color }" class="strength-text">
                {{ passwordStrength.text }}
              </span>
              <div class="strength-bar">
                <div
                  v-for="i in 5"
                  :key="i"
                  class="strength-segment"
                  :class="{ active: i <= passwordStrength.level }"
                  :style="{
                    backgroundColor:
                      i <= passwordStrength.level
                        ? passwordStrength.color
                        : 'var(--el-border-color-lighter)',
                  }"
                />
              </div>
            </div>
          </el-form-item>

          <el-form-item v-if="showTotp" prop="totp_code">
            <el-input-otp
              ref="totpInputRef"
              v-model="loginForm.totp_code"
              :maxlength="6"
              format="number"
              class="totp-input"
              @complete="handleLogin(loginFormRef)"
            />
            <div class="totp-tip">{{ t('login.totpTip') }}</div>
          </el-form-item>

          <el-button
            class="submit-btn"
            :loading="loading"
            type="primary"
            size="large"
            @click="handleLogin(loginFormRef)"
          >
            {{ t('login.submit') }}
          </el-button>
        </el-form>
      </section>

      <!-- 语言切换：放在登录卡片下方，英文用户第一次进来也能看懂 -->
      <div class="locale-switch">
        <span
          v-for="item in localeOptions"
          :key="item.value"
          class="locale-item"
          :class="{ active: locale === item.value }"
          @click="changeLocale(item.value)"
        >
          {{ item.label }}
        </span>
      </div>
    </main>
  </div>
</template>

<style lang="scss" scoped>
.login-page {
  position: relative;
  min-height: 100vh;
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
  overflow: hidden;
  background-color: var(--el-fill-color-light);

  /* 主色柔光背景，让居中卡片更有层次、更「高级」 */
  &::before {
    content: '';
    position: absolute;
    width: 720px;
    height: 720px;
    top: -240px;
    right: -160px;
    border-radius: 50%;
    background: radial-gradient(circle, rgba(64, 158, 255, 0.12) 0%, transparent 70%);
    pointer-events: none;
  }

  &::after {
    content: '';
    position: absolute;
    width: 640px;
    height: 640px;
    bottom: -260px;
    left: -200px;
    border-radius: 50%;
    background: radial-gradient(circle, rgba(103, 194, 58, 0.08) 0%, transparent 70%);
    pointer-events: none;
  }
}

/* ── 表单区 ─────────────────────────────────────── */
.login-main {
  position: relative;
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
}

.locale-switch {
  display: flex;
  gap: 14px;
  margin-top: 22px;
  font-size: 13px;

  .locale-item {
    cursor: pointer;
    color: var(--el-text-color-secondary);
    user-select: none;
    transition: color 0.2s;

    &:hover {
      color: var(--el-color-primary);
    }

    &.active {
      color: var(--el-color-primary);
      font-weight: 600;
    }
  }
}

.login-card {
  width: 420px;
  max-width: 100%;
  padding: 44px 40px 36px;
  background: var(--el-bg-color);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 14px;
  box-shadow: 0 20px 60px rgba(15, 23, 42, 0.08);
  text-align: center;
  animation: card-in 0.35s ease-out;

  .card-logo {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 52px;
    height: 52px;
    margin-bottom: 18px;
    border-radius: 14px;
    color: #fff;
    background: linear-gradient(140deg, var(--el-color-primary), #1b6ff0);
    box-shadow: 0 8px 22px rgba(30, 111, 240, 0.3);
  }

  .title {
    margin: 14px 0 28px;
    font-size: 24px;
    font-weight: 700;
    letter-spacing: 0.3px;
    color: var(--el-text-color-primary);
  }

  .login-form {
    text-align: left;

    :deep(.el-input__wrapper) {
      border-radius: 8px;
    }
  }

  .totp-input {
    width: 100%;

    :deep(.el-input__wrapper) {
      justify-content: center;
    }
  }

  .totp-tip {
    margin-top: 10px;
    font-size: 12px;
    color: var(--el-text-color-secondary);
    line-height: 1.5;
    text-align: left;
  }

  .submit-btn {
    width: 100%;
    margin-top: 6px;
    border-radius: 8px;
  }

  .password-strength {
    margin-top: 8px;
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;

    .strength-label {
      font-size: 12px;
      color: var(--el-text-color-secondary);
    }

    .strength-text {
      font-size: 12px;
      font-weight: 500;
    }

    .strength-bar {
      display: flex;
      gap: 2px;
      flex: 1;
      min-width: 100px;

      .strength-segment {
        height: 4px;
        flex: 1;
        border-radius: 2px;
        transition: background-color 0.3s;
      }
    }
  }
}

@keyframes card-in {
  from {
    opacity: 0;
    transform: translateY(14px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
</style>
