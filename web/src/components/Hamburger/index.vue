<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div style="padding: 0 15px" @click="toggleClick">
    <el-icon :class="{ 'is-active': isActive }" class="hamburger-icon">
      <Fold />
    </el-icon>
  </div>
</template>

<script setup lang="ts">
import { Fold } from '@/icons'

defineProps({
  isActive: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['toggleClick'])

const toggleClick = () => {
  emit('toggleClick')
}
</script>

<style lang="scss" scoped>
.hamburger-icon {
  vertical-align: middle;
  font-size: 20px;
  cursor: pointer;
  transition: transform 0.3s;

  &.is-active {
    transform: rotate(180deg);
  }
}
</style>
