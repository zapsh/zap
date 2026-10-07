<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <component :is="type" v-bind="linkProps">
    <slot />
  </component>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { isExternal } from '@/utils/validate'

const props = defineProps({
  to: {
    type: String,
    required: true,
  },
})

// 根据链接类型决定使用 <a> 标签还是 <router-link>
const type = computed(() => {
  if (isExternal(props.to)) {
    return 'a'
  }
  return 'router-link'
})

// 根据链接类型生成不同的属性
const linkProps = computed(() => {
  if (isExternal(props.to)) {
    return {
      href: props.to,
      target: '_blank',
      rel: 'noopener',
    }
  }
  return {
    to: props.to,
  }
})
</script>
