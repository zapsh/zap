<template>
  <div class="plugin-slot">
    <div v-if="loading" class="form-tip">加载插件…</div>
    <el-empty v-else-if="!plugins.length" :description="emptyText" :image-size="40" />
    <div v-else class="plugin-list">
      <el-card v-for="p in plugins" :key="p.name" shadow="never" class="plugin-card">
        <div class="plugin-head">
          <span class="plugin-title">{{ p.label || p.title }}</span>
          <el-button size="small" type="primary" @click="openRun(p)">{{ runLabel(p) }}</el-button>
        </div>
        <div v-if="p.tab" class="plugin-sub">{{ p.tab }}</div>
      </el-card>
    </div>

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
        <el-button type="primary" :loading="running" @click="run">{{ runLabel(current) }}</el-button>
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
import { onMounted, reactive, ref } from 'vue'
import { ElMessage } from 'element-plus'
import DirPicker from '@/components/DirPicker.vue'
import FilePicker from '@/components/FilePicker.vue'
import { pluginList, pluginRun, type PluginInfo, type PluginOption } from '@/api/plugin'

const props = defineProps<{ placementSlot: string; siteId?: number; webRoot?: string }>()

const loading = ref(false)
const plugins = ref<PluginInfo[]>([])
const emptyText = ref('该位置暂无可用插件')
const dialog = ref(false)
const running = ref(false)
const result = ref('')
const current = ref<PluginInfo | null>(null)
const form = reactive<Record<string, string>>({})
const boolVal = reactive<Record<string, boolean>>({})
const multiVal = reactive<Record<string, string[]>>({})

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
  form[fileTarget.value] = (paths || []).join(' ')
  fileVisible.value = false
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
  running.value = true
  result.value = ''
  const options: Record<string, string> = {}
  for (const opt of current.value.options || []) {
    if (opt.type === 'bool') options[opt.name] = boolVal[opt.name] ? 'true' : 'false'
    else if (opt.type === 'multiselect')
      options[opt.name] = (multiVal[opt.name] || []).join(' ')
    else options[opt.name] = form[opt.name] || ''
  }
  try {
    const r: any = await pluginRun({
      name: current.value.name,
      action: Object.keys(current.value.actions || {})[0] || 'run',
      site_id: props.siteId,
      options,
    })
    const data = r?.data?.data ?? r?.data
    result.value = typeof data === 'string' ? data : JSON.stringify(data, null, 2)
    ElMessage.success('执行完成')
  } catch (e: any) {
    result.value = e?.message || String(e)
    ElMessage.error('执行失败')
  } finally {
    running.value = false
  }
}
onMounted(async () => {
  loading.value = true
  try {
    const r: any = await pluginList({ slot: props.placementSlot, site_id: props.siteId })
    const data = r?.data?.data ?? r?.data
    plugins.value = Array.isArray(data) ? data : []
  } catch {
    plugins.value = []
  } finally {
    loading.value = false
  }
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
</style>
