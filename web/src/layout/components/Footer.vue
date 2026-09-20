<template>
  <div class="footer">
    <span class="app-name">ZAP</span>
    <span class="sep">·</span>
    <span
      class="version verlink"
      :class="{ 'no-cursor': !canGoUpdate }"
      :title="canGoUpdate ? t('layout.goUpdateTip') : ''"
      @click="goUpdate"
    >
      v{{ APP_VERSION }}
    </span>
    <template v-if="WEB_VERSION">
      <span class="sep">·</span>
      <span
        class="web-version verlink"
        :class="{ 'no-cursor': !canGoUpdate }"
        :title="canGoUpdate ? t('layout.goUpdateTip') : ''"
        @click="goUpdate"
      >
        Web v{{ WEB_VERSION }}
      </span>
    </template>
    <!-- 文档：入口已整合进「系统设置 → About ZAP」，这里只留常驻直达链接 -->
    <span class="sep">·</span>
    <span class="docs-link verlink" @click="goAbout">{{ t('layout.docs') }}</span>
    <span class="copyright">© {{ year }}</span>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useUserStore } from '@/stores/user'
import { getUpdateStatus } from '@/api/systemUpdate'

const { t } = useI18n()

// zap 版本：优先展示**运行时**版本（zapd 实际运行的二进制版本），
// 取不到时回退到构建时注入的 VITE_APP_VERSION（根 Cargo.toml [workspace.package]）。
// 原因：前端产物由 zapd 通过 rust-embed 内嵌（../web/dist），
// 若发布时未重新构建前端，构建版本号会落后于本次发布的二进制版本。
const BUILD_VERSION = import.meta.env.VITE_APP_VERSION || ''
const runtimeVersion = ref('')
const APP_VERSION = computed(() => runtimeVersion.value || BUILD_VERSION)
// Web 版本：前端包自身版本（构建时从 web/package.json 注入），与 Zap 版本独立
const WEB_VERSION = import.meta.env.VITE_WEB_VERSION || ''

const year = new Date().getFullYear()

const router = useRouter()
const userStore = useUserStore()

/** 系统更新页已并入 About ZAP（页签 tab=update），仍仅 admin 可见：非管理员不提供跳转 */
const canGoUpdate = computed(() => userStore.roles.includes('admin'))

onMounted(async () => {
  // /system/update/status 仅 admin 可访问：非管理员直接用构建时版本，避免 403 噪音
  if (!canGoUpdate.value) return
  try {
    const res = await getUpdateStatus()
    const v = res.data?.zapd_version
    if (v) runtimeVersion.value = v
  } catch {
    // 忽略：保留构建时注入的版本
  }
})

function goUpdate() {
  if (canGoUpdate.value) router.push('/system/about?tab=update')
}

/** 文档入口（系统设置 → About ZAP）：所有角色可见 */
function goAbout() {
  router.push('/system/about')
}
</script>

<style scoped>
.footer {
  height: 50px;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 6px;
  flex-shrink: 0;
  /* 用 element-plus 变量，自动适配明暗主题 */
  color: var(--el-text-color-secondary);
  border-top: 1px solid var(--el-border-color-lighter);
  font-size: 13px;
  padding: 0 12px;
}

.version {
  color: var(--el-color-primary);
  font-weight: 600;
}

.sep {
  opacity: 0.6;
}

.verlink {
  cursor: pointer;
  transition: text-decoration 0.1s;
}

.verlink:hover {
  text-decoration: underline;
}

.no-cursor {
  cursor: default;
}

.docs-link {
  color: var(--el-color-primary);
}

.copyright {
  margin-left: 4px;
}
</style>
