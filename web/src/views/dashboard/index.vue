<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="dashboard-container">
    <template v-if="roles.includes('admin')">
      <component :is="AdminDashboardAsync"></component>
    </template>
    <template v-else-if="roles.includes('reseller')">
      <component :is="ResellerDashboardAsync"></component>
    </template>
    <template v-else>
      <component :is="UserDashboardAsync"></component>
    </template>
  </div>
</template>

<script setup lang="ts">
import { defineAsyncComponent } from 'vue'
import { useUserStore } from '@/stores/user'

const userStore = useUserStore()
const roles = userStore.roles
const AdminDashboardAsync = defineAsyncComponent(() => import('@/views/dashboard/admin.vue'))
const ResellerDashboardAsync = defineAsyncComponent(() => import('@/views/dashboard/reseller.vue'))
const UserDashboardAsync = defineAsyncComponent(() => import('@/views/dashboard/user.vue'))
</script>

<style scoped>
.dashboard-container {
  padding: 20px;
}
</style>
