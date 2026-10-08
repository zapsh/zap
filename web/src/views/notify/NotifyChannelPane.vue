<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { computed, reactive, ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { getMailSettings, saveMailSettings } from '@/api/systemNotify'

const { t } = useI18n()
const loading = ref(false)
const saving = ref(false)

const providerOptions = computed(() => [
  { value: 'smtp', label: t('notifyCfg.providerSmtp') },
  { value: 'sendgrid', label: t('notifyCfg.providerSendgrid') },
  { value: 'aliyun', label: t('notifyCfg.providerAliyun') },
  { value: 'tencent', label: t('notifyCfg.providerTencent') },
  { value: 'mailgun', label: t('notifyCfg.providerMailgun') },
])

// 协议名保持英文，只有「无加密」需要本地化
const encryptionOptions = computed(() => [
  { value: 'ssl', label: 'SSL / TLS (465)' },
  { value: 'tls', label: 'STARTTLS (587)' },
  { value: 'none', label: t('notifyCfg.encNone') },
])

const mail = reactive({
  provider: 'smtp',
  from: '',
  host: '',
  port: '587',
  encryption: 'tls',
  username: '',
  password: '',
  password_set: false,
  password_hint: '',
  sg_api_key: '',
  sg_api_key_set: false,
  sg_api_key_hint: '',
  aliyun_access_key: '',
  aliyun_access_secret: '',
  aliyun_access_secret_set: false,
  aliyun_access_secret_hint: '',
  aliyun_region: 'cn-hangzhou',
  tencent_secret_id: '',
  tencent_secret_key: '',
  tencent_secret_key_set: false,
  tencent_secret_key_hint: '',
  tencent_region: 'ap-guangzhou',
  mailgun_api_key: '',
  mailgun_api_key_set: false,
  mailgun_api_key_hint: '',
  mailgun_domain: '',
  mailgun_region: 'us',
})

const passwordPlaceholder = computed(() =>
  mail.password_set
    ? t('notifyCfg.passwordKeepHint', { hint: mail.password_hint || t('notifyCfg.saved') })
    : t('notifyCfg.passwordPlaceholder'),
)
const sgKeyPlaceholder = computed(() =>
  mail.sg_api_key_set
    ? t('notifyCfg.secretKeepHint', { hint: mail.sg_api_key_hint || t('notifyCfg.saved') })
    : t('notifyCfg.sgApiKeyPlaceholder'),
)
const aliSecretPlaceholder = computed(() =>
  mail.aliyun_access_secret_set
    ? t('notifyCfg.secretKeepHint', { hint: mail.aliyun_access_secret_hint || t('notifyCfg.saved') })
    : t('notifyCfg.aliyunAccessSecret'),
)
const tcSecretPlaceholder = computed(() =>
  mail.tencent_secret_key_set
    ? t('notifyCfg.secretKeepHint', { hint: mail.tencent_secret_key_hint || t('notifyCfg.saved') })
    : t('notifyCfg.tencentSecretKey'),
)
const mgKeyPlaceholder = computed(() =>
  mail.mailgun_api_key_set
    ? t('notifyCfg.secretKeepHint', { hint: mail.mailgun_api_key_hint || t('notifyCfg.saved') })
    : t('notifyCfg.mailgunApiKey'),
)

async function load() {
  loading.value = true
  try {
    const res = await getMailSettings()
    const m = res.data.mail
    mail.provider = m.provider || 'smtp'
    mail.from = m.from ?? ''
    mail.host = m.host ?? ''
    mail.port = m.port || '587'
    mail.encryption = m.encryption || 'tls'
    mail.username = m.username ?? ''
    mail.password = ''
    mail.password_set = m.password_set === true
    mail.password_hint = m.password_hint ?? ''
    mail.sg_api_key = ''
    mail.sg_api_key_set = m.sg_api_key_set === true
    mail.sg_api_key_hint = m.sg_api_key_hint ?? ''
    mail.aliyun_access_key = m.aliyun_access_key ?? ''
    mail.aliyun_access_secret = ''
    mail.aliyun_access_secret_set = m.aliyun_access_secret_set === true
    mail.aliyun_access_secret_hint = m.aliyun_access_secret_hint ?? ''
    mail.aliyun_region = m.aliyun_region ?? 'cn-hangzhou'
    mail.tencent_secret_id = m.tencent_secret_id ?? ''
    mail.tencent_secret_key = ''
    mail.tencent_secret_key_set = m.tencent_secret_key_set === true
    mail.tencent_secret_key_hint = m.tencent_secret_key_hint ?? ''
    mail.tencent_region = m.tencent_region ?? 'ap-guangzhou'
    mail.mailgun_api_key = ''
    mail.mailgun_api_key_set = m.mailgun_api_key_set === true
    mail.mailgun_api_key_hint = m.mailgun_api_key_hint ?? ''
    mail.mailgun_domain = m.mailgun_domain ?? ''
    mail.mailgun_region = m.mailgun_region ?? 'us'
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    loading.value = false
  }
}

async function save() {
  const port = String(mail.port).trim()
  const num = Number(port)
  if (mail.provider === 'smtp' && mail.host.trim() && (!Number.isInteger(num) || num < 1 || num > 65535)) {
    ElMessage.warning(t('notifyCfg.invalidPort'))
    return
  }
  if (!mail.from.trim()) {
    ElMessage.warning(t('notifyCfg.from'))
    return
  }
  saving.value = true
  try {
    await saveMailSettings({
      provider: mail.provider,
      from: mail.from.trim(),
      host: mail.host.trim(),
      port: port || '',
      encryption: mail.encryption,
      username: mail.username.trim(),
      password: mail.password.trim(),
      sg_api_key: mail.sg_api_key.trim(),
      aliyun_access_key: mail.aliyun_access_key.trim(),
      aliyun_access_secret: mail.aliyun_access_secret.trim(),
      aliyun_region: mail.aliyun_region.trim(),
      tencent_secret_id: mail.tencent_secret_id.trim(),
      tencent_secret_key: mail.tencent_secret_key.trim(),
      tencent_region: mail.tencent_region.trim(),
      mailgun_api_key: mail.mailgun_api_key.trim(),
      mailgun_domain: mail.mailgun_domain.trim(),
      mailgun_region: mail.mailgun_region.trim(),
    })
    ElMessage.success(t('notifyCfg.mailSaved'))
    await load() // 重新拉取，刷新已保存密钥的掩码提示
  } catch {
    /* 拦截器已弹窗 */
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<template>
  <div v-loading="loading">
    <el-form :model="mail" label-width="160px" style="max-width: 720px" @submit.prevent>
      <el-form-item :label="t('notifyCfg.provider')">
        <el-select v-model="mail.provider" style="width: 280px">
          <el-option
            v-for="opt in providerOptions"
            :key="opt.value"
            :label="opt.label"
            :value="opt.value"
          />
        </el-select>
      </el-form-item>

      <el-form-item :label="t('notifyCfg.from')">
        <el-input
          v-model="mail.from"
          :placeholder="t('notifyCfg.fromPlaceholder')"
          clearable
        />
      </el-form-item>

      <!-- SMTP -->
      <template v-if="mail.provider === 'smtp'">
        <el-form-item :label="t('notifyCfg.smtpHost')">
          <el-input
            v-model="mail.host"
            :placeholder="t('notifyCfg.smtpHostPlaceholder')"
            clearable
          />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.port')">
          <el-input v-model="mail.port" placeholder="465 / 587 / 25" style="width: 180px" />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.encryption')">
          <el-select v-model="mail.encryption" style="width: 240px">
            <el-option
              v-for="opt in encryptionOptions"
              :key="opt.value"
              :label="opt.label"
              :value="opt.value"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('notifyCfg.account')">
          <el-input
            v-model="mail.username"
            :placeholder="t('notifyCfg.accountPlaceholder')"
            clearable
          />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.password')">
          <el-input
            v-model="mail.password"
            type="password"
            show-password
            :placeholder="passwordPlaceholder"
          />
        </el-form-item>
      </template>

      <!-- SendGrid -->
      <template v-else-if="mail.provider === 'sendgrid'">
        <el-form-item :label="t('notifyCfg.sgApiKey')">
          <el-input
            v-model="mail.sg_api_key"
            type="password"
            show-password
            :placeholder="sgKeyPlaceholder"
          />
        </el-form-item>
      </template>

      <!-- 阿里云 -->
      <template v-else-if="mail.provider === 'aliyun'">
        <el-form-item :label="t('notifyCfg.aliyunAccessKey')">
          <el-input v-model="mail.aliyun_access_key" clearable />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.aliyunAccessSecret')">
          <el-input
            v-model="mail.aliyun_access_secret"
            type="password"
            show-password
            :placeholder="aliSecretPlaceholder"
          />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.region')">
          <el-input
            v-model="mail.aliyun_region"
            :placeholder="t('notifyCfg.regionPlaceholder')"
            style="width: 240px"
          />
        </el-form-item>
      </template>

      <!-- 腾讯云 -->
      <template v-else-if="mail.provider === 'tencent'">
        <el-form-item :label="t('notifyCfg.tencentSecretId')">
          <el-input v-model="mail.tencent_secret_id" clearable />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.tencentSecretKey')">
          <el-input
            v-model="mail.tencent_secret_key"
            type="password"
            show-password
            :placeholder="tcSecretPlaceholder"
          />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.region')">
          <el-input
            v-model="mail.tencent_region"
            :placeholder="t('notifyCfg.regionPlaceholder')"
            style="width: 240px"
          />
        </el-form-item>
      </template>

      <!-- Mailgun -->
      <template v-else-if="mail.provider === 'mailgun'">
        <el-form-item :label="t('notifyCfg.mailgunApiKey')">
          <el-input
            v-model="mail.mailgun_api_key"
            type="password"
            show-password
            :placeholder="mgKeyPlaceholder"
          />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.mailgunDomain')">
          <el-input
            v-model="mail.mailgun_domain"
            :placeholder="t('notifyCfg.mailgunDomainPlaceholder')"
            clearable
          />
        </el-form-item>
        <el-form-item :label="t('notifyCfg.mailgunRegion')">
          <el-select v-model="mail.mailgun_region" style="width: 240px">
            <el-option value="us" :label="t('notifyCfg.regionUs')" />
            <el-option value="eu" :label="t('notifyCfg.regionEu')" />
          </el-select>
        </el-form-item>
      </template>

      <el-form-item>
        <el-button type="primary" :loading="saving" @click="save">
          {{ t('notifyCfg.saveMail') }}
        </el-button>
      </el-form-item>
    </el-form>
  </div>
</template>
