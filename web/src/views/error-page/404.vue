<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'

const router = useRouter()
const { t } = useI18n()

const goHome = () => {
  router.push('/')
}
</script>

<template>
  <div class="error-container">
    <div class="error-content">
      <div class="error-image">404</div>
      <h1 class="error-title">{{ t('app.notFoundTitle') }}</h1>
      <div class="error-desc">{{ t('app.notFoundDesc') }}</div>
      <el-button type="primary" @click="goHome">{{ t('app.backHome') }}</el-button>
    </div>
  </div>
</template>

<style scoped>
.error-container {
  width: 100%;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background-color: var(--el-bg-color-page);
}

.error-content {
  text-align: center;
  padding: 40px;
}

.error-image {
  font-size: 160px;
  font-weight: 600;
  color: var(--el-color-primary);
  line-height: 1;
  margin-bottom: 20px;
}

.error-title {
  font-size: 32px;
  color: var(--el-text-color-primary);
  margin: 20px 0;
  font-weight: 500;
}

.error-desc {
  font-size: 16px;
  color: var(--el-text-color-regular);
  margin-bottom: 30px;
}
</style>
