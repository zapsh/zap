<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <el-breadcrumb class="app-breadcrumb" separator="/">
    <transition-group name="breadcrumb">
      <el-breadcrumb-item v-for="(item, index) in breadcrumbs" :key="item.path">
        <span
          v-if="item.redirect === 'noRedirect' || index === breadcrumbs.length - 1"
          class="no-redirect"
          >{{ translateTitle(item.meta.title as string) }}</span
        >
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
import { compile } from 'path-to-regexp'
import { translateTitle } from '@/i18n'

const route = useRoute()
const router = useRouter()

// 面包屑数据
const breadcrumbs = ref<RouteLocationMatched[]>([])

// 获取面包屑数据
const getBreadcrumb = () => {
  // 过滤掉没有 meta.title 的路由
  let matched = route.matched.filter((item) => item.meta && item.meta.title)

  // 如果第一个不是首页，则添加首页
  const first = matched[0]
  if (first && first.path !== '/dashboard') {
    matched = [
      {
        path: '/dashboard',
        meta: { title: '首页' },
      } as unknown as RouteLocationMatched,
    ].concat(matched)
  }

  breadcrumbs.value = matched
}

// 处理链接点击
const handleLink = (item: RouteLocationMatched) => {
  const { redirect, path } = item

  // 如果有重定向，则跳转到重定向的路由
  if (redirect) {
    router.push(redirect.toString())
    return
  }

  // 否则跳转到路径
  router.push(pathCompile(path))
}

// 编译路径，替换路径中的参数
const pathCompile = (path: string) => {
  const { params } = route
  const toPath = compile(path)
  return toPath(params)
}

// 监听路由变化
watch(
  () => route.path,
  () => {
    getBreadcrumb()
  },
  { immediate: true },
)
</script>

<style scoped>
.app-breadcrumb.el-breadcrumb {
  display: inline-block;
  font-size: 14px;
  line-height: 50px;
  margin-left: 8px;
}

.app-breadcrumb.el-breadcrumb .no-redirect {
  color: #97a8be;
  cursor: text;
}

/* 面包屑动画 */
.breadcrumb-enter-active,
.breadcrumb-leave-active {
  transition: all 0.5s;
}

.breadcrumb-enter-from,
.breadcrumb-leave-to {
  opacity: 0;
  transform: translateX(20px);
}

.breadcrumb-leave-active {
  position: absolute;
}
</style>
