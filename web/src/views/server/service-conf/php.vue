<template>
  <div class="php-config">
    <el-card shadow="never" class="top-card">
      <div class="top-row">
        <div class="title">
          <span class="t-name">{{ t('servicesPhp.title') }}</span>
          <el-tag v-if="instances.length" size="small" type="info">
            {{ t('servicesPhp.versionsCount', { n: instances.length }) }}
          </el-tag>
        </div>
        <div class="actions">
          <span class="hint">
            {{ t('servicesPhp.hint') }}
          </span>
          <el-button size="small" :loading="loading" @click="load">{{
            t('servicesPhp.refresh')
          }}</el-button>
        </div>
      </div>
    </el-card>

    <el-alert
      v-if="hasEol"
      type="warning"
      :closable="false"
      show-icon
      class="mt-3"
      :title="t('servicesPhp.eolAlert')"
    />

    <template v-if="loading && !instances.length">
      <el-card shadow="never" class="mt-3">
        <el-skeleton :rows="5" animated />
      </el-card>
    </template>

    <el-card v-else-if="!instances.length" shadow="never" class="mt-3">
      <el-result icon="warning" :title="t('servicesPhp.noPhpTitle')" :sub-title="emptyHint">
        <template #extra>
          <el-button type="primary" @click="router.push('/appstore')">{{
            t('servicesPhp.goAppstore')
          }}</el-button>
        </template>
      </el-result>
    </el-card>

    <el-tabs v-else v-model="active" type="border-card" class="mt-3">
      <el-tab-pane v-for="inst in instances" :key="inst.svc" :name="inst.svc">
        <template #label>
          <span class="tab-label">
            {{ inst.svc }}
            <el-tag size="small" :type="inst.running ? 'success' : 'danger'" class="tab-tag">
              {{ inst.running ? t('servicesPhp.running') : t('servicesPhp.stopped') }}
            </el-tag>
            <el-tag v-if="inst.is_default" size="small" type="warning" class="tab-tag">
              {{ t('servicesPhp.defaultTag') }}
            </el-tag>
            <el-tag v-if="phpEol(inst)" size="small" type="danger" class="tab-tag">
              {{ t('servicesPhp.eolTag') }}
            </el-tag>
          </span>
        </template>
        <InstancePanel :key="inst.svc" :inst="inst" @changed="load" />
      </el-tab-pane>
    </el-tabs>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import InstancePanel from './php-instance.vue'
import { getServiceConfInstances, type ServiceConfInstance } from '@/api/servicesConf.ts'

const { t } = useI18n()
const router = useRouter()
const instances = ref<ServiceConfInstance[]>([])
const active = ref('')
const loading = ref(false)
/** 后端扫描 php-* 实例用的安装根目录（ZAP_APPS_DIR），用于排查"装了却识别不到" */
const appsDir = ref('')

const emptyHint = computed(() => {
  const base = t('servicesPhp.emptyHint')
  return appsDir.value ? t('servicesPhp.emptyHintDir', { base, dir: appsDir.value }) : base
})

/** PHP 版本解析为可比较整数（major*100+minor，如 7.4 → 704，8.1 → 801） */
function phpVerNum(v?: string): number {
  if (!v) return 0
  const m = v.match(/(\d+)\.(\d+)/)
  if (m) return parseInt(m[1], 10) * 100 + parseInt(m[2], 10)
  const s = v.match(/(\d)(\d)$/)
  if (s) return parseInt(s[1], 10) * 100 + parseInt(s[2], 10)
  return 0
}
/** 该 PHP 实例是否低于 PHP 8.1（PIE 要求 >= 8.1，不支持则提示源码编译 / EOL） */
function phpEol(inst: ServiceConfInstance): boolean {
  return phpVerNum(inst.version || inst.svc) < 801
}
const hasEol = computed(() => instances.value.some(phpEol))

async function load() {
  loading.value = true
  try {
    const res = await getServiceConfInstances('php')
    instances.value = res.data.instances
    appsDir.value = res.data.apps_dir || ''
    if (active.value && !instances.value.some((i) => i.svc === active.value)) {
      active.value = ''
    }
    if (!active.value && instances.value.length) {
      active.value = instances.value[0].svc
    }
  } catch {
    /* interceptor 已提示 */
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.php-config {
  min-height: 300px;
}
.top-card {
  border-radius: 8px;
}
.top-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.title {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.t-name {
  font-size: 16px;
  font-weight: 600;
}
.actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  max-width: 520px;
}
.mt-3 {
  margin-top: 12px;
}
.tab-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.tab-tag {
  margin-left: 2px;
}
</style>
