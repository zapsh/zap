<template>
  <div class="access-page">
    <!-- 一级 nav pill：用户 / 角色。只有一个可见时不显示，避免多余的空导航 -->
    <div v-if="tabs.length > 1" class="page-nav">
      <div class="nav-pills">
        <span
          v-for="it in tabs"
          :key="it.key"
          class="pill"
          :class="{ active: tab === it.key }"
          @click="tab = it.key"
        >
          <el-icon class="pill-icon"><component :is="it.icon" /></el-icon>
          {{ it.label }}
        </span>
      </div>
      <span class="nav-hint">{{ tab === 'users' ? t('users.titleHint') : t('roles.titleHint') }}</span>
    </div>

    <KeepAlive>
      <component :is="tab === 'users' ? UsersPanel : RolesPanel" ref="panelRef" :key="tab" />
    </KeepAlive>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { User, Lock } from '@/icons'
import { useUserStore } from '@/stores/user'
import UsersPanel from './UsersPanel.vue'
import RolesPanel from './RolesPanel.vue'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const userStore = useUserStore()

/** 角色管理只有 admin 能进：reseller 只看到用户（客户）面板 */
const isAdmin = computed(() => userStore.roles.includes('admin'))

const tabs = computed(() => {
  const list: { key: 'users' | 'roles'; label: string; icon: unknown }[] = [
    { key: 'users', label: t('users.title'), icon: User },
  ]
  if (isAdmin.value) list.push({ key: 'roles', label: t('roles.title'), icon: Lock })
  return list
})

/**
 * 当前面板。URL 带 ?tab=roles 时直接落到角色页（书签 / 菜单直达都靠它）。
 */
const tab = ref<'users' | 'roles'>(
  route.query.tab === 'roles' && isAdmin.value ? 'roles' : 'users',
)

// 切面板时同步到地址栏，刷新与分享链接都能回到同一页
watch(tab, (v) => {
  router.replace({ query: { ...route.query, tab: v } })
})

const panelRef = ref<{ reload?: () => void }>()

// 切回来时刷新一次：另一个面板里可能刚改过角色 / 用户
watch(tab, async (v) => {
  if (!v) return
  await nextTick()
  panelRef.value?.reload?.()
})

/** 非 admin 不允许停在角色页（直接改地址栏也要落回用户页） */
watch(isAdmin, (ok) => {
  if (!ok && tab.value === 'roles') tab.value = 'users'
})
</script>

<script lang="ts">
export default { name: 'SystemAccess' }
</script>

<style scoped>
.access-page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.page-nav {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  padding: 0 4px;
}

.nav-pills {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px;
  border-radius: 10px;
  background: var(--el-fill-color-light);
}

.nav-pills .pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  padding: 0 14px;
  border-radius: 8px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  transition:
    background-color 0.15s,
    color 0.15s;
}

.nav-pills .pill:hover {
  background: var(--el-fill-color);
}

.nav-pills .pill.active {
  background: var(--el-color-primary);
  color: #fff;
}

.pill-icon {
  font-size: 15px;
}

.nav-hint {
  font-size: 12px;
  color: var(--el-text-color-placeholder);
}
</style>
