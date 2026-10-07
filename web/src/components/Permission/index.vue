<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { computed } from 'vue'
import { useUserStore } from '@/stores/user'

const props = defineProps({
  // 权限标识
  permission: {
    type: [String, Array],
    default: null,
  },
  // 角色标识
  role: {
    type: [String, Array],
    default: null,
  },
  // 权限模式：allOf(且)、oneOf(或)
  mode: {
    type: String,
    default: 'oneOf',
    validator: (value: string) => ['allOf', 'oneOf'].includes(value),
  },
})

const userStore = useUserStore()
const { permissions, roles } = userStore.userInfo

// 检查是否有权限
const hasPermission = computed(() => {
  if (!props.permission) return true

  const requiredPermissions = Array.isArray(props.permission)
    ? props.permission
    : [props.permission]

  if (props.mode === 'allOf') {
    return requiredPermissions.every((permission) => permissions.includes(permission as string))
  } else {
    return requiredPermissions.some((permission) => permissions.includes(permission as string))
  }
})

// 检查是否有角色
const hasRole = computed(() => {
  if (!props.role) return true

  const requiredRoles = Array.isArray(props.role) ? props.role : [props.role]

  if (props.mode === 'allOf') {
    return requiredRoles.every((role) => roles.includes(role as string))
  } else {
    return requiredRoles.some((role) => roles.includes(role as string))
  }
})

// 是否有权限显示
const show = computed(() => {
  return hasPermission.value && hasRole.value
})
</script>

<template>
  <template v-if="show">
    <slot></slot>
  </template>
</template>
