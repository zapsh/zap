<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script setup lang="ts">
import { onBeforeMount } from 'vue'
import { useRoute, useRouter } from 'vue-router'

const route = useRoute()
const router = useRouter()

onBeforeMount(() => {
  const { params, query } = route
  const { path } = params

  // 将数组路径转换为字符串
  const pathStr = Array.isArray(path) ? path.join('/') : path

  router.replace({
    path: '/' + pathStr,
    query,
  })
})
</script>

<template>
  <div></div>
</template>
