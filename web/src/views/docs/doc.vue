<script setup lang="ts">
/**
 * 单篇文档渲染页。
 *
 * 四份 md 共用本组件，根据路由末段
 * (`/docs/changelog` · `/docs/manual` · `/docs/faq` · `/docs/upgrade`)
 * 决定拉哪一份 md。入口在「系统设置 → About ZAP」(`/system/about`)，
 * 本路由在 constantRoutes 里以 hidden 常驻，只保证链接可达。
 *
 * 后端已经把 md 渲染为 HTML 片段，这里直接 `v-html` 展示。
 * 样式靠 `:deep(.docs-md ...)` 限制作用域，只对 Markdown 内容生效，
 * 避免污染全局。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ArrowBack } from '@/icons'
import { getDocHtml, getDocsList, type DocMeta } from '@/api/docs'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()

const html = ref<string>('')
const loading = ref(true)
const error = ref<string>('')

// 路径末段 → 后端白名单 id（例如 `/docs/changelog` → `changelog`）
const docId = computed<string>(() => {
  const seg = route.path.split('/').filter(Boolean).pop() || ''
  return seg
})

// 当前 locale 下的展示标题（来自 docs.<id> i18n key）
const title = computed<string>(() => {
  const map: Record<string, string> = {
    changelog: 'docs.changelog',
    manual: 'docs.manual',
    faq: 'docs.faq',
    upgrade: 'docs.upgrade',
  }
  return t(map[docId.value] || 'docs.changelog')
})

// 后端目录里对应的兜底标题，仅当 i18n 没匹配时显示（一般用不到）
const fallbackTitle = ref<string>('')

async function load() {
  loading.value = true
  error.value = ''
  html.value = ''
  try {
    // locale 传给后端 → 多语言 md 回退（`_zh-CN` → `_zh` → 兜底）
    const resp = await getDocHtml(docId.value, locale.value)
    html.value = resp.data || ''
    if (!html.value) {
      error.value = t('docs.notFound')
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    error.value = t('docs.fetchFailed', { msg })
    html.value = ''
  } finally {
    loading.value = false
  }
}

// 切换 locale 时兜底标题需要重算（fallback 不参与 i18n，但 fallback ref 的中文/英文不变）
// 这里只在初次挂载拿一次目录，省一次接口
onMounted(async () => {
  try {
    const r = await getDocsList()
    const meta = (r.data || []).find((m: DocMeta) => m.id === docId.value)
    if (meta) fallbackTitle.value = locale.value === 'en-US' ? meta.title_en : meta.title_zh
  } catch {
    /* 目录失败不影响渲染 */
  }
  await load()
})

// 路由切换（从 changelog 切到 manual）或语言切换（zh-CN ↔ en-US）都重拉
watch([docId, () => locale.value], () => load())

function back() {
  // 文档入口已整合进「系统设置 → About ZAP」
  router.push('/system/about')
}
</script>

<template>
  <div class="docs-doc">
    <div class="docs-doc__header">
      <el-button text @click="back" class="docs-doc__back">
        <el-icon style="margin-right: 4px"><ArrowBack /></el-icon>
        {{ t('docs.back') }}
      </el-button>
      <h2 class="docs-doc__title">{{ title || fallbackTitle }}</h2>
    </div>

    <el-card v-loading="loading" shadow="never" class="docs-doc__card">
      <!--
        后端返回的 HTML 是从 Markdown 渲染出来的，由后端 pulldown-cmark 控制。
        前端不再二次处理，但通过 .docs-md 类给元素加 scoped 样式。
      -->
      <article v-if="html" class="docs-md" v-html="html"></article>
      <el-empty v-else-if="!loading" :description="error || t('docs.notFound')" />
    </el-card>
  </div>
</template>

<style scoped>
.docs-doc {
  padding: 16px;
  max-width: 1080px;
  margin: 0 auto;
}
.docs-doc__header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
}
.docs-doc__back {
  padding: 4px 8px;
}
.docs-doc__title {
  margin: 0;
  font-size: 22px;
  font-weight: 600;
}
.docs-doc__card {
  border-radius: 8px;
}

/* Markdown 内容样式 —— :deep() 让 scoped 样式穿透到 v-html 子节点 */
.docs-md {
  line-height: 1.75;
  color: var(--el-text-color-primary);
  font-size: 15px;
}
.docs-md :deep(h1),
.docs-md :deep(h2),
.docs-md :deep(h3),
.docs-md :deep(h4) {
  margin: 1.6em 0 0.8em;
  font-weight: 600;
  line-height: 1.3;
}
.docs-md :deep(h1) {
  font-size: 1.8em;
  border-bottom: 1px solid var(--el-border-color-lighter);
  padding-bottom: 0.3em;
}
.docs-md :deep(h2) {
  font-size: 1.5em;
}
.docs-md :deep(h3) {
  font-size: 1.25em;
}
.docs-md :deep(p) {
  margin: 0.8em 0;
}
.docs-md :deep(ul),
.docs-md :deep(ol) {
  padding-left: 1.6em;
  margin: 0.6em 0;
}
.docs-md :deep(li) {
  margin: 0.2em 0;
}
.docs-md :deep(code) {
  background: var(--el-fill-color-light);
  padding: 0.15em 0.4em;
  border-radius: 4px;
  font-size: 0.9em;
  font-family: 'JetBrains Mono', 'Fira Code', Consolas, monospace;
}
.docs-md :deep(pre) {
  background: var(--el-fill-color-light);
  padding: 12px 14px;
  border-radius: 6px;
  overflow-x: auto;
  line-height: 1.5;
}
.docs-md :deep(pre code) {
  background: transparent;
  padding: 0;
}
.docs-md :deep(blockquote) {
  border-left: 4px solid var(--el-color-primary-light-5);
  margin: 1em 0;
  padding: 0.4em 1em;
  color: var(--el-text-color-regular);
  background: var(--el-fill-color-blank);
}
.docs-md :deep(table) {
  border-collapse: collapse;
  margin: 1em 0;
  width: auto;
}
.docs-md :deep(th),
.docs-md :deep(td) {
  border: 1px solid var(--el-border-color-lighter);
  padding: 6px 12px;
}
.docs-md :deep(th) {
  background: var(--el-fill-color-light);
}
.docs-md :deep(a) {
  color: var(--el-color-primary);
  text-decoration: none;
}
.docs-md :deep(a:hover) {
  text-decoration: underline;
}
.docs-md :deep(hr) {
  border: 0;
  border-top: 1px solid var(--el-border-color-lighter);
  margin: 1.6em 0;
}
.docs-md :deep(img) {
  max-width: 100%;
  border-radius: 4px;
}
</style>