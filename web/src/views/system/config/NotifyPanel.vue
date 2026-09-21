<template>
  <div class="notify-config">
    <el-card v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('notifyCfg.title') }}</span>
          <span class="sub">{{ t('notifyCfg.subtitle') }}</span>
        </div>
      </template>

      <el-tabs v-model="activeTab">
        <!-- ── Mail ─────────────────────────────────────── -->
        <el-tab-pane :label="t('notifyCfg.tabMail')" name="mail">
          <el-alert
            type="info"
            :closable="false"
            show-icon
            :title="t('notifyCfg.mailHint')"
            style="margin-bottom: 16px"
          />
          <el-form :model="mail" label-width="150px" style="max-width: 660px" @submit.prevent>
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
            <el-form-item :label="t('notifyCfg.from')">
              <el-input
                v-model="mail.from"
                :placeholder="t('notifyCfg.fromPlaceholder')"
                clearable
              />
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
                :placeholder="mailPasswordPlaceholder"
              />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingMail" @click="saveMail">
                {{ t('notifyCfg.saveMail') }}
              </el-button>
            </el-form-item>
          </el-form>
        </el-tab-pane>
      </el-tabs>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { getMailSettings, saveMailSettings } from '@/api/systemNotify'

const { t } = useI18n()

// 目前只有 Mail 一个渠道；后续接短信 / Webhook 时在这里加一个 el-tab-pane 即可
const activeTab = ref('mail')
const loading = ref(false)
const savingMail = ref(false)

const mail = reactive({
  host: '',
  port: '587',
  encryption: 'tls',
  from: '',
  username: '',
  password: '',
})

// 已保存的密码只回显掩码提示（后端返回 password_hint），留空提交即表示不修改
const mailPasswordSet = ref(false)
const mailPasswordHint = ref('')
const mailPasswordPlaceholder = computed(() =>
  mailPasswordSet.value
    ? t('notifyCfg.passwordKeepHint', { hint: mailPasswordHint.value || t('notifyCfg.saved') })
    : t('notifyCfg.passwordPlaceholder'),
)

// 协议名保持英文，只有「无加密」需要本地化
const encryptionOptions = computed(() => [
  { value: 'ssl', label: 'SSL / TLS (465)' },
  { value: 'tls', label: 'STARTTLS (587)' },
  { value: 'none', label: t('notifyCfg.encNone') },
])

async function load() {
  loading.value = true
  try {
    const res = await getMailSettings()
    const m = res.data.mail
    mail.host = m.host ?? ''
    mail.port = m.port || '587'
    mail.encryption = m.encryption || 'tls'
    mail.from = m.from ?? ''
    mail.username = m.username ?? ''
    mail.password = '' // 密码不回显（后端仅返回是否已设置 + 掩码）
    mailPasswordSet.value = m.password_set === true
    mailPasswordHint.value = m.password_hint ?? ''
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

async function saveMail() {
  const port = String(mail.port).trim()
  const num = Number(port)
  if (mail.host.trim() && (!Number.isInteger(num) || num < 1 || num > 65535)) {
    ElMessage.warning(t('notifyCfg.invalidPort'))
    return
  }
  savingMail.value = true
  try {
    await saveMailSettings({
      host: mail.host.trim(),
      port: port || '',
      encryption: mail.encryption,
      from: mail.from.trim(),
      username: mail.username.trim(),
      password: mail.password.trim(),
    })
    mail.password = ''
    ElMessage.success(t('notifyCfg.mailSaved'))
    await load() // 重新拉取，刷新已保存密码的掩码提示
  } catch {
    /* handled */
  } finally {
    savingMail.value = false
  }
}

onMounted(load)
</script>

<script lang="ts">
export default { name: 'NotifyConfigPanel' }
</script>

<style scoped>
.notify-config {
  padding: 4px;
}
.card-header {
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.card-header .sub {
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
</style>
