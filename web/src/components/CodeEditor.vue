<template>
  <div ref="host" class="code-editor" :class="{ 'is-readonly': readonly }"></div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Compartment, EditorState, type Extension } from '@codemirror/state'
import { EditorView, placeholder } from '@codemirror/view'
import { basicSetup } from 'codemirror'
import { loadLangExtension, langFromPath, type EditorLangName } from '@/utils/editorLang'

const props = withDefaults(
  defineProps<{
    modelValue: string
    lang?: EditorLangName
    path?: string
    readonly?: boolean
    placeholder?: string
    autofocus?: boolean
    /**
     * 父级用 v-show 切标签时传：变为可见会重新测量一次。
     * CodeMirror 在 display:none 期间量不到尺寸，不测量的话尺寸/行号可能不对。
     */
    active?: boolean
  }>(),
  {
    lang: undefined,
    path: '',
    readonly: false,
    placeholder: undefined,
    autofocus: false,
    active: true,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  /** 光标位置（1 起算），供调用方的状态栏显示「行 / 列」 */
  (e: 'cursor', pos: { line: number; col: number }): void
}>()

const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null

/**
 * 语言与只读状态各用一个 Compartment：切换时 dispatch reconfigure 即可，
 * 不用重建 EditorView（重建会丢光标、丢滚动位置、丢 undo 历史）。
 */
const langConf = new Compartment()
const readOnlyConf = new Compartment()

function effectiveLang(): EditorLangName {
  if (props.lang) return props.lang
  if (props.path) return langFromPath(props.path)
  return 'text'
}

/** 基础外观:等宽字体、行高、gutter 配色,与 Element Plus 默认风格协调 */
const baseTheme = EditorView.theme({
  '&': { height: '100%', fontSize: '13px' },
  '.cm-scroller': {
    fontFamily: "'JetBrains Mono', Menlo, Consolas, 'Courier New', monospace",
    lineHeight: '1.6',
    overflow: 'auto',
  },
  '.cm-content': {
    padding: '8px 0',
    caretColor: 'var(--el-color-primary, #409eff)',
    // 用 EP 变量，浅色 / 深色主题自动适配
    color: 'var(--el-text-color-primary)',
    backgroundColor: 'transparent',
  },
  '.cm-gutters': {
    backgroundColor: 'var(--el-fill-color-light)',
    color: 'var(--el-text-color-placeholder)',
    borderRight: '1px solid var(--el-border-color-lighter)',
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-activeLine': { backgroundColor: 'rgba(64, 158, 255, 0.05)' },
  '.cm-activeLineGutter': { backgroundColor: 'rgba(64, 158, 255, 0.08)' },
  '.cm-tooltip': { zIndex: 3100 },
})

/** 光标所在行列（1 起算） */
function cursorOf(state: EditorState) {
  const pos = state.selection.main.head
  const line = state.doc.lineAt(pos)
  return { line: line.number, col: pos - line.from + 1 }
}

/** 语言切换的序号：异步加载回来时据此丢弃过期结果（期间又切了一次） */
let langSeq = 0

/**
 * 异步加载语言包并热替换。视图先以「无语言」创建，语言 chunk 到位后
 * reconfigure 上去，高亮才生效——换来的好处是 CodeEditor 主 chunk 不再
 * 塞进所有语言包。
 */
async function applyLang(name: EditorLangName) {
  const seq = ++langSeq
  let ext: Extension
  try {
    ext = await loadLangExtension(name)
  } catch {
    return
  }
  if (!view || seq !== langSeq) return
  view.dispatch({ effects: langConf.reconfigure(ext) })
}

function createView() {
  if (!host.value || view) return
  const extensions: Extension[] = [
    basicSetup,
    baseTheme,
    langConf.of([]),
    readOnlyConf.of(EditorState.readOnly.of(props.readonly)),
    EditorView.lineWrapping,
    EditorView.updateListener.of((u) => {
      if (u.docChanged) emit('update:modelValue', u.state.doc.toString())
      if (u.docChanged || u.selectionSet) emit('cursor', cursorOf(u.state))
    }),
  ]
  if (props.placeholder) extensions.push(placeholder(props.placeholder))
  const state = EditorState.create({ doc: props.modelValue ?? '', extensions })
  view = new EditorView({ state, parent: host.value })
  emit('cursor', cursorOf(state))
  if (props.autofocus) view.focus()
  void applyLang(effectiveLang())
}

function destroyView() {
  view?.destroy()
  view = null
}

onMounted(createView)
onBeforeUnmount(destroyView)

// 外部内容变化(切文件/加载完成后回填)时同步文档,来自编辑器自身的修改不重复触发
watch(
  () => props.modelValue,
  (val) => {
    if (!view) return
    const doc = view.state.doc.toString()
    const next = val ?? ''
    if (doc === next) return
    view.dispatch({ changes: { from: 0, to: doc.length, insert: next } })
  },
)

// 语言变化：热替换（不重建视图，光标与滚动位置都不动）
watch(
  () => effectiveLang(),
  (name) => {
    void applyLang(name)
  },
)

// 只读变化：同样只 reconfigure
watch(
  () => props.readonly,
  (val) => {
    view?.dispatch({ effects: readOnlyConf.reconfigure(EditorState.readOnly.of(val)) })
  },
)

// 多标签场景：从隐藏切回可见时重新测量，避免沿用 display:none 时的旧尺寸
watch(
  () => props.active,
  (val) => {
    if (val) view?.requestMeasure()
  },
)
</script>

<style scoped>
.code-editor {
  position: relative;
  height: 100%;
  min-height: 120px;
  background: var(--el-bg-color);
}

.code-editor :deep(.cm-editor) {
  height: 100%;
}

.code-editor.is-readonly :deep(.cm-editor) {
  background: var(--el-fill-color-light);
}

.code-editor.is-readonly :deep(.cm-content) {
  cursor: default;
}
</style>
