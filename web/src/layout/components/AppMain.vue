<template>
  <section class="app-main">
    <el-alert
      v-if="isDemo"
      type="warning"
      :closable="false"
      show-icon
      :title="t('layout.demoTip')"
      class="demo-banner"
    />
    <el-alert
      v-if="userStore.sudoMode"
      type="warning"
      :closable="false"
      show-icon
      class="sudo-banner"
    >
      <template #title>
        <span class="sudo-banner__text">{{
          t('layout.sudoBanner', { name: userStore.sudoUsername })
        }}</span>
        <el-button type="primary" size="small" @click="exitSudo">{{
          t('layout.sudoExit')
        }}</el-button>
      </template>
    </el-alert>
    <router-view v-slot="{ Component }">
      <transition name="fade-transform" mode="out-in">
        <component :is="Component" />
      </transition>
    </router-view>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessageBox } from 'element-plus'
import { useUserStore } from '@/stores/user'

const { t } = useI18n()

// AppMain component is a container for the router-view
const userStore = useUserStore()
const isDemo = computed(() => userStore.roles.includes('demo'))

/** 退出代登录：确认后恢复原操作员会话并重载 */
async function exitSudo() {
  try {
    await ElMessageBox.confirm(t('layout.sudoExitConfirm'), t('layout.sudoExit'), {
      type: 'warning',
      confirmButtonText: t('layout.sudoExit'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  userStore.stopSudo()
  window.location.reload()
}
</script>

<style scoped>
.app-main {
  padding: 20px;
  height: calc(100vh - 50px - 30px); /* 减去顶部导航栏和底部状态栏的高度 */
  overflow-y: auto;
  box-sizing: border-box;
  background-color: #f0f2f5;
}
.demo-banner {
  margin-bottom: 16px;
}
.sudo-banner {
  margin-bottom: 16px;
}
.sudo-banner :deep(.el-alert__title) {
  display: flex;
  align-items: center;
  gap: 12px;
}
.sudo-banner__text {
  font-weight: 500;
}

/* 页面切换动画 */
.fade-transform-enter-active,
.fade-transform-leave-active {
  transition: all 0.3s;
}

.fade-transform-enter-from {
  opacity: 0;
  transform: translateX(-30px);
}

.fade-transform-leave-to {
  opacity: 0;
  transform: translateX(30px);
}
</style>
