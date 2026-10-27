// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 终端剪贴板快捷键（所有 xterm 终端统一接入）：
 * - Ctrl+C / Cmd+C：有选中 → 复制到剪贴板并清空选中；无选中 → 照常发送 ^C（SIGINT）
 * - 粘贴（Ctrl+V / 右键）不做任何接管：xterm 内核原生处理，接管反而会粘两次
 *
 * 注意：Ctrl+Shift+C 是浏览器保留快捷键（打开 DevTools），网页无法拦截，
 * 因此不依赖它 —— 统一由 Ctrl+C 智能区分「复制 / 中断」。
 */
import type { Terminal } from '@xterm/xterm'

/** 非安全上下文（http 部署）下 navigator.clipboard 不存在，退回 execCommand */
function fallbackCopy(text: string): void {
  const ta = document.createElement('textarea')
  ta.value = text
  ta.style.position = 'fixed'
  ta.style.opacity = '0'
  document.body.appendChild(ta)
  ta.select()
  try {
    document.execCommand('copy')
  } catch {
    // 旧浏览器不支持，忽略
  }
  ta.remove()
}

function copyText(text: string): void {
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).catch(() => fallbackCopy(text))
    return
  }
  fallbackCopy(text)
}

export function attachClipboardShortcuts(term: Terminal): void {
  term.attachCustomKeyEventHandler((ev: KeyboardEvent): boolean => {
    if (ev.type !== 'keydown') return true
    const key = ev.key.toLowerCase()
    const mod = ev.ctrlKey || ev.metaKey
    if (!mod || ev.altKey) return true

    if (key === 'c') {
      const selected = term.getSelection()
      if (selected) {
        copyText(selected)
        term.clearSelection()
        return false // 有选中时不把 ^C 发给 shell
      }
      // 无选中：裸 Ctrl+C 照常发送 ^C；Ctrl+Shift+C 交给浏览器（保留键）
      return !ev.shiftKey
    }

    // 其余按键（含 Ctrl+V 粘贴）一律交回 xterm 内核 / 浏览器原生处理
    return true
  })
}
