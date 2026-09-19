<template>
  <div class="team-container">
    <TeamPanel />
  </div>
</template>

<script setup lang="ts">
// 团队成员已并入个人中心（顶栏头像 → 个人中心 → 团队成员），
// 侧边栏不再放入口；这里保留路由，避免旧链接 / 收藏直接 404。
import TeamPanel from './TeamPanel.vue'
</script>

<style scoped>
.team-container {
  padding: 20px;
}
</style>
