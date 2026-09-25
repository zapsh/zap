<template>
  <div>
    <!-- 全局默认访问开关 -->
    <el-card shadow="never" class="default-card">
      <div class="default-row">
        <div class="default-main">
          <div class="default-title">
            <el-switch
              :model-value="inst.is_default"
              :loading="toggling"
              :disabled="toggling"
              @change="toggleDefault"
            />
            <span class="t-name">{{ t('servicesPhpPanel.defaultAccess') }}</span>
            <el-tag v-if="inst.is_default" type="success" size="small">{{
              t('servicesPhpPanel.systemDefault')
            }}</el-tag>
            <el-tag v-else size="small" type="info">{{
              t('servicesPhpPanel.notRegistered')
            }}</el-tag>
          </div>
          <div class="default-desc">
            {{ t('servicesPhpPanel.defaultDesc1') }}<code>php / php-cgi / pear / pecl</code
            >{{ t('servicesPhpPanel.defaultDesc2') }} <code>/usr/local/bin</code
            >{{ t('servicesPhpPanel.defaultDesc3') }}<code>php</code
            >{{ t('servicesPhpPanel.defaultDesc4', { version: inst.version || inst.svc }) }}
          </div>
        </div>
      </div>
    </el-card>

    <!-- 实例配置（关键项 / 配置文件 / 启停控制，svc = 实例名） -->
    <ServiceConfPage
      :service="inst.svc"
      :label="`PHP ${inst.version || inst.svc}`"
      :desc="descText"
      :install-hint="t('servicesPhpPanel.installHint')"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import ServiceConfPage from '../shared/ServiceConfPage.vue'
import { setServiceConfDefault, type ServiceConfInstance } from '@/api/servicesConf.ts'

const props = defineProps<{
  inst: ServiceConfInstance
}>()

const emit = defineEmits<{
  (e: 'changed'): void
}>()

const { t } = useI18n()
const toggling = ref(false)

const descText = computed(() => {
  const dir = props.inst.dir
  const file = props.inst.conf_file
  const parts: string[] = []
  if (dir) parts.push(t('servicesCommon.installDir', { path: dir }))
  if (file) parts.push(t('servicesCommon.confPath', { path: file }))
  if (props.inst.unit) parts.push(t('servicesCommon.unitPath', { path: props.inst.unit }))
  const prefix = parts.length
    ? parts.join(t('servicesPhpPanel.separator')) + t('servicesPhpPanel.partsSuffix')
    : ''
  return prefix + t('servicesPhpPanel.descSuffix')
})

async function toggleDefault(next: boolean) {
  if (toggling.value) return
  if (next === props.inst.is_default) return
  const version = props.inst.version || props.inst.svc
  const tip = next
    ? t('servicesPhpPanel.confirmOn', { version })
    : t('servicesPhpPanel.confirmOff', { version })
  try {
    await ElMessageBox.confirm(tip, t('common.tip'), { type: next ? 'warning' : 'info' })
  } catch {
    return
  }
  toggling.value = true
  try {
    const res = await setServiceConfDefault(props.inst.svc, next)
    ElMessage.success(
      res.data?.registered
        ? t('servicesPhpPanel.registered', { names: res.data.registered.join(' / ') })
        : res.data?.removed
          ? t('servicesPhpPanel.unregistered', { names: res.data.removed.join(' / ') })
          : res.message || t('servicesCommon.opSuccess'),
    )
    emit('changed')
  } catch {
    /* interceptor 已提示 */
  } finally {
    toggling.value = false
  }
}
</script>

<style scoped>
.default-card {
  border-radius: 8px;
  margin-bottom: 12px;
}
.default-row {
  display: flex;
  align-items: flex-start;
}
.default-main {
  min-width: 0;
  flex: 1;
}
.default-title {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.t-name {
  font-size: 15px;
  font-weight: 600;
}
.default-desc {
  margin-top: 8px;
  font-size: 12px;
  line-height: 1.7;
  color: var(--el-text-color-secondary);
}
.default-desc code {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
  color: var(--el-color-primary);
  background: var(--el-fill-color-light);
  border-radius: 3px;
  padding: 0 4px;
}
</style>
