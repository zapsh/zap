<template>
  <div class="nav-tabs">
    <div ref="scrollerRef" class="nav-tabs__scroll" @wheel="onWheel">
      <button
        v-for="tab in rendered"
        :key="tab.path"
        type="button"
        class="nav-tab"
        :class="{ 'is-active': tab.path === route.path, 'is-pinned': !tab.closable }"
        :title="tab.tip"
        @click="go(tab)"
        @contextmenu.prevent="openMenu(tab, $event)"
      >
        <el-icon class="nav-tab__icon"><Icon :icon="tab.icon" /></el-icon>

        <!-- 常驻标签（仪表盘）只留图标，省位置 -->
        <template v-if="!tab.iconOnly">
          <span v-if="tab.parentLabel" class="nav-tab__parent">{{ tab.parentLabel }}</span>
          <span class="nav-tab__label">{{ tab.label }}</span>
        </template>

        <el-icon v-if="tab.closable" class="nav-tab__close" @click.stop="close(tab)">
          <Close />
        </el-icon>
      </button>
    </div>

    <!-- 右键菜单（fixed 定位：标签栏是横向滚动容器，absolute 会被裁掉） -->
    <ul v-show="menu.visible" class="nav-tabs__menu" :style="{ left: `${menu.left}px`, top: `${menu.top}px` }">
      <li @click="refresh(menu.tab)">{{ t('layout.tagsRefresh') }}</li>
      <li v-if="menu.tab?.closable" @click="close(menu.tab)">
        {{ t('layout.tagsCloseCurrent') }}
      </li>
      <li @click="closeOthers(menu.tab)">{{ t('layout.tagsCloseOthers') }}</li>
      <li @click="closeAll">{{ t('layout.tagsCloseAll') }}</li>
    </ul>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { Close, Icon } from '@/icons'
import { translateTitle } from '@/i18n'
import { DASHBOARD_PATH, useTagsStore, type NavTab } from '@/stores/tags'

const route = useRoute()
const router = useRouter()
const { t, locale } = useI18n()
const tagsStore = useTagsStore()

const scrollerRef = ref<HTMLElement>()

/** 渲染用：把标题原文翻成当前语言，并算好 tooltip */
interface RenderedTab extends NavTab {
  label: string
  parentLabel: string
  /** 常驻标签只显示图标 */
  iconOnly: boolean
  tip: string
}

const rendered = computed<RenderedTab[]>(() => {
  // 触碰 locale：translateTitle 走的是全局 composer，不参与响应式追踪，
  // 不显式依赖一下，切语言时标签标题不会跟着变。
  void locale.value
  return tagsStore.tabs.map((tab) => {
    const label = translateTitle(tab.title)
    const parentLabel = tab.parentTitle ? translateTitle(tab.parentTitle) : ''
    return {
      ...tab,
      label,
      parentLabel,
      iconOnly: !tab.closable,
      tip: parentLabel ? `${parentLabel} › ${label}` : label,
    }
  })
})

// ── 右键菜单 ────────────────────────────────────────────────
const menu = reactive({ visible: false, left: 0, top: 0, tab: undefined as RenderedTab | undefined })

function openMenu(tab: RenderedTab, e: MouseEvent) {
  const width = 140
  menu.left = Math.min(e.clientX, window.innerWidth - width)
  menu.top = e.clientY + 4
  menu.tab = tab
  menu.visible = true
}

function closeMenu() {
  menu.visible = false
}

onMounted(() => document.addEventListener('click', closeMenu))
onBeforeUnmount(() => document.removeEventListener('click', closeMenu))

// ── 操作 ────────────────────────────────────────────────────
function go(tab: RenderedTab) {
  if (tab.path !== route.path) router.push(tab.path)
}

function close(tab?: RenderedTab) {
  if (!tab) return
  const next = tagsStore.close(tab.path, route.path)
  if (next && next !== route.path) router.push(next)
}

function closeOthers(tab?: RenderedTab) {
  if (!tab) return
  tagsStore.closeOthers(tab.path)
  if (tab.path !== route.path) router.push(tab.path)
}

function closeAll() {
  router.push(tagsStore.closeAll())
}

/** 标签溢出时让滚轮也能横向滚（竖向滚轮默认对横向容器无效，很别扭） */
function onWheel(e: WheelEvent) {
  const el = scrollerRef.value
  if (!el || el.scrollWidth <= el.clientWidth) return
  const delta = Math.abs(e.deltaY) > Math.abs(e.deltaX) ? e.deltaY : e.deltaX
  if (!delta) return
  el.scrollLeft += delta
  e.preventDefault()
}

async function refresh(tab?: RenderedTab) {
  if (!tab) return
  // 常驻存活的页面（文件管理 / 终端）躺在 keep-alive 缓存里，直接跳中转页再回来还是
  // 老实例，刷新点了等于没点：先把缓存摘掉，等老实例真正卸载后再放回白名单。
  // 顺序不能乱（见 stores/tags.ts 的 suspendCache）：摘早了白摘，放早了老实例会被重新缓存。
  const resume = tagsStore.suspendCache(tab.path)
  await nextTick()
  // 走 /redirect 中转：由它 replace 回原地址，从而重建页面组件
  await router.replace(`/redirect${tab.path}`)
  await nextTick()
  resume()
}

/** 激活标签滚进视野（标签多到溢出时，切换靠键盘 / 侧栏也不会迷失位置） */
watch(
  () => route.path,
  async () => {
    await nextTick()
    scrollerRef.value?.querySelector('.is-active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  },
)

/** 常驻标签兜底：所有标签关光后回仪表盘 */
watch(
  () => tagsStore.tabs.length,
  (len) => {
    if (len === 0) router.push(DASHBOARD_PATH)
  },
)
</script>

<style lang="scss" scoped>
.nav-tabs {
  flex: 1;
  min-width: 0;
  position: relative;
  display: flex;
  align-items: center;
}

.nav-tabs__scroll {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 8px;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;

  &::-webkit-scrollbar {
    display: none;
  }
}

.nav-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  height: 32px;
  padding: 0 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: #c8d4e2;
  font-size: 13px;
  line-height: 1;
  white-space: nowrap;
  cursor: pointer;
  transition:
    background 0.2s,
    color 0.2s;

  &:hover {
    background: rgba(255, 255, 255, 0.08);
    color: #fff;
  }

  &.is-active {
    background: rgba(255, 255, 255, 0.16);
    color: #fff;
  }

  &.is-pinned {
    padding: 0 8px;
  }
}

.nav-tab__icon {
  font-size: 16px;
  flex-shrink: 0;
}

/* 二级标签显示「一级 › 二级」：父级弱化，当前页突出 */
.nav-tab__parent {
  color: rgba(255, 255, 255, 0.55);
  flex-shrink: 0;

  &::after {
    content: '›';
    margin: 0 6px;
    font-size: 12px;
  }
}

.nav-tab.is-active .nav-tab__parent {
  color: rgba(255, 255, 255, 0.7);
}

.nav-tab__label {
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.nav-tab__close {
  font-size: 14px;
  padding: 2px;
  border-radius: 50%;
  color: rgba(255, 255, 255, 0.5);
  transition:
    background 0.2s,
    color 0.2s;

  &:hover {
    background: rgba(255, 255, 255, 0.24);
    color: #fff;
  }
}

.nav-tabs__menu {
  position: fixed;
  z-index: 3000;
  margin: 0;
  padding: 4px 0;
  list-style: none;
  min-width: 120px;
  border-radius: 6px;
  background: var(--el-bg-color-overlay);
  box-shadow: var(--el-box-shadow);
  font-size: 13px;
  color: var(--el-text-color-primary);

  li {
    padding: 7px 14px;
    cursor: pointer;

    &:hover {
      background: var(--el-fill-color-light);
    }
  }
}
</style>
