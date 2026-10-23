<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="pill-panels">
    <!-- 侧边栏布局：面板较多（默认 > 6）时自动切换，避免顶部 pill 横向过度增长 -->
    <div v-if="resolvedLayout === 'sidebar'" class="side-layout">
      <div class="side-nav">
        <button
          v-for="it in tabs"
          :key="it.key"
          class="side-item"
          :class="{ active: active === it.key }"
          type="button"
          @click="active = it.key"
        >
          <el-icon v-if="it.icon" class="side-icon"><component :is="it.icon" /></el-icon>
          <span class="side-label">{{ it.label }}</span>
        </button>
      </div>
      <div class="side-panel">
        <KeepAlive>
          <component :is="currentTab?.panel" ref="panelRef" :key="active" v-bind="currentTab?.props || {}" v-on="currentTab?.on || {}" />
        </KeepAlive>
      </div>
    </div>

    <!-- 顶部 pill 布局（默认） -->
    <template v-else>
      <!-- 一级 nav pill：只有一个面板时不渲染，避免多余的空导航 -->
      <div v-if="tabs.length > 1" class="panel-nav">
        <div ref="pillsEl" class="nav-pills">
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
        <component :is="currentTab?.panel" ref="panelRef" :key="active" v-bind="currentTab?.props || {}" v-on="currentTab?.on || {}" />
      </KeepAlive>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch, type Component } from 'vue'
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
  /** 透传给面板组件的额外 props（如自定义服务页的 service / label） */
  props?: Record<string, unknown>
  /** 透传给面板组件的事件监听器（如自定义服务管理页的 changed） */
  on?: Record<string, (...args: unknown[]) => void>
}

const props = defineProps<{
  tabs: PillTab[]
  /**
   * 布局模式：
   * - `pill`：(默认) 顶部分段导航，横向溢出时容器内滑动
   * - `sidebar`：左侧竖向导航 + 右侧面板
   * - `auto`：面板数量 ≤ `sidebarThreshold` 用 pill，否则自动切 sidebar（解决横向增长）
   */
  layout?: 'pill' | 'sidebar' | 'auto'
  /** auto 模式下切侧边栏的阈值（默认 6） */
  sidebarThreshold?: number
}>()

const route = useRoute()
const router = useRouter()

function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

/** 自动模式下：超过阈值用侧边栏，否则顶部 pill */
const resolvedLayout = computed<'pill' | 'sidebar'>(() => {
  if (props.layout === 'sidebar') return 'sidebar'
  if (props.layout === 'pill') return 'pill'
  return (props.tabs.length || 0) > (props.sidebarThreshold ?? 6) ? 'sidebar' : 'pill'
})

/** 初始面板：优先地址栏 ?tab=，无效值（旧书签 / 手改参数）落到第一个 */
function initialKey(): string {
  const q = str(route.query.tab)
  return props.tabs.some((t) => t.key === q) ? q : (props.tabs[0]?.key ?? '')
}

const active = ref(initialKey())
const panelRef = ref<{ reload?: () => void }>()
const pillsEl = ref<HTMLElement | null>(null)

/** 窄屏下 pill 行可横向滑动：把激活 pill 滚到可视区中间（容器内滚动，不动页面） */
function scrollActiveIntoView() {
  const wrap = pillsEl.value
  if (!wrap) return
  const el = wrap.querySelector('.pill.active') as HTMLElement | null
  if (!el) return
  const target = el.offsetLeft - (wrap.clientWidth - el.offsetWidth) / 2
  wrap.scrollTo({ left: Math.max(0, target), behavior: 'smooth' })
}

onMounted(scrollActiveIntoView)

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
  // 移动端窄屏：确保新激活的 pill 可见
  scrollActiveIntoView()
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
  /* 窄屏溢出时容器内横向滑动（触摸可滑，滚动条隐藏），不再把 pill 挤变形/裁掉 */
  max-width: 100%;
  overflow-x: auto;
  scrollbar-width: none;
}

.nav-pills::-webkit-scrollbar {
  display: none;
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
  /* 不被容器压缩：完整宽度靠横向滚动查看 */
  flex: none;
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

/* ── 侧边栏布局 ───────────────────────────── */
.side-layout {
  display: flex;
  gap: 14px;
  align-items: flex-start;
}

.side-nav {
  flex: 0 0 200px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px;
  border-radius: 10px;
  background: var(--el-fill-color-light);
  position: sticky;
  top: 12px;
}

.side-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 9px 12px;
  border: none;
  border-radius: 8px;
  background: transparent;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  cursor: pointer;
  text-align: left;
  transition:
    background-color 0.15s,
    color 0.15s;
}

.side-item:hover {
  background: var(--el-fill-color);
}

.side-item.active {
  background: var(--el-color-primary);
  color: #fff;
}

.side-icon {
  font-size: 15px;
}

.side-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.side-panel {
  flex: 1;
  min-width: 0;
}

/* 窄屏：侧边栏改为顶部横向滚动，面板在下方 */
@media (max-width: 768px) {
  .side-layout {
    flex-direction: column;
  }
  .side-nav {
    flex: none;
    width: 100%;
    flex-direction: row;
    overflow-x: auto;
    scrollbar-width: none;
    position: static;
  }
  .side-nav::-webkit-scrollbar {
    display: none;
  }
  .side-item {
    width: auto;
    flex: none;
  }
}
</style>
