<template>
  <div class="svc-docker-page">
    <DockerSettings />
  </div>
</template>

<script setup lang="ts">
// Docker 服务配置的主体已抽到「容器管理」页面右上角的「设置」里，
// 这里保留旧菜单入口，直接复用同一个组件，两处行为始终一致。
import DockerSettings from '@/views/docker/components/DockerSettings.vue'
</script>

<style scoped>
.svc-docker-page {
  padding: 16px;
}
</style>
