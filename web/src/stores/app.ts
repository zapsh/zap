// SPDX-License-Identifier: AGPL-3.0-only
import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useAppStore = defineStore('app', () => {
  const sidebar = ref({
    opened: true,
    withoutAnimation: false,
  })

  const device = ref('desktop')

  /** 强制 PC 模式：开启后即便窄屏也按桌面布局展示（侧栏常驻、不分盖式抽屉）。
   *  初值读 localStorage，保证首屏即生效；个人中心 → 偏好设置会同步到服务端。 */
  const forcePc = ref(localStorage.getItem('zap_force_pc') === '1')

  function setForcePc(v: boolean) {
    forcePc.value = v
    localStorage.setItem('zap_force_pc', v ? '1' : '0')
  }

  function toggleSidebar() {
    sidebar.value.opened = !sidebar.value.opened
    sidebar.value.withoutAnimation = false
  }

  function openSidebar() {
    sidebar.value.opened = true
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
    forcePc,
    setForcePc,
    toggleSidebar,
    openSidebar,
    closeSidebar,
    toggleDevice,
  }
})
