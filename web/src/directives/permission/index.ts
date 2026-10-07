// SPDX-License-Identifier: AGPL-3.0-only
import type { DirectiveBinding } from 'vue'
import { useUserStore } from '@/stores/user'

/**
 * 权限指令
 * 用法：
 * 单个权限: v-permission="'user:add'"
 * 多个权限: v-permission="['user:add', 'user:edit']"
 * 角色权限: v-permission-role="'admin'"
 */

function checkPermission(el: HTMLElement, binding: DirectiveBinding) {
  const userStore = useUserStore()
  const { value } = binding
  const { permissions } = userStore.userInfo

  if (value && value instanceof Array && value.length > 0) {
    const permissionRoles = value
    const hasPermission = permissions.some((permission) => {
      return permissionRoles.includes(permission)
    })

    if (!hasPermission) {
      el.parentNode && el.parentNode.removeChild(el)
    }
  } else {
    throw new Error(`need permissions! Like v-permission="['user:add','user:edit']"`)
  }
}

function checkRole(el: HTMLElement, binding: DirectiveBinding) {
  const userStore = useUserStore()
  const { value } = binding
  const { roles } = userStore.userInfo

  if (value && typeof value === 'string') {
    const roleRequired = value
    const hasRole = roles.includes(roleRequired)

    if (!hasRole) {
      el.parentNode && el.parentNode.removeChild(el)
    }
  } else {
    throw new Error(`need role! Like v-permission-role="'admin'"`)
  }
}

export default {
  // 注册v-permission指令
  permission: {
    mounted(el: HTMLElement, binding: DirectiveBinding) {
      checkPermission(el, binding)
    },
    updated(el: HTMLElement, binding: DirectiveBinding) {
      checkPermission(el, binding)
    },
  },
  // 注册v-permission-role指令
  permissionRole: {
    mounted(el: HTMLElement, binding: DirectiveBinding) {
      checkRole(el, binding)
    },
    updated(el: HTMLElement, binding: DirectiveBinding) {
      checkRole(el, binding)
    },
  },
}
