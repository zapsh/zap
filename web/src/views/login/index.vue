<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { ref, reactive, computed, nextTick } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { useUserStore } from '@/stores/user'
import { useLocale } from '@/composables/useLocale'
import type { FormInstance, FormRules } from 'element-plus'
import type { LoginForm } from '@/types/user'
import { Key, Lock, Translate, User } from '@/icons'

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
const totpInputRef = ref()

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

// 登录方法（两步验证：密码正确但未提交验证码时进入第二步）
const handleLogin = async (formEl: FormInstance | undefined) => {
  if (!formEl) return

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
  <div class="login-container">
    <!-- 语言切换：登录是「第一眼」界面，英文用户第一次进来也能看懂 -->
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

    <el-form
      ref="loginFormRef"
      :model="loginForm"
      :rules="loginRules"
      class="login-form"
      autocomplete="on"
      label-position="top"
      @submit.prevent
    >
      <div class="title-container">
        <h3 class="title">ZAP</h3>
      </div>

      <el-form-item prop="username">
        <el-input
          v-model="loginForm.username"
          :placeholder="t('login.username')"
          type="text"
          tabindex="1"
          autocomplete="on"
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
          tabindex="2"
          autocomplete="on"
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
        <el-input
          ref="totpInputRef"
          v-model="loginForm.totp_code"
          :placeholder="t('login.totpCode')"
          type="text"
          maxlength="6"
          tabindex="3"
          autocomplete="one-time-code"
          inputmode="numeric"
          @keyup.enter="handleLogin(loginFormRef)"
        >
          <template #prefix>
            <el-icon><Key /></el-icon>
          </template>
        </el-input>
        <div class="totp-tip">{{ t('login.totpTip') }}</div>
      </el-form-item>

      <el-button
        :loading="loading"
        type="primary"
        style="width: 100%; margin-bottom: 30px"
        @click="handleLogin(loginFormRef)"
      >
        {{ t('login.submit') }}
      </el-button>
    </el-form>
  </div>
</template>

<style lang="scss" scoped>
.login-container {
  position: relative;
  min-height: 100vh;
  width: 100%;
  background-color: var(--el-fill-color-light);
  overflow: hidden;
  display: flex;
  justify-content: center;
  align-items: center;

  .locale-switch {
    position: absolute;
    top: 20px;
    right: 24px;
    display: flex;
    gap: 12px;
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

  .login-form {
    width: 420px;
    max-width: 100%;
    padding: 30px 35px;
    background: var(--el-bg-color);
    border-radius: 4px;
    box-shadow: var(--el-box-shadow-light);
  }

  .title-container {
    position: relative;
    text-align: center;
    margin-bottom: 30px;

    .title {
      font-size: 26px;
      color: var(--el-text-color-primary);
      margin: 0;
      font-weight: bold;
    }
  }

  .totp-tip {
    margin-top: 6px;
    font-size: 12px;
    color: var(--el-text-color-secondary);
    line-height: 1.5;
  }

  .password-strength {
    margin-top: 8px;
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
</style>
