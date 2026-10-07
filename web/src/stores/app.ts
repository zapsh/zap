// SPDX-License-Identifier: AGPL-3.0-only
import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useAppStore = defineStore('app', () => {
  const sidebar = ref({
    opened: true,
    withoutAnimation: false,
  })

  const device = ref('desktop')

  function toggleSidebar() {
    sidebar.value.opened = !sidebar.value.opened
    sidebar.value.withoutAnimation = false
  }

  function closeSidebar(withoutAnimation?: boolean) {
    sidebar.value.opened = false
    sidebar.value.withoutAnimation = withoutAnimation || false
  }

  function toggleDevice(val: string) {
    device.value = val
  }

  return {
    sidebar,
    device,
    toggleSidebar,
    closeSidebar,
    toggleDevice,
  }
})
