<template>
  <div class="mirror-config">
    <el-card v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('mirrorCfg.title') }}</span>
          <span class="sub">{{ t('mirrorCfg.subtitle') }}</span>
        </div>
      </template>

      <el-alert
        type="info"
        :closable="false"
        show-icon
        :title="t('mirrorCfg.hint')"
        style="margin-bottom: 16px"
      />

      <el-form :model="form" label-width="140px" style="max-width: 720px" @submit.prevent>
        <el-form-item :label="t('mirrorCfg.source')">
          <el-select
            v-model="form.pkgMirror"
            filterable
            allow-create
            default-first-option
            style="width: 420px"
          >
            <el-option v-for="p in presets" :key="p.value" :label="p.label" :value="p.value" />
          </el-select>
          <div class="hint">{{ t('mirrorCfg.sourceHint') }}</div>
          <!-- 填本地目录时把要求说在前面：目录得在面板所在机器上存在，
               存在性只有后端知道，这里先提醒，省一次「保存 → 报错」的往返 -->
          <div v-if="looksLocal" class="hint warn">{{ t('mirrorCfg.localPending') }}</div>
          <div v-else-if="looksBad" class="hint bad">{{ t('mirrorCfg.badFormat') }}</div>
        </el-form-item>

        <el-form-item :label="t('mirrorCfg.current')">
          <el-tag :type="isLocal ? 'warning' : 'success'" size="small">
            {{ isLocal ? t('mirrorCfg.local') : t('mirrorCfg.remote') }}
          </el-tag>
          <span class="preview">{{ pkgMirror || '-' }}</span>
        </el-form-item>

        <el-form-item :label="t('mirrorCfg.filePath')">
          <span class="preview">{{ configPath || '-' }}</span>
        </el-form-item>

        <el-form-item>
          <el-button type="primary" :loading="saving" @click="save">
            {{ t('mirrorCfg.save') }}
          </el-button>
          <el-button size="small" @click="load">{{ t('common.refresh') }}</el-button>
        </el-form-item>
      </el-form>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { getMirror, saveMirror } from '@/api/systemMirror'
import type { MirrorPreset } from '@/api/systemMirror'

/**
 * 「系统设置 → 下载源」：应用商店取源码包（nginx / php / mysql / …）的镜像 base。
 *
 * 三种形态：
 * - 国内镜像（默认）与 Cloudflare 镜像：下拉直接选；
 * - 本地目录（离线机房）：在下拉里直接输入绝对路径，如 /opt/zap-pkg。
 *   目录结构要与镜像一致（/opt/zap-pkg/php/php-8.3.6.tar.gz），且必须预先存在。
 */
const { t } = useI18n()

const loading = ref(false)
const saving = ref(false)
const pkgMirror = ref('')
const configPath = ref('')
const isLocal = ref(false)
const presets = ref<MirrorPreset[]>([])
const form = ref({ pkgMirror: '' })

/** 看起来是本地目录源（`/…` 或 `file://…`） */
const looksLocal = computed(() => /^(file:\/\/|\/)/.test(form.value.pkgMirror.trim()))
/** 非空但哪一类都不是：http(s) 不是、本地路径也不是 */
const looksBad = computed(() => {
  const v = form.value.pkgMirror.trim()
  return v.length > 0 && !looksLocal.value && !/^https?:\/\//.test(v)
})

async function load() {
  loading.value = true
  try {
    const res = await getMirror()
    const d = res.data
    pkgMirror.value = d.pkgMirror || ''
    configPath.value = d.path || ''
    isLocal.value = !!d.isLocal
    presets.value = d.presets ?? []
    form.value.pkgMirror = pkgMirror.value
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

async function save() {
  const base = form.value.pkgMirror.trim()
  if (!base) {
    ElMessage.warning(t('mirrorCfg.empty'))
    return
  }
  // 格式先自己挡一道：这类错误后端也会拒，但没必要为它跑一趟请求
  if (!looksLocal.value && !/^https?:\/\//.test(base)) {
    ElMessage.warning(t('mirrorCfg.badFormat'))
    return
  }
  saving.value = true
  try {
    const res = await saveMirror(base)
    const saved = res.data?.pkgMirror || base
    pkgMirror.value = saved
    form.value.pkgMirror = saved
    // 本地目录由后端统一规范成 file:// 前缀，这里按它回显类型
    isLocal.value = saved.startsWith('file://')
    ElMessage.success(res.message || t('mirrorCfg.saved'))
  } catch (e) {
    // 业务错误（目录不存在 / 不合法）走 HTTP 200 + code≠0，拦截器不弹窗，
    // 得自己把后端那句「本地目录不存在或不是目录: /opt/xxx」摆出来 ——
    // 那句话里带着路径和原因，正是运维要看到的东西。
    const msg = e instanceof Error ? e.message : ''
    ElMessage({
      message: msg || t('mirrorCfg.saveFailed'),
      type: 'error',
      duration: 6000,
      showClose: true,
    })
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<script lang="ts">
export default { name: 'MirrorConfig' }
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  gap: 8px;
}
.sub {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.hint {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  margin-top: 4px;
  line-height: 1.6;
}
.hint.warn {
  color: var(--el-color-warning);
}
.hint.bad {
  color: var(--el-color-danger);
}
.preview {
  margin-left: 8px;
  font-family: var(--el-font-family-monospace, monospace);
  font-size: 12px;
  color: var(--el-text-color-regular);
  word-break: break-all;
}
</style>
