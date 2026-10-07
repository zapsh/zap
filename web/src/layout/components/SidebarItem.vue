<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div v-if="!isHiddenSelf && !allChildrenHidden">
    <!-- 没有子菜单的情况 -->
    <template
      v-if="
        hasOneShowingChild(item.children, item) &&
        (!onlyOneChild.children?.length || onlyOneChild.noShowingChildren) &&
        !item.alwaysShow
      "
    >
      <app-link v-if="onlyOneChild.meta" :to="resolvePath(onlyOneChild.path)">
        <el-menu-item :index="resolvePath(onlyOneChild.path)">
          <el-icon v-if="resolvedIcon">
            <Icon :icon="resolvedIcon" />
          </el-icon>
          <template #title>
            <span>{{ translateTitle(onlyOneChild.meta.title) }}</span>
          </template>
        </el-menu-item>
      </app-link>
    </template>

    <!-- 有子菜单的情况（折叠时浮层样式见全局 .el-menu--popup） -->
    <el-sub-menu v-else :index="resolvePath(item.path)">
      <template #title>
        <el-icon v-if="item.meta && item.meta.icon">
          <Icon :icon="item.meta.icon" />
        </el-icon>
        <span>{{ translateTitle(item.meta.title) }}</span>
      </template>

      <!-- 递归渲染子菜单 -->
      <sidebar-item
        v-for="child in item.children"
        :key="child.path"
        :item="child"
        :is-collapse="isCollapse"
        :base-path="resolvePath(child.path)"
      />
    </el-sub-menu>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { isExternal } from '@/utils/validate'
import AppLink from './AppLink.vue'
import path from 'path-browserify'
// 菜单标题来自后端菜单表（中文），由 translateTitle 按当前语言翻译
import { translateTitle } from '@/i18n'
// 离线图标：图标名为字符串，由 <Icon> 解析成对应图标组件（见 @/icons）
import { Icon } from '@/icons'

const props = defineProps({
  item: {
    type: Object,
    required: true,
  },
  isCollapse: {
    type: Boolean,
    default: false,
  },
  basePath: {
    type: String,
    default: '',
  },
})

// 唯一子菜单
const onlyOneChild = ref<any>(null)

const isHiddenSelf = computed(() => !!(props.item.meta && props.item.meta.hidden))

/**
 * 子菜单全部隐藏（如「系统设置」下只剩隐藏的 About ZAP）：父级也不该再显示，
 * 否则会渲染一个点进去什么都没有的空壳目录。
 */
const allChildrenHidden = computed(() => {
  const children = props.item.children || []
  if (!children.length) return false
  return children.every((c: any) => c.meta && c.meta.hidden)
})

// 单子菜单显示时：优先子菜单图标，缺失则回退父菜单图标。
// 菜单图标来自数据库，可能是不认识的名字，由 <Icon> 内部解析并兜底。
const resolvedIcon = computed(() => onlyOneChild.value?.meta?.icon || props.item.meta?.icon || '')

/**
 * 判断是否只有一个显示的子菜单
 */
const hasOneShowingChild = (children = [], parent: any) => {
  if (!children) {
    children = []
  }

  const showingChildren = children.filter((item: any) => {
    if (item.meta && item.meta.hidden) {
      return false
    } else {
      // 临时设置
      onlyOneChild.value = item
      return true
    }
  })

  // 当只有一个子路由时，默认显示子路由
  if (showingChildren.length === 1) {
    return true
  }

  // 没有子路由则显示父路由
  if (showingChildren.length === 0) {
    onlyOneChild.value = { ...parent, path: '', noShowingChildren: true }
    return true
  }

  return false
}

/**
 * 解析路径
 */
const resolvePath = (routePath: string) => {
  if (isExternal(routePath)) {
    return routePath
  }
  if (isExternal(props.basePath)) {
    return props.basePath
  }
  return path.resolve(props.basePath, routePath)
}
</script>
