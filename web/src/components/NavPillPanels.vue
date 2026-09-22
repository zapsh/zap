<template>
  <div class="pill-panels">
    <!-- 一级 nav pill：只有一个面板时不渲染，避免多余的空导航 -->
    <div v-if="tabs.length > 1" class="panel-nav">
      <div class="nav-pills">
        <span
          v-for="it in tabs"
          :key="it.key"
          class="pill"
          :class="{ active: active === it.key }"
          @click="active = it.key"
        >
          <el-icon v-if="it.icon" class="pill-icon"><component :is="it.icon" /></el-icon>
          {{ it.label }}
        </span>
      </div>
      <span v-if="currentTab?.hint" class="nav-hint">{{ currentTab.hint }}</span>
    </div>

    <!-- 面板保持存活：切回来不丢滚动位置与查询条件；需要刷新的面板可自行 expose reload -->
    <KeepAlive>
      <component :is="currentTab?.panel" ref="panelRef" :key="active" />
    </KeepAlive>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch, type Component } from 'vue'
import { useRoute, useRouter } from 'vue-router'

export interface PillTab {
  /** 面板标识，同时作为地址栏 ?tab= 的值 */
  key: string
  label: string
  icon?: Component
  /** 该 pill 对应的面板组件 */
  panel: Component
  /** 选中该 pill 时在右侧显示的一句话说明，可选 */
  hint?: string
}

const props = defineProps<{
  tabs: PillTab[]
}>()

const route = useRoute()
const router = useRouter()

function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

/** 初始面板：优先地址栏 ?tab=，无效值（旧书签 / 手改参数）落到第一个 */
function initialKey(): string {
  const q = str(route.query.tab)
  return props.tabs.some((t) => t.key === q) ? q : (props.tabs[0]?.key ?? '')
}

const active = ref(initialKey())
const panelRef = ref<{ reload?: () => void }>()

const currentTab = computed(
  () => props.tabs.find((t) => t.key === active.value) ?? props.tabs[0],
)

// 切换时同步到地址栏：刷新、复制链接都能回到同一个面板
watch(active, async (v) => {
  if (route.query.tab !== v) {
    await router.replace({ query: { ...route.query, tab: v } })
  }
  // 面板若在另一个 pill 里刚被改过（角色、套餐…），切回来刷一次
  await nextTick()
  panelRef.value?.reload?.()
})

// 前进/后退等外部改地址栏的情况跟着切
watch(
  () => str(route.query.tab),
  (v) => {
    if (v && v !== active.value && props.tabs.some((t) => t.key === v)) active.value = v
  },
)

// tabs 受权限变化增减时（如非管理员看不到某个面板），把停在不存在的 pill 上的值拨回来
watch(
  () => props.tabs.map((t) => t.key).join(','),
  () => {
    if (!props.tabs.length) return
    const q = str(route.query.tab)
    const next = props.tabs.some((t) => t.key === q) ? q : props.tabs[0].key
    if (next !== active.value) active.value = next
  },
)
</script>

<script lang="ts">
export default { name: 'NavPillPanels' }
</script>

<style scoped>
.pill-panels {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.panel-nav {
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
