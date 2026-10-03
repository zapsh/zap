<template>
  <div class="market-page">
    <!-- 页头：面向租户的应用市场（仅 Web 应用 + 插件，可安装，scope 由包 roles 控制） -->
    <el-card shadow="never" class="head-card">
      <div class="head-row">
        <el-icon :size="22" color="#409eff"><Goods /></el-icon>
        <div>
          <div class="head-title">{{ t('appMarket.title') }}</div>
          <div class="head-sub">{{ t('appMarket.subtitle') }}</div>
        </div>
      </div>
    </el-card>

    <!-- 复用应用商店的安装能力，但只暴露 Web 应用 + 插件两类；
         安装/升级/卸载按包 app.yaml 的 roles 白名单（scope）门禁，非 admin 只能触发自己被授权的包 -->
    <AppStorePackages :categories="['webapps', 'plugins']" />
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Goods } from '@/icons'
import AppStorePackages from '@/views/appstore/AppStorePackages.vue'

const { t } = useI18n()
</script>

<style scoped>
.market-page {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.head-card :deep(.el-card__body) {
  padding: 14px 16px;
}
.head-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.head-title {
  font-size: 17px;
  font-weight: 600;
}
.head-sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
