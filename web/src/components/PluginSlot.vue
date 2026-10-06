<template>
  <div class="plugin-slot">
    <!-- context 模式：入口由宿主（右键菜单）自己渲染，这里只保留弹窗，见 defineExpose -->
    <!-- 工具栏模式：内联「图标 + 文字」按钮，作为编辑器顶部工具栏的一部分 -->
    <template v-if="mode === 'toolbar'">
      <el-button
        v-for="p in plugins"
        :key="p.name"
        size="small"
        @click="openPlugin(p)"
      >
        <template v-if="p.icon" #icon><Icon :icon="p.icon" /></template>
        {{ p.label || p.title }}
      </el-button>
    </template>

    <!-- 列表模式（默认）：卡片 + 打开按钮 -->
    <template v-else-if="mode !== 'context'">
      <div v-if="loading" class="form-tip">加载插件…</div>
      <el-empty v-else-if="!plugins.length && !hideEmpty" :description="emptyText" :image-size="40" />
      <div v-else class="plugin-list">
        <el-card v-for="p in plugins" :key="p.name" shadow="never" class="plugin-card">
          <div class="plugin-head">
            <span class="plugin-title">{{ p.label || p.title }}</span>
            <el-button size="small" type="primary" @click="openPlugin(p)">
              {{ p.html ? '打开' : runLabel(p) }}
            </el-button>
          </div>
          <div v-if="p.tab" class="plugin-sub">{{ p.tab }}</div>
        </el-card>
      </div>
    </template>

    <el-dialog v-model="dialog" :title="current?.label || '运行插件'" width="480px">
      <el-form v-if="current" label-width="110px">
        <el-form-item v-for="opt in current.options || []" :key="opt.name" :label="opt.label">
          <el-input
            v-if="opt.type === 'string' || opt.type === 'number'"
            v-model="form[opt.name]"
            :placeholder="opt.placeholder"
          />
          <el-switch v-else-if="opt.type === 'bool'" v-model="boolVal[opt.name]" />
          <el-select
            v-else-if="opt.type === 'select'"
            v-model="form[opt.name]"
            :placeholder="opt.placeholder"
          >
            <el-option
              v-for="c in normChoices(opt.choices)"
              :key="c.value"
              :label="c.label"
              :value="c.value"
            />
          </el-select>
          <el-select
            v-else-if="opt.type === 'multiselect'"
            v-model="multiVal[opt.name]"
            multiple
            :placeholder="opt.placeholder"
          >
            <el-option
              v-for="c in normChoices(opt.choices)"
              :key="c.value"
              :label="c.label"
              :value="c.value"
            />
          </el-select>
          <div v-else-if="opt.type === 'dir'" class="dir-opt">
            <el-input
              v-model="form[opt.name]"
              :placeholder="opt.placeholder || '留空 = 站点根目录'"
              readonly
              style="flex: 1"
            >
              <template #append>
                <el-button @click="openDir(opt)">选择目录</el-button>
              </template>
            </el-input>
          </div>
          <div v-else-if="opt.type === 'file' || opt.type === 'files'" class="dir-opt">
            <el-input
              v-model="form[opt.name]"
              :placeholder="opt.placeholder || (opt.type === 'files' ? '可多选，留空 = 不选' : '留空 = 不选')"
              readonly
              style="flex: 1"
            >
              <template #append>
                <el-button @click="openFile(opt)">选择文件</el-button>
              </template>
            </el-input>
          </div>
          <div v-if="opt.desc" class="form-tip">{{ opt.desc }}</div>
        </el-form-item>
      </el-form>
      <pre v-if="result" class="plugin-log">{{ result }}</pre>
      <template #footer>
        <el-button @click="dialog = false">关闭</el-button>
        <el-button
          v-if="running && current?.async"
          :loading="cancelling"
          @click="cancelRun"
        >
          取消
        </el-button>
        <el-button type="primary" :loading="running" @click="run">{{ runLabel(current) }}</el-button>
      </template>
    </el-dialog>

    <!-- 自带 HTML 界面的插件：内容塞进沙箱 iframe，脚本只能走 postMessage 回调后端 -->
    <el-dialog
      v-model="htmlDialog"
      :title="current?.label || '插件'"
      width="880px"
      top="6vh"
      @closed="htmlDoc = ''"
    >
      <div v-loading="htmlLoading" class="html-plugin">
        <iframe
          v-if="htmlDoc"
          ref="frameRef"
          class="plugin-frame"
          title="plugin-ui"
          sandbox="allow-scripts"
          :srcdoc="htmlDoc"
        />
        <div v-else-if="!htmlLoading" class="form-tip">未能加载插件界面</div>
      </div>
      <template #footer>
        <el-button @click="htmlDialog = false">关闭</el-button>
      </template>
    </el-dialog>

    <DirPicker
      v-model="dirVisible"
      :start-path="dirStartPath"
      title="选择目标目录"
      @confirm="onDirConfirm"
    />

    <FilePicker
      v-model="fileVisible"
      :multiple="fileMultiple"
      :start-path="fileStartPath"
      title="选择文件"
      @confirm="onFileConfirm"
    />
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import DirPicker from '@/components/DirPicker.vue'
import FilePicker from '@/components/FilePicker.vue'
import {
  pluginList,
  pluginRun,
  pluginCancel,
  pluginUi,
  pluginGetConfig,
  pluginSetConfig,
  type PluginInfo,
  type PluginOption,
} from '@/api/plugin'
import { getToken } from '@/utils/auth'
import { API_BASE } from '@/utils/base'
import { Icon } from '@/icons'

const props = defineProps<{
  placementSlot: string
  siteId?: number
  webRoot?: string
  /**
   * 显示模式：
   * - list=卡片列表（默认，站点详情「插件」页）
   * - toolbar=内联「图标+文字」按钮，用于编辑器工具栏等紧凑场景
   * - context=不渲染入口，由宿主（右键菜单等）通过 ref 调 openPlugin 拉起，见 defineExpose
   */
  mode?: 'list' | 'toolbar' | 'context'
  /** 当前工作目录（绝对路径）：随每次调用透传给插件（如 Git 插件在此目录下执行）。 */
  cwd?: string
  /** 当前选中的文件路径（file.context 槽位右键所选），随调用透传给插件。 */
  files?: string[]
  /** 列表模式的空态文案 */
  emptyText?: string
  /** 列表模式下没有插件时连空态一起隐藏（宿主自己控制要不要显示这块区域） */
  hideEmpty?: boolean
}>()

const loading = ref(false)
const plugins = ref<PluginInfo[]>([])
const emptyText = ref(props.emptyText || '该位置暂无可用插件')
const dialog = ref(false)
const running = ref(false)
const result = ref('')
const current = ref<PluginInfo | null>(null)
const currentTaskId = ref('') // 异步任务的 task_id，用于取消
const cancelling = ref(false) // 取消请求进行中
const form = reactive<Record<string, string>>({})
const boolVal = reactive<Record<string, boolean>>({})
const multiVal = reactive<Record<string, string[]>>({})

// iframe（ui.html）通过 zap.pickDir() / zap.pickFile() 发起的选择请求（pending）；
// 选择器确认后由 replyPick 把结果回传 iframe，与表单内的 dir/file 选项共用同一套组件。
const pickerReq = ref<{ id: string; type: 'dir' | 'file'; multiple: boolean } | null>(null)

// 目录选择器：从站点根（webRoot）出发选目录，返回相对于站点根的相对路径，
// 与插件 main.lua 里 `site_root .. "/" .. target` 的语义一致；无 webRoot 时返回绝对路径。
const dirVisible = ref(false)
const dirTarget = ref('')
const dirStartPath = ref('')
function openDir(opt: PluginOption) {
  dirTarget.value = opt.name
  dirStartPath.value = props.webRoot || ''
  dirVisible.value = true
}
function onDirConfirm(absPath: string) {
  const root = props.webRoot
  let rel = absPath
  if (root && absPath.startsWith(root)) {
    rel = absPath.slice(root.length).replace(/^\/+/, '')
  }
  // iframe 发起的选择请求：把结果回传 iframe，不写表单字段
  if (pickerReq.value?.type === 'dir') {
    replyPick(pickerReq.value.id, rel)
    pickerReq.value = null
    dirVisible.value = false
    return
  }
  form[dirTarget.value] = rel
  dirVisible.value = false
}

// 文件选择器（file 单选 / files 多选）：从站点根（webRoot）或家目录出发选文件，回传绝对路径。
// 多选用空格连接成单个字符串（与 multiselect 一致），插件侧用 zap.option 读取后自行 split。
const fileVisible = ref(false)
const fileTarget = ref('')
const fileMultiple = ref(false)
const fileStartPath = ref('')
function openFile(opt: PluginOption) {
  fileTarget.value = opt.name
  fileMultiple.value = opt.type === 'files'
  fileStartPath.value = props.webRoot || ''
  fileVisible.value = true
}
function onFileConfirm(paths: string[]) {
  // iframe 发起的选择请求：回传绝对路径（多选用空格连接，与表单 files 一致），不写表单字段
  if (pickerReq.value?.type === 'file') {
    const data = pickerReq.value.multiple ? (paths || []).join(' ') : (paths?.[0] || '')
    replyPick(pickerReq.value.id, data)
    pickerReq.value = null
    fileVisible.value = false
    return
  }
  form[fileTarget.value] = (paths || []).join(' ')
  fileVisible.value = false
}

/** iframe 通过 zap.pickDir() / zap.pickFile() 请求打开父页面的选择器（复用页面上的组件）。 */
function openPicker(d: any) {
  if (d.pick === 'file') {
    pickerReq.value = { id: d.id, type: 'file', multiple: !!d.multiple }
    fileMultiple.value = !!d.multiple
    fileStartPath.value = (d.startPath as string) || props.webRoot || ''
    fileVisible.value = true
  } else {
    pickerReq.value = { id: d.id, type: 'dir', multiple: false }
    dirStartPath.value = (d.startPath as string) || props.webRoot || ''
    dirVisible.value = true
  }
}

/** 把选择结果回传给 iframe（与 RPC 桥的 pending 约定一致：{ ok, data }）。 */
function replyPick(id: string, data: any) {
  frameRef.value?.contentWindow?.postMessage({ __zapRpc: 1, id, ok: true, data }, '*')
}

/** 取某个 action 的元信息（async / dangerous），manifest 用旧写法时为空。 */
function actionSpec(action: string): Record<string, any> | undefined {
  const list = (current.value?.action_specs || []) as Record<string, any>[]
  return list.find((a) => a?.name === action)
}
/** 是否走异步（SSE 日志流）：插件级 async 或该 action 自己声明的 async。 */
function isAsyncAction(p: PluginInfo | null, action: string) {
  return !!p?.async || !!actionSpec(action)?.async
}
/**
 * 破坏性动作的二次确认：manifest 标了 dangerous 的动作在宿主侧统一拦一道。
 * 接口另有确认时插件自己不再重复；后端也会挡只读演示账号，这里只是让交互更清楚。
 */
async function confirmDangerous(action: string): Promise<boolean> {
  const spec = actionSpec(action)
  if (!spec?.dangerous) return true
  try {
    await ElMessageBox.confirm(
      `「${spec.label || action}」是不可撤销的破坏性操作，确定继续？`,
      '危险操作确认',
      { type: 'warning', confirmButtonText: '继续', cancelButtonText: '取消' },
    )
    return true
  } catch {
    return false
  }
}

type Choice = { label: string; value: string }
function normChoices(c?: (string | Choice)[]): Choice[] {
  return (c || []).map((x) =>
    typeof x === 'string' ? { label: x, value: x } : { label: x.label, value: x.value },
  )
}
function runLabel(p?: PluginInfo | null) {
  if (!p) return '运行'
  const keys = p.actions ? Object.keys(p.actions) : []
  const a = keys[0]
  return a ? (p.actions as any)[a] : '运行'
}
/**
 * 插件界面 ↔ 面板的 RPC 桥。
 *
 * 注入到 iframe 的 `srcdoc` 里，给插件页面提供 `window.zap.call(action, options, onLine)`。
 * iframe 挂的是 `sandbox="allow-scripts"`（**不含 allow-same-origin**），所以它是
 * 不透明源：碰不到面板 DOM / Cookie / localStorage，也发不出带凭据的请求 ——
 * 要调后端只能 postMessage 给父页面，由父页面带着真实 JWT 代跑 `/plugin/run`，
 * 权限点与 scope 仍旧在后端把关。
 */
const RPC_BRIDGE = `<script>
(function () {
  var seq = 0, pending = {};
  window.addEventListener('message', function (ev) {
    var d = ev.data;
    if (!d || d.__zapRpc !== 1 || !d.id) return;
    var p = pending[d.id];
    if (!p) return;
    // 流式日志：持续推送，不结算 Promise
    if (d.stream === true) { if (typeof p.onLine === 'function') p.onLine(d.line); return; }
    delete pending[d.id];
    if (d.ok) p.resolve(d.data); else p.reject(new Error(d.data || '调用失败'));
  });
  function call(action, options, onLine) {
    return new Promise(function (resolve, reject) {
      var id = 'r' + (++seq);
      pending[id] = { resolve: resolve, reject: reject, onLine: onLine };
      parent.postMessage(
        { __zapRpc: 1, id: id, action: action, options: options || {}, stream: typeof onLine === 'function' },
        '*'
      );
    });
  }
  window.zap = {
    call: call,
    run: function (options, onLine) { return call('run', options, onLine); },
    // 调用父页面的目录 / 文件选择器（与 manifest options 里的 type:dir / type:file 同一套组件），
    // 返回 Promise：resolve 选择结果（dir 回传相对站点根路径；file 回传绝对路径，多选用空格连接）
    pickDir: function (opts) {
      return new Promise(function (resolve, reject) {
        var id = 'p' + (++seq);
        pending[id] = { resolve: resolve, reject: reject };
        parent.postMessage(
          { __zapRpc: 1, id: id, pick: 'dir', startPath: (opts && opts.startPath) || '', title: (opts && opts.title) || '' },
          '*'
        );
      });
    },
    pickFile: function (opts) {
      return new Promise(function (resolve, reject) {
        var id = 'p' + (++seq);
        pending[id] = { resolve: resolve, reject: reject };
        parent.postMessage(
          { __zapRpc: 1, id: id, pick: 'file', multiple: !!(opts && opts.multiple), startPath: (opts && opts.startPath) || '', title: (opts && opts.title) || '' },
          '*'
        );
      });
    },
    // 持久化配置（KV，按插件 + 当前面板用户）：与 Lua 侧的 zap.config 同一份数据，
    // 界面可以记住用户上次的选择；settings 为 null / 空串即删除该键。
    configGet: function (key) {
      return new Promise(function (resolve, reject) {
        var id = 'c' + (++seq);
        pending[id] = { resolve: resolve, reject: reject };
        parent.postMessage({ __zapRpc: 1, id: id, configGet: true, key: key }, '*');
      });
    },
    configSet: function (key, value) {
      return new Promise(function (resolve, reject) {
        var id = 'c' + (++seq);
        pending[id] = { resolve: resolve, reject: reject };
        parent.postMessage({ __zapRpc: 1, id: id, configSet: true, key: key, value: value }, '*');
      });
    }
  };
})();
<\/script>
`

const htmlDialog = ref(false)
const htmlLoading = ref(false)
const htmlDoc = ref('')
const frameRef = ref<HTMLIFrameElement | null>(null)

/** 把后端响应收敛成插件拿得到的文本（同步插件是 log，异步插件是累积日志）。 */
function toText(payload: any): string {
  if (typeof payload?.log === 'string') return payload.log
  if (typeof payload === 'string') return payload
  return JSON.stringify(payload ?? null, null, 2)
}

function reply(id: string, ok: boolean, data: any) {
  frameRef.value?.contentWindow?.postMessage({ __zapRpc: 1, id, ok, data }, '*')
}

/** 异步插件：订阅 SSE 把日志攒起来，可选逐行回推给 iframe。 */
function watchFrameLog(
  logPath: string,
  id: string,
  stream: boolean,
  resolve: (s: string) => void,
  reject: (e: Error) => void,
) {
  const url = `${API_BASE}/plugin/watch?token=${encodeURIComponent(getToken())}&log_path=${encodeURIComponent(logPath)}`
  const es = new EventSource(url)
  let acc = ''
  const finish = (code?: number) => {
    es.close()
    if (code === undefined || code === 0) resolve(acc)
    else reject(new Error(acc || `插件退出码 ${code}`))
  }
  es.onmessage = (ev) => {
    try {
      const d = JSON.parse(ev.data)
      if (d.type === 'done') return finish(d.code)
      if (d.type === 'log') {
        acc += d.line + '\n'
        if (stream) {
          frameRef.value?.contentWindow?.postMessage(
            { __zapRpc: 1, id, stream: true, line: d.line },
            '*',
          )
        }
      }
    } catch {
      acc += ev.data + '\n'
    }
  }
  es.onerror = () => finish()
}

/**
 * 插件界面的持久化配置：转发 `/plugin/config`。
 *
 * 与 Lua 里的 `zap.config` 是同一份数据（都按「插件 + 面板用户」分桶），
 * 于是界面也能记住用户上次的选择，不必每次都让用户重填。
 */
async function handleConfig(d: any) {
  const p = current.value
  if (!p) return reply(d.id, false, '插件上下文已关闭')
  try {
    if (d.configGet) {
      const r: any = await pluginGetConfig(p.name)
      const payload = r?.data?.data ?? r?.data ?? {}
      const cfg = payload?.config ?? {}
      return reply(d.id, true, cfg[String(d.key)] ?? null)
    }
    await pluginSetConfig(p.name, { [String(d.key)]: String(d.value ?? '') })
    return reply(d.id, true, true)
  } catch (e: any) {
    return reply(d.id, false, e?.message || '配置读写失败')
  }
}

/** 处理 iframe 发来的调用：转发成 `/plugin/run`，再把结果 post 回去。 */
async function handleRpc(d: any) {
  const p = current.value
  if (!p) return reply(d.id, false, '插件上下文已关闭')
  const action = String(d.action || 'run')
  if (!(await confirmDangerous(action))) return reply(d.id, false, '已取消')
  const options: Record<string, string> = {}
  if (d.options && typeof d.options === 'object') {
    for (const [k, v] of Object.entries(d.options)) options[k] = String(v)
  }
  // 透传当前工作目录与右键选中的文件
  if (props.cwd && !options.cwd) options.cwd = props.cwd
  if (props.files?.length && !options.files) options.files = JSON.stringify(props.files)
  try {
    const r: any = await pluginRun({ name: p.name, action, site_id: props.siteId, options })
    const body = r?.data ?? r
    const payload = body?.data ?? body
    if (payload && payload.async) {
      await new Promise<string>((resolve, reject) =>
        watchFrameLog(payload.log_path, d.id, d.stream === true, resolve, reject),
      ).then(
        (out) => reply(d.id, true, out),
        (e) => reply(d.id, false, e?.message || String(e)),
      )
    } else {
      reply(d.id, true, toText(payload))
    }
  } catch (e: any) {
    reply(d.id, false, e?.message || String(e))
  }
}

/**
 * 接管 UIKit 的 UI 请求（`zap.ui.notify / confirm / prompt`）。
 *
 * 插件界面在沙箱 iframe 里，拿不到 Element Plus；这些请求由宿主用面板同一套组件渲染，
 * 插件界面因此不必自己写吐司 / 确认框，也不用再 `window.confirm` 那种原生弹窗。
 * 协议见 data/plugins/_lib/ui.js：`{ __zapUi: 1, id, kind, ... }`，回 `{ __zapUi: 1, id, ok, data }`。
 */
async function handleUi(d: any, frame: HTMLIFrameElement) {
  const send = (ok: boolean, data: any) =>
    frame.contentWindow?.postMessage({ __zapUi: 1, id: d.id, ok, data }, '*')
  try {
    if (d.kind === 'ping') return send(true, true)
    if (d.kind === 'notify') {
      const types: Record<string, any> = { success: 'success', warning: 'warning', error: 'error', info: 'info' }
      ElMessage({ message: String(d.message || ''), type: types[String(d.type || 'info')] || 'info' })
      return send(true, true)
    }
    if (d.kind === 'confirm') {
      await ElMessageBox.confirm(String(d.message || ''), d.title || '请确认', {
        type: d.danger ? 'warning' : 'info',
        confirmButtonText: d.confirmText || '确定',
        cancelButtonText: d.cancelText || '取消',
      })
      return send(true, true)
    }
    if (d.kind === 'prompt') {
      const r = await ElMessageBox.prompt(String(d.message || ''), d.title || '请输入', {
        inputValue: String(d.value || ''),
        confirmButtonText: '确定',
        cancelButtonText: '取消',
      })
      return send(true, String(r?.value ?? ''))
    }
    return send(false, '未知的 UI 请求')
  } catch {
    // 用户取消：confirm 回 false，prompt 回 null（不是错误，别让插件的 await 抛异常）
    return send(true, d.kind === 'prompt' ? null : false)
  }
}

function onWindowMessage(ev: MessageEvent) {
  // 只认自己这个 iframe 发来的消息：判 source，别把页面上其它 postMessage 当指令
  const frame = frameRef.value
  if (!frame || ev.source !== frame.contentWindow) return
  const d = ev.data
  if (!d || typeof d !== 'object') return
  if (d.__zapUi === 1 && d.id) {
    void handleUi(d, frame)
    return
  }
  if (d.__zapRpc !== 1 || !d.id) return
  // iframe 请求打开目录 / 文件选择器（zap.pickDir / zap.pickFile）
  if (d.pick) {
    openPicker(d)
    return
  }
  // 插件界面的持久化配置读写（zap.configGet / zap.configSet）
  if (d.configGet || d.configSet) {
    void handleConfig(d)
    return
  }
  void handleRpc(d)
}

/** 自带 HTML 界面的插件：拉取 `ui.html` 并在沙箱 iframe 里渲染。 */
async function openHtml(p: PluginInfo) {
  current.value = p
  htmlDoc.value = ''
  htmlDialog.value = true
  htmlLoading.value = true
  try {
    const r: any = await pluginUi({ name: p.name })
    const body = r?.data ?? r
    const payload = body?.data ?? body
    const html = String(payload?.html || '')
    if (!html) {
      ElMessage.error('插件界面为空')
      htmlDialog.value = false
      return
    }
    htmlDoc.value = RPC_BRIDGE + html
  } catch (e: any) {
    ElMessage.error(e?.message || '加载插件界面失败')
    htmlDialog.value = false
  } finally {
    htmlLoading.value = false
  }
}

function openPlugin(p: PluginInfo) {
  current.value = p
  if (p.html) void openHtml(p)
  else openRun(p)
}

// file.context 这类槽位：入口由宿主（右键菜单）渲染，通过 ref 拿到插件列表并拉起界面。
defineExpose({ openPlugin, plugins })

function openRun(p: PluginInfo) {
  current.value = p
  result.value = ''
  for (const k of Object.keys(form)) delete form[k]
  for (const opt of p.options || []) {
    if (opt.type === 'bool') boolVal[opt.name] = opt.default === 'true'
    else if (opt.type === 'multiselect') multiVal[opt.name] = opt.default ? [opt.default] : []
    else form[opt.name] = opt.default || ''
  }
  dialog.value = true
}
async function run() {
  if (!current.value) return
  const act = Object.keys(current.value.actions || {})[0] || 'run'
  if (!(await confirmDangerous(act))) return
  running.value = true
  result.value = ''
  const options: Record<string, string> = {}
  for (const opt of current.value.options || []) {
    if (opt.type === 'bool') options[opt.name] = boolVal[opt.name] ? 'true' : 'false'
    else if (opt.type === 'multiselect')
      options[opt.name] = (multiVal[opt.name] || []).join(' ')
    else options[opt.name] = form[opt.name] || ''
  }
  // 透传当前工作目录与选中文件（同 handleRpc）
  if (props.cwd && !options.cwd) options.cwd = props.cwd
  if (props.files?.length && !options.files) options.files = JSON.stringify(props.files)
  let isAsync = false
  try {
    const r: any = await pluginRun({
      name: current.value.name,
      action: act,
      site_id: props.siteId,
      options,
    })
    const payload = r?.data?.data ?? r?.data
    // 异步插件：开 SSE 订阅日志流（实时逐行显示），running 由流收尾，这里不重置
    if (payload && payload.async) {
      isAsync = true
      currentTaskId.value = payload.task_id || ''
      cancelling.value = false
      watchLog(payload.log_path)
    } else {
      // 同步插件：一次性拿回结果
      result.value =
        typeof payload?.log === 'string'
          ? payload.log
          : typeof payload === 'string'
            ? payload
            : JSON.stringify(payload, null, 2)
      ElMessage.success('执行完成')
    }
  } catch (e: any) {
    result.value = e?.message || String(e)
    ElMessage.error('执行失败')
  } finally {
    // 异步插件的 running 由 watchLog 在流结束时关闭，这里只收尾同步分支
    if (!isAsync) running.value = false
  }
}

/// 通过 SSE 订阅异步插件的日志流，逐行追加到 result。
function watchLog(logPath: string) {
  const url = `${API_BASE}/plugin/watch?token=${encodeURIComponent(getToken())}&log_path=${encodeURIComponent(logPath)}`
  const es = new EventSource(url)
  es.onmessage = (ev) => {
    try {
      const d = JSON.parse(ev.data)
      if (d.type === 'done') {
        es.close()
        running.value = false
        currentTaskId.value = ''
        cancelling.value = false
        if (d.code === 0) ElMessage.success('执行完成')
        else if (d.code === -2) ElMessage.warning('任务已取消')
        else ElMessage.error('执行失败')
      } else if (d.type === 'log') {
        result.value += d.line + '\n'
      }
    } catch {
      result.value += ev.data + '\n'
    }
  }
  es.onerror = () => {
    // 网络中断或流异常结束
    es.close()
    if (running.value) {
      running.value = false
      currentTaskId.value = ''
      cancelling.value = false
      ElMessage.error('日志流连接中断')
    }
  }
}

/// 取消正在运行的异步插件。
function cancelRun() {
  if (!currentTaskId.value) return
  cancelling.value = true
  pluginCancel(currentTaskId.value)
    .then(() => {
      // 实际结束（含「任务已取消」日志）由 SSE 流推送，这里只等流收尾
      ElMessage.info('正在取消…')
    })
    .catch((e: any) => {
      cancelling.value = false
      ElMessage.error(e?.message || '取消请求失败')
    })
}
onMounted(async () => {
  window.addEventListener('message', onWindowMessage)
  await load()
})

onUnmounted(() => window.removeEventListener('message', onWindowMessage))

async function load() {
  loading.value = true
  try {
    // lang 由 api/plugin.ts 自动带上（跟随面板语言，含 Element Plus 切换）
    const r: any = await pluginList({ slot: props.placementSlot, site_id: props.siteId })
    const data = r?.data?.data ?? r?.data
    plugins.value = Array.isArray(data) ? data : []
  } catch {
    plugins.value = []
  } finally {
    loading.value = false
  }
}

// 面板语言切换后插件文案要跟着变：
// 列表那部分（标题 / 动作名）重新拉一次即可；已经打开的 HTML 界面里那个
// `__ZAP_LANG__` 是注入时写死的，必须整份重取才会更新。
const { locale } = useI18n({ useScope: 'global' })
watch(locale, async () => {
  const reopened = htmlDialog.value && current.value?.html ? current.value : null
  await load()
  if (reopened) await openHtml(reopened)
})
</script>

<style scoped>
.plugin-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.plugin-card {
  margin-bottom: 0;
}
.plugin-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.plugin-title {
  font-weight: 600;
}
.plugin-sub {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  margin-top: 4px;
}
.dir-opt {
  display: flex;
  width: 100%;
}
.plugin-log {
  background: var(--el-fill-color-light);
  border-radius: 6px;
  padding: 10px;
  max-height: 240px;
  overflow: auto;
  white-space: pre-wrap;
  font-size: 12px;
}
/* 插件自带界面：iframe 由插件自己撑高度，最小高度保证空页面也不塌 */
.html-plugin {
  min-height: 120px;
}
.plugin-frame {
  width: 100%;
  min-height: 360px;
  height: 60vh;
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
  background: #fff;
  display: block;
}
</style>
