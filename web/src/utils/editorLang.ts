/**
 * CodeMirror 6 语言支持（按需加载）。
 *
 * 语言包体量差异很大（lang-javascript / lang-php / lang-rust 各自几百 KB），
 * 静态全量 import 会把它们统统塞进 CodeEditor 那个 chunk，不管用户有没有用到。
 * 这里改成**按语言名动态 import**：
 * - 每个语言一个 `() => import(...)`，由打包器切成独立 chunk，用到才下载；
 * - 加载结果进 `cache`（同一个语言第二次用直接命中），并发请求合并成一次（inflight）；
 * - 调用方（CodeEditor）通过 Compartment 热替换语言扩展，不重建 EditorView，
 *   光标位置 / 滚动位置 / undo 历史都保留。
 *
 * 语言来源：
 * - shell / nginx / ini / yaml / python / css / xml 取自 @codemirror/legacy-modes 的单个模式文件（很小）
 * - js / ts / json / markdown / html / php / rust / go 使用官方语言包
 */
import type { Extension } from '@codemirror/state'
import { StreamLanguage } from '@codemirror/language'

export type EditorLangName =
  | 'text'
  | 'shell'
  | 'javascript'
  | 'json'
  | 'markdown'
  | 'html'
  | 'css'
  | 'xml'
  | 'yaml'
  | 'ini'
  | 'nginx'
  | 'python'
  | 'php'
  | 'rust'
  | 'go'

/** 语言名 → 动态加载器（加载完成后返回可直接塞进编辑器的 Extension） */
const LOADERS: Record<EditorLangName, () => Promise<Extension>> = {
  text: async () => [],
  shell: async () =>
    StreamLanguage.define((await import('@codemirror/legacy-modes/mode/shell')).shell),
  javascript: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true, jsx: true }),
  json: async () => (await import('@codemirror/lang-json')).json(),
  markdown: async () => (await import('@codemirror/lang-markdown')).markdown(),
  // 官方包支持标签闭合补全,并对 <style>/<script> 内嵌 CSS/JS 继续高亮
  html: async () => (await import('@codemirror/lang-html')).html(),
  css: async () => StreamLanguage.define((await import('@codemirror/legacy-modes/mode/css')).css),
  xml: async () => StreamLanguage.define((await import('@codemirror/legacy-modes/mode/xml')).xml),
  yaml: async () =>
    StreamLanguage.define((await import('@codemirror/legacy-modes/mode/yaml')).yaml),
  ini: async () =>
    StreamLanguage.define((await import('@codemirror/legacy-modes/mode/properties')).properties),
  nginx: async () =>
    StreamLanguage.define((await import('@codemirror/legacy-modes/mode/nginx')).nginx),
  python: async () =>
    StreamLanguage.define((await import('@codemirror/legacy-modes/mode/python')).python),
  php: async () => (await import('@codemirror/lang-php')).php(),
  rust: async () => (await import('@codemirror/lang-rust')).rust(),
  go: async () => (await import('@codemirror/lang-go')).go(),
}

/** 已加载的语言扩展：切走再切回来不用重新下载 */
const cache = new Map<EditorLangName, Extension>()
/** 正在加载中的请求：并发切换同一语言时合并为一次 import */
const inflight = new Map<EditorLangName, Promise<Extension>>()

/** 按语言名异步构建 CodeMirror Extension（带缓存） */
export function loadLangExtension(name: EditorLangName): Promise<Extension> {
  const hit = cache.get(name)
  if (hit) return Promise.resolve(hit)
  const pending = inflight.get(name)
  if (pending) return pending

  const task = LOADERS[name]()
    .then((ext) => {
      cache.set(name, ext)
      inflight.delete(name)
      return ext
    })
    .catch((err) => {
      // 加载失败不该把编辑器拖死：退化为纯文本，并让下一次还能重试
      inflight.delete(name)
      throw err
    })
  inflight.set(name, task)
  return task
}

/** 语言选择器用：值是 EditorLangName，'auto' 表示「按扩展名自动判断」 */
export interface LangOption {
  value: EditorLangName | 'auto'
  label: string
}

/** 语言下拉的可选项（不含 auto，auto 由调用方按当前文件动态拼） */
export const LANG_OPTIONS: LangOption[] = [
  { value: 'text', label: 'Plain Text' },
  { value: 'shell', label: 'Shell' },
  { value: 'javascript', label: 'JavaScript / TypeScript' },
  { value: 'json', label: 'JSON' },
  { value: 'markdown', label: 'Markdown' },
  { value: 'html', label: 'HTML' },
  { value: 'css', label: 'CSS / SCSS' },
  { value: 'xml', label: 'XML' },
  { value: 'yaml', label: 'YAML' },
  { value: 'ini', label: 'INI / Conf' },
  { value: 'nginx', label: 'Nginx' },
  { value: 'python', label: 'Python' },
  { value: 'php', label: 'PHP' },
  { value: 'rust', label: 'Rust' },
  { value: 'go', label: 'Go' },
]

const LABELS = new Map(LANG_OPTIONS.map((o) => [o.value, o.label]))

/** 语言名 → 展示名（状态栏 / 下拉里直接用） */
export function langLabel(name: EditorLangName | 'auto'): string {
  return LABELS.get(name) || String(name)
}

function baseName(path: string): string {
  return (
    String(path || '')
      .split(/[\\/]/)
      .pop() || ''
  )
}

/**
 * 按文件路径(或文件名)推断语法:
 * - 名字含 nginx 的 conf 视为 nginx(如 nginx.conf)
 * - 其余 .conf/.ini/.cnf 等 key-value 配置走 ini(properties 模式,php.ini / my.cnf / postgresql.conf 通用)
 * - html/htm/xhtml/vue 走 html(含内嵌 CSS/JS 高亮),css/scss/less 走 css,xml/svg 走 xml
 */
export function langFromPath(path: string): EditorLangName {
  const name = baseName(path).toLowerCase()
  if (!name) return 'text'
  const dot = name.lastIndexOf('.')
  const ext = dot > 0 ? name.slice(dot + 1) : ''
  if (name.includes('nginx')) return 'nginx'
  switch (ext) {
    case 'sh':
    case 'bash':
    case 'zsh':
    case 'ksh':
      return 'shell'
    case 'js':
    case 'jsx':
    case 'mjs':
    case 'cjs':
    case 'ts':
    case 'tsx':
    case 'mts':
    case 'cts':
      return 'javascript'
    case 'json':
      return 'json'
    case 'md':
    case 'markdown':
      return 'markdown'
    case 'html':
    case 'htm':
    case 'xhtml':
    case 'shtml':
    case 'vue':
      return 'html'
    case 'css':
    case 'scss':
    case 'less':
      return 'css'
    case 'xml':
    case 'svg':
    case 'xsl':
    case 'xslt':
    case 'rss':
      return 'xml'
    case 'yaml':
    case 'yml':
      return 'yaml'
    case 'ini':
    case 'properties':
    case 'conf':
    case 'cfg':
    case 'cnf':
      return 'ini'
    case 'py':
      return 'python'
    case 'php':
      return 'php'
    case 'rs':
      return 'rust'
    case 'go':
      return 'go'
    default:
      return 'text'
  }
}
