<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="sidebar-wrapper">
    <!-- Logo -->
    <div class="logo-container">
      <router-link to="/" class="logo-link">
        <el-icon class="logo-icon"><ElectricBoltOutline /></el-icon>
        <div v-if="!isCollapse" class="logo-title">ZAP</div>
      </router-link>
    </div>

    <!-- 菜单 -->
    <el-scrollbar>
      <el-menu
        :default-active="activeMenu"
        :default-openeds="defaultOpeneds"
        :collapse="isCollapse"
        :unique-opened="true"
        :collapse-transition="false"
        mode="vertical"
      >
        <sidebar-item
          v-for="route in permissionStore.menus"
          :key="route.path"
          :item="route"
          :base-path="route.path"
          :is-collapse="isCollapse"
        />
      </el-menu>
    </el-scrollbar>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { useAppStore } from '@/stores/app'
import { usePermissionStore } from '@/stores/permission'
import SidebarItem from './SidebarItem.vue'
import { ElectricBoltOutline } from '@/icons'

const route = useRoute()
const appStore = useAppStore()
const permissionStore = usePermissionStore()
// 是否折叠
const isCollapse = computed(() => !appStore.sidebar.opened)
// 当前激活的菜单
const activeMenu = computed<string>(() => {
  const { meta, path } = route
  // 如果设置了activeMenu，则使用activeMenu
  if (meta.activeMenu) {
    return meta.activeMenu as string
  }
  return path
})

// 当前激活菜单的父级路径，刷新/直接访问深层路由时自动展开分组
const defaultOpeneds = computed(() => {
  const parts = activeMenu.value.split('/').filter(Boolean)
  const res: string[] = []
  let acc = ''
  for (const p of parts.slice(0, -1)) {
    acc += `/${p}`
    res.push(acc)
  }
  return res
})
</script>

<style lang="scss" scoped>
.sidebar-wrapper {
  height: 100%;
  display: flex;
  flex-direction: column;
  background-color: #001529;
}

.logo-container {
  height: 50px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background-color: #002140;
  overflow: hidden;
}

.logo-link {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  height: 100%;
  width: 100%;
  text-decoration: none;
}

.logo-icon {
  font-size: 22px;
  color: #409eff;
}

.logo-title {
  color: #fff;
  font-size: 18px;
  font-weight: 700;
  letter-spacing: 1px;
  white-space: nowrap;
}

.el-scrollbar {
  flex: 1;
  // Element Plus 菜单 CSS 变量：统一暗色主题
  --el-menu-bg-color: transparent;
  --el-menu-text-color: #c8d4e2;
  --el-menu-hover-text-color: #fff;
  --el-menu-active-color: #fff;
  --el-menu-hover-bg-color: rgba(64, 158, 255, 0.12);
  --el-menu-item-height: 50px;
  --el-menu-sub-item-height: 44px;
  --el-menu-item-font-size: 14px;
  --el-menu-base-level-padding: 22px;
  --el-menu-level-padding: 22px;
  --el-menu-icon-width: 24px;
}

.el-menu {
  border-right: none;
}

:deep(.el-menu-item:hover),
:deep(.el-sub-menu__title:hover) {
  color: #fff;
}

:deep(.el-menu-item.is-active) {
  position: relative;
  background: linear-gradient(90deg, rgba(64, 158, 255, 0.25), rgba(64, 158, 255, 0.05));
  color: #fff;
  font-weight: 600;
}

:deep(.el-menu-item.is-active::before) {
  content: '';
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  height: 22px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background-color: #409eff;
  box-shadow: 0 0 8px rgba(64, 158, 255, 0.6);
}

:deep(.el-sub-menu.is-active > .el-sub-menu__title) {
  color: #fff;
}

/* 折叠状态：图标水平居中 */
:deep(.el-menu--collapse .el-menu-item .el-icon),
:deep(.el-menu--collapse .el-sub-menu__title .el-icon) {
  margin-right: 0;
}

:deep(.el-scrollbar__wrap) {
  overflow-x: hidden !important;
}
</style>
