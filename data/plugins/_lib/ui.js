/*
 * 插件界面运行时（UIKit）—— 由宿主在返回插件 ui.html 时自动注入，与 ui.css 配套。
 *
 * 前提：宿主的 RPC 桥已经在 window.zap 上放好了 call / pickDir / pickFile
 * （见前端 PluginSlot.vue 的 RPC_BRIDGE，注入顺序是先桥后本文件），这里只做扩展，
 * 不覆盖已有成员。
 *
 * 两类能力：
 *   1. 需要宿主配合的（通知条 / 确认框 / 输入框）→ postMessage 让宿主用 Element Plus 渲染，
 *      观感与面板一致；老宿主不认这套协议时有本地兜底（内置吐司 / window.confirm）。
 *   2. 纯前端渲染（表格 / 页签 / diff 着色 / 忙碌态）→ 直接在 iframe 内写完。
 *
 * 协议：iframe → 宿主 { __zapUi: 1, id, kind, ... }；宿主回 { __zapUi: 1, id, ok, data }。
 */
;(function () {
  var zap = (window.zap = window.zap || {})
  if (zap.ui) return // 重复注入（比如宿主也塞了一份）时以先注入的为准

  var seq = 0
  var pending = {}
  // 探测宿主是否接管 UI 时对多久没应答算「不支持」：这只用于 ping，
  // 真正的确认框要等人点，不能套这个超时
  var PROBE_TIMEOUT = 1200
  // 交互类请求的最长等待：宿主可能把弹窗开着很久，给个宽松上限防止永久挂起
  var ASK_TIMEOUT = 600000

  window.addEventListener('message', function (ev) {
    var d = ev.data
    if (!d || d.__zapUi !== 1 || !d.id) return
    var p = pending[d.id]
    if (!p) return
    delete pending[d.id]
    window.clearTimeout(p.timer)
    if (d.ok) p.resolve(d.data)
    else p.reject(new Error(d.data || '已取消'))
  })

  /**
   * 先探一次宿主是否会接管 UI（ping）。
   *
   * 弹确认框是「等人点」的动作，不能有短超时 —— 否则用户还没点，
   * 这边就以为宿主不支持、自己又弹一个 window.confirm。所以能力只探测一次：
   * 探到就用宿主，探不到就一直走本地兜底。
   */
  var probe = null
  function ensureHost() {
    if (probe) return probe
    probe = new Promise(function (resolve) {
      var id = 'u' + ++seq
      var settled = false
      var finish = function (v) {
        if (settled) return
        settled = true
        window.clearTimeout(timer)
        delete pending[id]
        resolve(v)
      }
      var timer = window.setTimeout(function () { finish(false) }, PROBE_TIMEOUT)
      pending[id] = {
        resolve: function () { finish(true) },
        reject: function () { finish(false) },
        timer: timer,
      }
      try {
        parent.postMessage({ __zapUi: 1, id: id, kind: 'ping' }, '*')
      } catch (e) {
        finish(false)
      }
    })
    return probe
  }

  /** 宿主已确认接管时，把一次 UI 请求丢给它（不设短超时）。 */
  function askHost(payload, fallback) {
    return ensureHost().then(function (supported) {
      if (!supported) return fallback
      return new Promise(function (resolve) {
        var id = 'u' + ++seq
        var settled = false
        var done = function (v) {
          if (settled) return
          settled = true
          window.clearTimeout(timer)
          delete pending[id]
          resolve(v)
        }
        var timer = window.setTimeout(function () { done(fallback) }, ASK_TIMEOUT)
        pending[id] = {
          resolve: function (v) { done(v) },
          reject: function () { done(fallback) },
          timer: timer,
        }
        try {
          parent.postMessage(Object.assign({ __zapUi: 1, id: id }, payload), '*')
        } catch (e) {
          done(fallback)
        }
      })
    })
  }

  // ── 小工具 ────────────────────────────────────────
  function escapeHtml(s) {
    return String(s == null ? '' : s).replace(/[&<>"]/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]
    })
  }

  function node(idOrEl) {
    return typeof idOrEl === 'string' ? document.getElementById(idOrEl) : idOrEl
  }

  /** 建元素：el('div', { class: 'zui-row' }, [子节点 | 字符串] | html字符串) */
  function el(tag, attrs, children) {
    var n = document.createElement(tag)
    if (attrs) {
      for (var k in attrs) {
        if (!Object.prototype.hasOwnProperty.call(attrs, k)) continue
        var v = attrs[k]
        if (v == null || v === false) continue
        if (k === 'html') n.innerHTML = String(v)
        else if (k === 'text') n.textContent = String(v)
        else if (k.indexOf('on') === 0 && typeof v === 'function') n.addEventListener(k.slice(2).toLowerCase(), v)
        else n.setAttribute(k, v === true ? '' : String(v))
      }
    }
    if (children) {
      var list = Array.isArray(children) ? children : [children]
      for (var i = 0; i < list.length; i++) {
        if (list[i] == null) continue
        n.appendChild(typeof list[i] === 'string' ? document.createTextNode(list[i]) : list[i])
      }
    }
    return n
  }

  // ── 本地兜底吐司 ──────────────────────────────────
  function toast(message, type) {
    var box = document.querySelector('.zui-toasts')
    if (!box) {
      box = el('div', { class: 'zui-toasts' })
      document.body.appendChild(box)
    }
    var t = el('div', { class: 'zui-toast' + (type ? ' ' + type : ''), text: String(message) })
    box.appendChild(t)
    window.setTimeout(function () {
      if (t.parentNode) t.parentNode.removeChild(t)
    }, 3000)
  }

  /**
   * 通知条：优先让宿主弹 Element Plus 的 Message，宿主不接管时退化为界面内吐司。
   * @param {string} message
   * @param {'success'|'warning'|'error'|'info'} [type]
   */
  function notify(message, type) {
    return askHost({ kind: 'notify', message: String(message), type: type || 'info' }, false).then(function (handled) {
      if (!handled) toast(message, type === 'error' ? 'danger' : type === 'success' ? 'ok' : type === 'warning' ? 'warn' : '')
      return true
    })
  }

  /** 确认框：宿主用 ElMessageBox 渲染（返回 true / false），兜底用 window.confirm。 */
  function confirm(message, opts) {
    opts = opts || {}
    return askHost(
      { kind: 'confirm', message: String(message), title: opts.title || '', danger: !!opts.danger, confirmText: opts.confirmText || '', cancelText: opts.cancelText || '' },
      null,
    ).then(function (v) {
      return v === null ? window.confirm(String(message)) : !!v
    })
  }

  /** 输入框：返回输入的字符串，取消返回 null。 */
  function prompt(message, defaultValue, opts) {
    opts = opts || {}
    return askHost(
      { kind: 'prompt', message: String(message), value: defaultValue == null ? '' : String(defaultValue), title: opts.title || '' },
      null,
    ).then(function (v) {
      if (v !== null) return v
      var r = window.prompt(String(message), defaultValue == null ? '' : String(defaultValue))
      return r === null ? null : r
    })
  }

  // ── 表格渲染 ──────────────────────────────────────
  /**
   * 表格：zap.ui.table('#box', {
   *   columns: [{ key: 'name', label: '名称', width: '30%', render: fn(row) -> string|Node }],
   *   rows: [...], onRowClick: fn(row, index), empty: '无数据', key: 'id'
   * })
   */
  function table(target, spec) {
    var box = node(target)
    if (!box) return null
    spec = spec || {}
    var rows = spec.rows || []
    box.innerHTML = ''
    if (!rows.length) {
      box.appendChild(el('div', { class: 'zui-empty', text: spec.empty || '无数据' }))
      return null
    }
    var wrap = el('div', { class: 'zui-table-wrap' })
    var t = el('table', { class: 'zui-table' })
    var thead = el('thead')
    var htr = el('tr')
    ;(spec.columns || []).forEach(function (c) {
      htr.appendChild(el('th', { text: c.label || c.key, style: c.width ? 'width:' + c.width : null }))
    })
    thead.appendChild(htr)
    t.appendChild(thead)
    var tb = el('tbody')
    rows.forEach(function (row, idx) {
      var tr = el('tr')
      if (spec.rowClass) {
        var cls = spec.rowClass(row, idx)
        if (cls) tr.className = cls
      }
      ;(spec.columns || []).forEach(function (c) {
        var td = el('td')
        var v = row[c.key]
        if (typeof c.render === 'function') {
          var out = c.render(row, idx)
          if (out && out.nodeType) td.appendChild(out)
          else td.innerHTML = String(out == null ? '' : out)
        } else {
          td.textContent = v == null ? '' : String(v)
        }
        tr.appendChild(td)
      })
      if (typeof spec.onRowClick === 'function') {
        tr.addEventListener('click', function () { spec.onRowClick(row, idx) })
      }
      tb.appendChild(tr)
    })
    t.appendChild(tb)
    wrap.appendChild(t)
    box.appendChild(wrap)
    return t
  }

  // ── 页签渲染 ──────────────────────────────────────
  /**
   * 页签：zap.ui.tabs('#tabs', [{ id: 'status', label: '状态' }], function (id) { 切换 })
   * 返回 { select(id), active() }；面板容器用 [data-panel="<id>"] 标记，自动切 .is-active。
   */
  function tabs(target, list, onChange) {
    var box = node(target)
    if (!box) return null
    list = list || []
    box.innerHTML = ''
    box.className = 'zui-tabs'
    var current = null
    var btns = {}
    function show(id) {
      current = id
      list.forEach(function (t) {
        if (btns[t.id]) btns[t.id].className = 'zui-tab' + (t.id === id ? ' is-active' : '')
      })
      Array.prototype.forEach.call(document.querySelectorAll('[data-panel]'), function (p) {
        p.className = 'zui-panel' + (p.getAttribute('data-panel') === id ? ' is-active' : '')
      })
      if (typeof onChange === 'function') onChange(id)
    }
    list.forEach(function (t) {
      var b = el('button', { class: 'zui-tab', type: 'button', text: t.label || t.id })
      b.addEventListener('click', function () { show(t.id) })
      btns[t.id] = b
      box.appendChild(b)
    })
    if (list.length) show(list[0].id)
    return {
      select: show,
      active: function () { return current },
    }
  }

  // ── diff 着色 ─────────────────────────────────────
  /** 往 pre 里塞 diff 文本，并按 +/-/@@ 行上色。 */
  function diff(target, text, isErr) {
    var box = node(target)
    if (!box) return
    box.className = 'zui-pre' + (isErr ? ' err' : '')
    var lines = String(text == null ? '' : text).split('\n')
    box.innerHTML = ''
    lines.forEach(function (l) {
      var cls = ''
      if (l.charAt(0) === '+' && l.indexOf('+++') !== 0) cls = 'diff-add'
      else if (l.charAt(0) === '-' && l.indexOf('---') !== 0) cls = 'diff-del'
      else if (l.indexOf('@@') === 0) cls = 'diff-hunk'
      box.appendChild(el('span', { class: cls, text: l }))
      box.appendChild(document.createTextNode('\n'))
    })
  }

  // ── 忙碌态 ────────────────────────────────────────
  /** 锁/放界面上的按钮，避免连点触发并发任务。 */
  function busy(root, on, text) {
    var scope = node(root) || document
    Array.prototype.forEach.call(scope.querySelectorAll('button'), function (b) {
      b.disabled = !!on
    })
    var bar = document.querySelector('[data-status]')
    if (bar) {
      bar.className = on ? 'zui-tip' : 'zui-tip'
      bar.textContent = on ? text || '执行中…' : text || ''
    }
  }

  zap.ui = {
    version: '1.0',
    el: el,
    escape: escapeHtml,
    notify: notify,
    toast: toast,
    confirm: confirm,
    prompt: prompt,
    table: table,
    tabs: tabs,
    diff: diff,
    busy: busy,
  }
})()
