<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-breadcrumb class="app-breadcrumb" separator="/">
    <transition-group name="breadcrumb">
      <el-breadcrumb-item v-for="(item, index) in breadcrumbs" :key="item.path">
        <span v-if="index === breadcrumbs.length - 1" class="no-redirect">{{
          translateTitle(item.meta.title as string)
        }}</span>
        <a v-else @click.prevent="handleLink(item)">{{
          translateTitle(item.meta.title as string)
        }}</a>
      </el-breadcrumb-item>
    </transition-group>
  </el-breadcrumb>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { useRoute, useRouter, type RouteLocationMatched } from 'vue-router'
import { translateTitle } from '@/i18n'

const route = useRoute()
const router = useRouter()

const breadcrumbs = ref<RouteLocationMatched[]>([])

// 过滤路由
const getBreadcrumb = () => {
  let matched = route.matched.filter((item) => item.meta && item.meta.title)
  // 如果第一个不是首页，那么就把首页放在第一个
  const first = matched[0]
  if (first && first.path !== '/dashboard') {
    matched = [
      {
        // 用 i18n key：translateTitle 命中语言包后按当前语言渲染
        path: '/dashboard',
        meta: { title: 'menu.dashboard' },
      } as unknown as RouteLocationMatched,
    ].concat(matched)
  }
  breadcrumbs.value = matched
}

// 点击面包屑导航
const handleLink = (item: RouteLocationMatched) => {
  const { path } = item
  router.push(path)
}

watch(
  () => route.path,
  () => getBreadcrumb(),
  {
    immediate: true,
  },
)
</script>

<style lang="scss" scoped>
.app-breadcrumb {
  display: inline-block;
  font-size: 14px;
  line-height: 50px;
  margin-left: 8px;

  .no-redirect {
    color: var(--el-text-color-placeholder);
    cursor: text;
  }

  a {
    color: var(--el-text-color-regular);
    cursor: pointer;
    &:hover {
      color: var(--el-color-primary);
    }
  }
}

.breadcrumb-enter-active,
.breadcrumb-leave-active {
  transition: all 0.5s;
}

.breadcrumb-enter-from,
.breadcrumb-leave-active {
  opacity: 0;
  transform: translateX(20px);
}

.breadcrumb-leave-to {
  opacity: 0;
  transform: translateX(-20px);
}
</style>
