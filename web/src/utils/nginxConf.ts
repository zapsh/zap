// SPDX-License-Identifier: AGPL-3.0-only
/**
 * Nginx 主配置（nginx.conf）的可视化读写工具。
 *
 * 设计原则：
 * - 不整体重写用户配置，只在「对应块内」做最小行级修改；
 * - 面板写入的字段统一收敛到一个带标记的托管区
 *   （`# zap-visual-begin` … `# zap-visual-end`），重复保存先删旧区再写新区；
 * - worker_processes 在顶层处理，其余字段只写进 `http { }` 块；
 * - http 内已显式定义过的同名字段行会被移除并归入托管区，避免重复定义歧义
 *   （conf.d 子文件若重复定义同名指令，nginx 后加载者覆盖，属预期行为）。
 */

/** 可视化字段（全部为 http 上下文直接子指令） */
export const HTTP_VISUAL_KEYS = [
  'server_tokens',
  'client_max_body_size',
  'sendfile',
  'tcp_nopush',
  'keepalive_timeout',
  'gzip',
  'gzip_min_length',
  'gzip_comp_level',
  'gzip_types',
] as const

/** worker_processes 为顶层指令，单独处理 */
export const TOP_LEVEL_KEY = 'worker_processes'

export type HttpVisualKey = (typeof HTTP_VISUAL_KEYS)[number]

export type VisualValues = Record<string, string | null>

const VISUAL_BEGIN = '# zap-visual-begin'
const VISUAL_END = '# zap-visual-end'

interface ParsedLine {
  /** 原始行号 */
  i: number
  text: string
  /** 所在块栈顶（null = 顶层） */
  top: string | null
  /** 是否为「指令行」（以分号结尾且本行无块括号） */
  isDirective: boolean
  /** 是否为纯闭合行（只含 }） */
  isCloseOnly: boolean
  /** 顶层指令行（块栈为空） */
  isTop: boolean
  /** http 块内直接子指令 */
  isHttpChild: boolean
}

/** 轻量行级解析：跟踪块栈，识别指令所属层级。 */
export function classifyLines(text: string): ParsedLine[] {
  const raw = text.split('\n')
  const stack: string[] = []
  const out: ParsedLine[] = []
  for (let i = 0; i < raw.length; i++) {
    const t = raw[i]
    const trim = t.trim()
    const top = stack.length ? stack[stack.length - 1] : null
    if (trim === '' || trim.startsWith('#')) {
      out.push({ i, text: t, top, isDirective: false, isCloseOnly: false, isTop: !stack.length, isHttpChild: false })
      continue
    }
    let openCount = 0
    let closeCount = 0
    for (const ch of trim) {
      if (ch === '{') openCount++
      else if (ch === '}') closeCount++
    }
    const isCloseOnly = closeCount > 0 && openCount === 0
    const isOpen = openCount > 0
    const isDirective = !isOpen && !isCloseOnly && trim.endsWith(';')
    out.push({
      i,
      text: t,
      top,
      isDirective,
      isCloseOnly,
      isTop: !stack.length && isDirective,
      isHttpChild: top === 'http' && isDirective,
    })
    // 更新块栈：先弹后压（同行开合罕见，仅做计数）
    if (closeCount > 0) {
      for (let k = 0; k < closeCount; k++) stack.pop()
    }
    if (openCount > 0) {
      const name = (trim.split(/\s+/)[0] || 'block').replace(/\{/g, '')
      for (let k = 0; k < openCount; k++) stack.push(name)
    }
  }
  return out
}

function directiveKey(trim: string): string {
  return trim.split(/\s+/)[0] ?? ''
}

function directiveValue(trim: string): string {
  const s = trim.slice(trim.indexOf(' '))
  return s.trim().replace(/;\s*$/, '')
}

/** 读取可视化字段当前生效值（取最后一个匹配，接近 nginx 后者覆盖的语义）。 */
export function parseVisualValues(text: string): VisualValues {
  const values: VisualValues = {}
  for (const r of classifyLines(text)) {
    if (!r.isDirective) continue
    const trim = r.text.trim()
    const key = directiveKey(trim)
    const val = directiveValue(trim)
    if (r.isHttpChild && (HTTP_VISUAL_KEYS as readonly string[]).includes(key)) {
      values[key] = val
    } else if (r.isTop && key === TOP_LEVEL_KEY) {
      values[TOP_LEVEL_KEY] = val
    }
  }
  return values
}

/** 删除旧的托管区（begin..end 连续行）。 */
function stripVisualRange(text: string): string {
  const lines = text.split('\n')
  let begin = -1
  let end = -1
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim()
    if (t.startsWith(VISUAL_BEGIN)) begin = i
    else if (t.startsWith(VISUAL_END)) {
      if (begin >= 0 && end < 0) end = i
    }
  }
  if (begin < 0 || end < begin) return text
  lines.splice(begin, end - begin + 1)
  return lines.join('\n')
}

/** 找到 http 块的结束行号（http { 对应的第一个 } 行）。 */
function findHttpClose(lines: string[]): number | null {
  const cls = classifyLines(lines.join('\n'))
  for (const r of cls) {
    if (r.isCloseOnly && r.top === 'http') return r.i
  }
  return null
}

/** 拼接托管区内容（不含缩进）。 */
function buildVisualBlock(values: VisualValues): string[] {
  const out: string[] = [VISUAL_BEGIN + ' 面板可视化设置（Nginx 配置页保存，请勿手工修改本区）']
  const order: string[] = [...HTTP_VISUAL_KEYS]
  for (const key of order) {
    const v = values[key]
    if (v === null || v === undefined) continue
    const val = String(v).trim()
    if (!val) continue
    if (key === 'gzip_types' && val.startsWith('#')) continue
    out.push(`${key} ${val};`)
  }
  out.push(VISUAL_END)
  return out
}

/**
 * 把可视化字段应用到主配置文本。
 * - 返回 null 表示文本无 http 块（无法安全写入）。
 */
export function applyVisualValues(text: string, values: VisualValues): string | null {
  // 1. 移除旧托管区
  let t = stripVisualRange(text)
  let lines = t.split('\n')

  // 2. 收集待删除行：http 直接子层的同名指令 + 顶层 worker_processes
  const drop = new Set<number>()
  for (const r of classifyLines(t)) {
    if (!r.isDirective) continue
    const trim = r.text.trim()
    const key = directiveKey(trim)
    if (r.isHttpChild && (HTTP_VISUAL_KEYS as readonly string[]).includes(key)) drop.add(r.i)
    else if (r.isTop && key === TOP_LEVEL_KEY) drop.add(r.i)
  }
  if (drop.size) {
    lines = lines.filter((_, idx) => !drop.has(idx))
  }
  const after = lines.join('\n')

  // 3. http 块托管区写入（放在 http 块末尾、include 之后，保证生效顺序最后）
  const httpClose = findHttpClose(lines)
  if (httpClose === null) return null
  const block = buildVisualBlock(values)
  const indented = block.map((l) => (l.startsWith('#') ? l : `    ${l}`))
  lines.splice(httpClose, 0, ...indented)
  let out = lines.join('\n')

  // 4. worker_processes（顶层）：插入到第一个非空行前
  const wp = values[TOP_LEVEL_KEY]
  if (wp !== null && wp !== undefined && String(wp).trim()) {
    const seg = out.split('\n')
    let at = 0
    while (at < seg.length && seg[at].trim() === '') at++
    seg.splice(at, 0, `${TOP_LEVEL_KEY} ${String(wp).trim()};`)
    out = seg.join('\n')
  }

  // 保留文件原有结尾换行习惯
  if (text.endsWith('\n') && !out.endsWith('\n')) out += '\n'
  return out
}

/** 主配置内容里是否仍包含面板站点 include（保存保护用）。 */
export function hasPanelInclude(content: string): boolean {
  return content.includes('sites-enabled')
}
