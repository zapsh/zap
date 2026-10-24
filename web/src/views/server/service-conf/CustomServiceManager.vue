<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="custom-mgr">
    <div class="head">
      <div class="head-text">
        <div class="h-title">{{ t('servicesCustom.title') }}</div>
        <div class="h-desc">{{ t('servicesCustom.desc') }}</div>
      </div>
      <div class="head-actions">
        <el-button :loading="loading" size="small" @click="loadDefs">
          {{ t('servicesCustom.refresh') }}
        </el-button>
        <el-button type="primary" size="small" @click="openCreate">
          {{ t('servicesCustom.createNew') }}
        </el-button>
      </div>
    </div>

    <el-table v-loading="loading" :data="defs" stripe class="mt-3">
      <el-table-column prop="key" label="Key" min-width="140">
        <template #default="{ row }">
          <span class="mono">{{ row.key }}</span>
        </template>
      </el-table-column>
      <el-table-column prop="label" :label="t('servicesCustom.label')" min-width="140" />
      <el-table-column prop="format" :label="t('servicesCustom.format')" width="90" />
      <el-table-column prop="manager" :label="t('servicesCustom.manager')" width="110">
        <template #default="{ row }">
          <el-tag size="small" :type="row.manager === 'openrc' ? 'success' : 'warning'">
            {{ row.manager || 'systemd' }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="t('servicesCustom.origin')" width="110">
        <template #default="{ row }">
          <el-tag size="small" :type="row.origin === 'user' ? 'primary' : 'info'">
            {{ row.origin === 'user' ? t('servicesCustom.originUser') : t('servicesCustom.originBuiltin') }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column prop="fields" :label="t('servicesCustom.fields')" width="80" align="center" />
      <el-table-column :label="t('servicesCustom.registered')" width="110" align="center">
        <template #default="{ row }">
          <el-tooltip
            :content="row.has_unit ? '' : t('servicesCustom.noUnitTip')"
            :disabled="row.has_unit"
          >
            <el-switch
              :model-value="!!row.registered"
              :disabled="!row.has_unit || busyKeys.has(row.key)"
              :loading="busyKeys.has(row.key)"
              @change="(v: boolean) => toggleRegister(row, v)"
            />
          </el-tooltip>
        </template>
      </el-table-column>
      <el-table-column :label="t('common.operation')" min-width="160" fixed="right">
        <template #default="{ row }">
          <el-button
            size="small"
            :disabled="row.origin !== 'user'"
            @click="openEdit(row)"
          >
            {{ t('servicesCustom.edit') }}
          </el-button>
          <el-button
            size="small"
            type="danger"
            :disabled="row.origin !== 'user'"
            @click="remove(row)"
          >
            {{ t('servicesCustom.delete') }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>

    <!-- 新建 / 编辑对话框 -->
    <el-dialog
      v-model="dialog"
      :title="editing ? t('servicesCustom.edit') : t('servicesCustom.createNew')"
      width="840px"
      top="5vh"
      @closed="resetForm"
    >
      <el-form label-width="120px" label-position="right">
        <el-row :gutter="16">
          <el-col :span="12">
            <el-form-item :label="t('servicesCustom.key')">
              <el-input
                v-model="form.key"
                :disabled="editing"
                :placeholder="t('servicesCustom.keyPlaceholder')"
              />
            </el-form-item>
          </el-col>
          <el-col :span="12">
            <el-form-item :label="t('servicesCustom.label')">
              <el-input v-model="form.label" :placeholder="t('servicesCustom.labelPlaceholder')" />
            </el-form-item>
          </el-col>
          <el-col :span="24">
            <el-form-item :label="t('servicesCustom.execStart')">
              <el-input
                v-model="form.exec_start"
                :disabled="svcRunning"
                :placeholder="t('servicesCustom.execStartPlaceholder')"
              />
            </el-form-item>
            <el-alert
              v-if="svcRunning"
              type="warning"
              :closable="false"
              show-icon
              class="field-tip"
              :title="t('servicesCustom.execStartLockedTip')"
            />
          </el-col>
          <el-col :span="24">
            <el-form-item :label="t('servicesCustom.workingDir')">
              <el-input v-model="form.working_dir" :placeholder="t('servicesCustom.workingDirPlaceholder')" />
            </el-form-item>
          </el-col>
          <el-col :span="12">
            <el-form-item :label="t('servicesCustom.unitType')">
              <el-select v-model="form.type" class="w-full">
                <el-option v-for="opt in unitTypeOptions" :key="opt" :label="opt" :value="opt" />
              </el-select>
            </el-form-item>
          </el-col>
          <el-col :span="12">
            <el-form-item :label="t('servicesCustom.unitUser')">
              <el-input
                v-model="form.user"
                :placeholder="t('servicesCustom.unitUserPlaceholder')"
              />
            </el-form-item>
          </el-col>
          <el-col :span="12">
            <el-form-item :label="t('servicesCustom.unitGroup')">
              <el-input
                v-model="form.group"
                :placeholder="t('servicesCustom.unitGroupPlaceholder')"
              />
            </el-form-item>
          </el-col>
        </el-row>

        <!-- 重启策略：Restart / RestartSec / StartLimit*，默认折叠，集中设置 -->
        <el-collapse v-model="restartCollapse" class="restart-collapse">
          <el-collapse-item name="restart" :title="t('servicesCustom.restartPolicy')">
            <el-row :gutter="16">
              <el-col :span="12">
                <el-form-item :label="t('servicesCustom.unitRestart')">
                  <el-select v-model="form.restart" class="w-full">
                    <el-option :label="t('servicesCustom.unitRestartDefault')" value="" />
                    <el-option label="no" value="no" />
                    <el-option label="on-success" value="on-success" />
                    <el-option label="on-failure" value="on-failure" />
                    <el-option label="always" value="always" />
                  </el-select>
                </el-form-item>
              </el-col>
              <el-col :span="12">
                <el-form-item :label="t('servicesCustom.unitRestartSec')">
                  <el-input
                    v-model="form.restart_sec"
                    :placeholder="t('servicesCustom.unitRestartSecPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="12">
                <el-form-item :label="t('servicesCustom.unitStartLimitInterval')">
                  <el-input
                    v-model="form.start_limit_interval_sec"
                    :placeholder="t('servicesCustom.unitStartLimitIntervalPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="12">
                <el-form-item :label="t('servicesCustom.unitStartLimitBurst')">
                  <el-input
                    v-model="form.start_limit_burst"
                    :placeholder="t('servicesCustom.unitStartLimitBurstPlaceholder')"
                  />
                </el-form-item>
              </el-col>
            </el-row>
          </el-collapse-item>
        </el-collapse>

        <!-- 高级选项：配置文件 / 环境变量 / 启动前命令 / 服务描述 / 自定义指令，默认折叠 -->
        <el-collapse v-model="advCollapse" class="adv-collapse">
          <el-collapse-item name="adv" :title="t('servicesCustom.advOptions')">
            <el-row :gutter="16">
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.configFile')">
                  <el-input
                    v-model="form.main"
                    type="textarea"
                    :rows="3"
                    :placeholder="t('servicesCustom.configFilePlaceholder')"
                  />
                  <div class="field-hint">{{ t('servicesCustom.configFileHint') }}</div>
                </el-form-item>
              </el-col>
              <el-col v-if="form.main.trim()" :span="24">
                <el-form-item :label="t('servicesCustom.format')">
                  <el-select v-model="form.format" class="w-full">
                    <el-option label="ini" value="ini" />
                    <el-option label="json" value="json" />
                  </el-select>
                  <div class="field-hint">{{ t('servicesCustom.formatHint') }}</div>
                </el-form-item>
              </el-col>
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.environment')">
                  <el-input
                    v-model="form.environment"
                    type="textarea"
                    :rows="3"
                    :placeholder="t('servicesCustom.envPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.precmd')">
                  <el-input
                    v-model="form.precmd"
                    type="textarea"
                    :rows="3"
                    :placeholder="t('servicesCustom.precmdPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.unitDescription')">
                  <el-input
                    v-model="form.description"
                    :placeholder="t('servicesCustom.unitDescriptionPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.unitExtra')">
                  <el-input
                    v-model="form.extra"
                    type="textarea"
                    :rows="4"
                    :placeholder="t('servicesCustom.unitExtraPlaceholder')"
                  />
                  <div class="field-hint">{{ t('servicesCustom.unitExtraHint') }}</div>
                </el-form-item>
              </el-col>
            </el-row>
          </el-collapse-item>
        </el-collapse>

        <!-- 启动依赖（After / Wants / Requires）：默认折叠，减少常用项干扰 -->
        <el-collapse v-model="depsCollapse" class="deps-collapse">
          <el-collapse-item name="deps" :title="t('servicesCustom.unitDeps')">
            <el-row :gutter="16">
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.unitAfter')">
                  <el-input
                    v-model="form.after"
                    type="textarea"
                    :rows="2"
                    :placeholder="t('servicesCustom.unitAfterPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.unitWants')">
                  <el-input
                    v-model="form.wants"
                    type="textarea"
                    :rows="2"
                    :placeholder="t('servicesCustom.unitWantsPlaceholder')"
                  />
                </el-form-item>
              </el-col>
              <el-col :span="24">
                <el-form-item :label="t('servicesCustom.unitRequires')">
                  <el-input
                    v-model="form.requires"
                    type="textarea"
                    :rows="2"
                    :placeholder="t('servicesCustom.unitRequiresPlaceholder')"
                  />
                </el-form-item>
              </el-col>
            </el-row>
          </el-collapse-item>
        </el-collapse>

        <!-- YAML 定义：默认折叠隐藏；展开可查看/手写，保存时若未手改则按表单自动生成。
             服务管理器由系统自动识别，无需选择 -->
        <el-divider content-position="left">
          <el-button link type="primary" @click="toggleYaml">
            {{ showYaml ? t('servicesCustom.collapseYaml') : t('servicesCustom.expandYaml') }}
          </el-button>
        </el-divider>
        <div v-show="showYaml">
          <div class="yaml-tools">
            <span class="yaml-hint">{{ t('servicesCustom.yamlHint') }}</span>
          </div>
          <div class="yaml-editor">
            <CodeEditor :model-value="form.yaml" @update:model-value="onYamlInput" lang="yaml" />
          </div>
        </div>
        <el-alert
          v-if="unitNotes.length"
          type="success"
          :closable="false"
          class="unit-notes"
          :title="t('servicesCustom.unitNotesTitle')"
        >
          <ul class="note-list">
            <li v-for="(n, i) in unitNotes" :key="i">{{ n }}</li>
          </ul>
        </el-alert>
      </el-form>

      <template #footer>
        <div class="footer-actions">
          <div class="footer-left">
            <el-button
              size="small"
              :loading="genBusy"
              :disabled="!form.key"
              @click="unitGenerate"
            >
              {{ t('servicesCustom.unitGenerate') }}
            </el-button>
            <el-button
              size="small"
              type="danger"
              plain
              :loading="genBusy"
              :disabled="!form.key"
              @click="unitRemove"
            >
              {{ t('servicesCustom.unitRemove') }}
            </el-button>
          </div>
          <div class="footer-right">
            <el-button @click="dialog = false">{{ t('common.cancel') }}</el-button>
            <el-button type="primary" :loading="saving" @click="save">{{ t('common.save') }}</el-button>
          </div>
        </div>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { onActivated, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import CodeEditor from '@/components/CodeEditor.vue'
import {
  deleteServiceConfDef,
  generateServiceUnit,
  getServiceConfDef,
  getServiceConfDefs,
  getServiceConfStatus,
  removeServiceUnit,
  writeServiceConfDef,
  type ServiceConfDef,
} from '@/api/servicesConf'

const { t } = useI18n()
const emit = defineEmits<{ (e: 'changed'): void }>()

const defs = ref<ServiceConfDef[]>([])
const loading = ref(false)
const dialog = ref(false)
const editing = ref(false)
const saving = ref(false)
const genBusy = ref(false)
const unitNotes = ref<string[]>([])
const unitError = ref('')
const svcRunning = ref(false)
const showYaml = ref(false)
const yamlDirty = ref(false)
// 编辑时磁盘上的原始 YAML：保存时以它为基底合并，保留表单未覆盖的手写字段（bin_candidates/fields 等）。
const originalYaml = ref('')
const busyKeys = ref<Set<string>>(new Set())

const form = reactive({
  key: '',
  label: '',
  format: 'ini',
  main: '',
  exec_start: '',
  working_dir: '',
  environment: '',
  precmd: '',
  description: '',
  after: '',
  wants: '',
  requires: '',
  user: '',
  group: '',
  restart: '',
  restart_sec: '',
  type: 'simple',
  extra: '',
  start_limit_interval_sec: '',
  start_limit_burst: '',
  yaml: '',
})

function resetForm() {
  form.key = ''
  form.label = ''
  form.format = 'ini'
  form.main = ''
  form.exec_start = ''
  form.working_dir = ''
  form.environment = ''
  form.precmd = ''
  form.description = ''
  form.after = ''
  form.wants = ''
  form.requires = ''
  form.user = ''
  form.group = ''
  form.restart = ''
  form.restart_sec = ''
  form.type = 'simple'
  form.extra = ''
  form.start_limit_interval_sec = ''
  form.start_limit_burst = ''
  form.yaml = ''
  originalYaml.value = ''
  editing.value = false
  unitNotes.value = []
  unitError.value = ''
  svcRunning.value = false
  showYaml.value = false
  yamlDirty.value = false
}

async function loadDefs() {
  loading.value = true
  try {
    const r = await getServiceConfDefs()
    defs.value = r.data.items || []
  } catch {
    defs.value = []
  } finally {
    loading.value = false
  }
}

/** 由表单字段生成 YAML：key/label/format/main + 可选 unit 自启模板（exec_start/workdir/env/precmd） */
function generateYaml() {
  const lines: string[] = []
  const pushStr = (k: string, v: string) => {
    if (v === '') return
    lines.push(`${k}: "${v.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`)
  }
  pushStr('key', form.key)
  pushStr('label', form.label)
  pushStr('format', form.format)
  // 定义版本（默认 1.0.0）；后端忽略未知字段，仅作为本文件版本标记
  lines.push('version: "1.0.0"')
  // 配置文件可多个：输出为 YAML 列表
  const mains = form.main.split('\n').map((x) => x.trim()).filter(Boolean)
  if (mains.length) {
    lines.push('main:')
    for (const m of mains) {
      lines.push(`  - "${m.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`)
    }
  }
  const listOf = (v: string) => v.split('\n').map((x) => x.trim()).filter(Boolean)
  const env = listOf(form.environment)
  const pre = listOf(form.precmd)
  const afters = listOf(form.after)
  const wants = listOf(form.wants)
  const requires = listOf(form.requires)
  const unitLines: string[] = []
  const uPush = (k: string, v: string) => {
    if (!v) return
    unitLines.push(`  ${k}: "${v.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`)
  }
  const uPushList = (k: string, items: string[]) => {
    if (!items.length) return
    unitLines.push(`  ${k}:`)
    for (const it of items) {
      unitLines.push(`    - "${it.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`)
    }
  }
  // [Unit] 段：描述 / 启动依赖 / 弱依赖 / 强依赖
  uPush('description', form.description)
  uPushList('after', afters)
  uPushList('wants', wants)
  uPushList('requires', requires)
  // [Service] 段
  uPush('exec_start', form.exec_start)
  uPush('working_dir', form.working_dir)
  uPush('user', form.user)
  uPush('group', form.group)
  uPushList('environment', env)
  uPushList('precmd', pre)
  uPush('restart', form.restart)
  // Type=（服务类型，缺省 simple）
  uPush('type', form.type)
  // 数字型（后端为整数）：仅在合法非负整数时输出，且不加引号
  const pushNum = (k: string, v: string) => {
    const n = v.trim()
    if (n !== '' && /^\d+$/.test(n)) unitLines.push(`  ${k}: ${Number(n)}`)
  }
  pushNum('restart_sec', form.restart_sec)
  pushNum('start_limit_interval_sec', form.start_limit_interval_sec)
  pushNum('start_limit_burst', form.start_limit_burst)
  // 自定义指令（每行 Key=Value，原样注入 systemd 单元）
  uPushList('extra', listOf(form.extra))
  if (unitLines.length) {
    lines.push('unit:')
    lines.push(...unitLines)
  }
  form.yaml = lines.join('\n') + '\n'
}

/** 折叠/展开 YAML 编辑器；首次展开且未手改时，按当前表单预览生成内容 */
function toggleYaml() {
  showYaml.value = !showYaml.value
  if (showYaml.value && !yamlDirty.value) generateYaml()
}

/** 用户在编辑器里手改 YAML 时标记，保存即以此为准（不再用表单覆盖） */
function onYamlInput(v: string) {
  form.yaml = v
  yamlDirty.value = true
}

/**
 * 解析已保存的 YAML 回填到结构化表单字段（编辑时调用）。
 * 仅做够用的行级解析：支持 key/label/format/main 与 unit 块下的 exec_start/working_dir、
 * environment/precmd 列表；手写的复杂结构无法识别时相关字段留空，用户仍可手动补充。
 */
function parseDef(yaml: string): Record<string, string> {
  const out: Record<string, string> = {}
  const stripQ = (s: string) => s.trim().replace(/^"|"$/g, '').trim()
  const clean = (s: string) => {
    const t = s.trim()
    // 引号值不处理行内注释；裸值去掉 ` # 注释`
    return t.startsWith('"') ? stripQ(t) : stripQ(t.replace(/\s+#.*$/, ''))
  }
  let inUnit = false
  let listKey: string | null = null
  const buf: string[] = []
  const flush = () => {
    if (listKey && buf.length) out[listKey] = buf.join('\n')
    listKey = null
    buf.length = 0
  }
  for (const raw of yaml.split('\n')) {
    const t = raw.trim()
    if (t === '' || t.startsWith('#')) continue
    // 列表项（main / environment / precmd）
    if (t.startsWith('- ') && listKey) {
      buf.push(clean(t.slice(2)))
      continue
    }
    flush()
    const indent = raw.length - raw.trimStart().length
    const m = t.match(/^([\w-]+):\s*(.*)$/)
    if (!m) continue
    const k = m[1]
    const v = clean(m[2])
    if (indent === 0) {
      inUnit = k === 'unit'
      if (inUnit) continue
      if (k === 'key' || k === 'label' || k === 'format') out[k] = v
      else if (k === 'main') {
        if (v) out.main = v
        else listKey = 'main'
      }
    } else if (inUnit) {
      if (
        k === 'exec_start' ||
        k === 'working_dir' ||
        k === 'description' ||
        k === 'user' ||
        k === 'group' ||
        k === 'restart' ||
        k === 'restart_sec' ||
        k === 'start_limit_interval_sec' ||
        k === 'start_limit_burst' ||
        k === 'type'
      ) {
        out[k] = v
      } else if (
        k === 'environment' ||
        k === 'precmd' ||
        k === 'after' ||
        k === 'wants' ||
        k === 'requires' ||
        k === 'extra'
      ) {
        if (v) out[k] = v
        else listKey = k
      }
    }
  }
  flush()
  return out
}

/** 把 YAML 按缩进 `indent` 切成「键块」：每个 `键:` 行开启一块，其后的更深缩进行归属该块。 */
function splitBlocks(lines: string[], indent: number): { key: string; lines: string[] }[] {
  const re = new RegExp(`^ {${indent}}([\\w-]+):`)
  const out: { key: string; lines: string[] }[] = []
  let cur: { key: string; lines: string[] } | null = null
  for (const line of lines) {
    const m = line.match(re)
    if (m) {
      cur = { key: m[1], lines: [line] }
      out.push(cur)
    } else if (cur) {
      cur.lines.push(line)
    }
  }
  return out
}

/** 表单管理的顶层键 */
const MANAGED_TOP = new Set(['key', 'label', 'format', 'version', 'main', 'unit'])
/** 表单管理的 unit 子键 */
const MANAGED_UNIT = new Set([
  'exec_start',
  'working_dir',
  'environment',
  'precmd',
  'description',
  'after',
  'wants',
  'requires',
  'user',
  'group',
  'restart',
  'restart_sec',
  'start_limit_interval_sec',
  'start_limit_burst',
  'type',
  'extra',
])
/** systemd Type 可选项 */
const unitTypeOptions = ['simple', 'exec', 'forking', 'oneshot', 'notify', 'dbus', 'idle']
/** 启动依赖折叠面板的展开项（默认折叠） */
const depsCollapse = ref<string[]>([])
/** 高级选项折叠面板的展开项（默认折叠） */
const advCollapse = ref<string[]>([])
/** 重启策略折叠面板的展开项（默认折叠） */
const restartCollapse = ref<string[]>([])

/**
 * 以 `original` 为基底、用 `generated`（由表单生成）覆盖表单管理的部分：
 * 顶层保留 bin_candidates / fields / exts 等手写键，unit 下保留 user / group / restart 等手写子键。
 */
function mergeYaml(original: string, generated: string): string {
  const origBlocks = splitBlocks(original.split('\n'), 0)
  const genBlocks = splitBlocks(generated.split('\n'), 0)
  const origUnit = origBlocks.find((b) => b.key === 'unit')
  const genKeys = new Set(genBlocks.map((b) => b.key))
  const out: string[] = []
  for (const b of genBlocks) {
    if (b.key === 'unit' && origUnit) {
      // unit 块逐子键合并：表单管理的子键用生成值，其余保留原值
      const genSub = splitBlocks(b.lines.slice(1), 2)
      const origSub = splitBlocks(origUnit.lines.slice(1), 2)
      out.push('unit:')
      for (const s of genSub) out.push(...s.lines)
      for (const s of origSub) if (!MANAGED_UNIT.has(s.key)) out.push(...s.lines)
      continue
    }
    out.push(...b.lines)
  }
  // 原 YAML 有 unit、但表单生成里没有（例如解析没读到 exec_start）→ 保留原 unit，避免误删
  if (!genKeys.has('unit') && origUnit) out.push(...origUnit.lines)
  // 表单不管理的顶层键（bin_candidates / fields / exts …）原样保留
  for (const b of origBlocks) {
    if (!MANAGED_TOP.has(b.key) && !genKeys.has(b.key)) out.push(...b.lines)
  }
  while (out.length && out[out.length - 1].trim() === '') out.pop()
  return out.join('\n') + '\n'
}

/**
 * 把 YAML 编辑器里的「表单未管理」额外参数叠加到 base 上（避免保存时被覆盖）：
 * - 顶层 / unit 下的非托管键（ini_comment、自定义 unit 指令等）一律补入；
 * - 托管键（key/label/format/main 与 unit 子键…）仅当 base 中缺失（即表单没填）时才从编辑器补入，
 *   表单已填则以表单为准。
 */
function overlayExtras(editor: string, base: string): string {
  const edTop = splitBlocks(editor.split('\n'), 0)
  const baseTop = splitBlocks(base.split('\n'), 0)
  const baseTopKeys = new Set(baseTop.map((b) => b.key))
  const baseUnit = baseTop.find((b) => b.key === 'unit')
  const baseSubs = new Map(
    (baseUnit ? splitBlocks(baseUnit.lines.slice(1), 2) : []).map((s) => [s.key, s.lines]),
  )
  const edUnit = edTop.find((b) => b.key === 'unit')
  const mergedSubs = new Map(baseSubs)
  if (edUnit) {
    for (const s of splitBlocks(edUnit.lines.slice(1), 2)) {
      if (!MANAGED_UNIT.has(s.key) || !baseSubs.has(s.key)) mergedSubs.set(s.key, s.lines)
    }
  }
  const out: string[] = []
  for (const b of baseTop) {
    if (b.key === 'unit') continue
    out.push(...b.lines)
  }
  if (mergedSubs.size) {
    out.push('unit:')
    for (const lines of mergedSubs.values()) out.push(...lines)
  }
  for (const b of edTop) {
    if (b.key === 'unit') continue
    if (!MANAGED_TOP.has(b.key) || !baseTopKeys.has(b.key)) out.push(...b.lines)
  }
  while (out.length && out[out.length - 1].trim() === '') out.pop()
  return out.join('\n') + '\n'
}

async function openCreate() {
  resetForm()
  editing.value = false
  dialog.value = true
}

async function openEdit(row: ServiceConfDef) {
  resetForm()
  editing.value = true
  form.key = row.key
  form.label = row.label
  form.format = row.format || 'ini'
  dialog.value = true
  // 取服务运行状态：运行中则「启动命令」等启动参数禁止编辑（需先停止）
  try {
    const s = await getServiceConfStatus(row.key)
    svcRunning.value = s.data.running === true
  } catch {
    svcRunning.value = false
  }
  try {
    const r = await getServiceConfDef(row.key)
    if (r.data.exists && r.data.content) {
      form.yaml = r.data.content
      originalYaml.value = r.data.content
      Object.assign(form, parseDef(r.data.content))
    }
  } catch {
    /* 读不到原文就让用户从表单生成 */
  }
}

async function save() {
  if (!form.key.trim()) {
    ElMessage.warning(t('servicesCustom.keyRequired'))
    return
  }
  // 保存时决定最终 YAML：
  // ① 始终按当前表单生成（表单字段为托管键的权威来源）；
  // ② 编辑既有服务时先与磁盘原 YAML 合并，保留手写字段（bin_candidates/fields 等）；
  // ③ 再把 YAML 编辑器里「表单未管理的额外参数」（ini_comment、自定义 unit 指令等）
  //    叠加进来，避免被覆盖；表单没填的托管键（如仅在 YAML 写的 exec_start）也从编辑器补入。
  const editorContent = form.yaml
  generateYaml()
  let finalYaml = form.yaml
  if (editing.value && originalYaml.value.trim()) {
    finalYaml = mergeYaml(originalYaml.value, finalYaml)
  }
  if (editorContent.trim()) {
    finalYaml = overlayExtras(editorContent, finalYaml)
  }
  form.yaml = finalYaml
  if (!form.yaml.trim()) {
    ElMessage.warning(t('servicesCustom.yamlRequired'))
    return
  }
  saving.value = true
  try {
    const r = await writeServiceConfDef(form.key.trim(), form.yaml)
    if (r.code !== 0) {
      ElMessage.error(r.message || t('servicesCustom.saveFailed'))
      return
    }
    ElMessage.success(t('servicesCustom.saved'))
    dialog.value = false
    await loadDefs()
    emit('changed')
  } catch (e: unknown) {
    const msg = (e as { response?: { data?: { message?: string } } })?.response?.data?.message
    ElMessage.error(msg || t('servicesCustom.saveFailed'))
  } finally {
    saving.value = false
  }
}

/**
 * 列表里的「注册为服务」开关：开 → 生成并启用单元（注册），关 → 移除单元（注销）。
 * 仅在定义带 unit 模板时可用；切换后刷新列表以同步 registered 状态。
 */
async function toggleRegister(row: ServiceConfDef, val: boolean) {
  if (!row.has_unit) return
  busyKeys.value.add(row.key)
  try {
    if (val) {
      const r = await generateServiceUnit(row.key)
      if (r.code !== 0) ElMessage.error(r.message || t('servicesCustom.enableFailed'))
      else ElMessage.success(t('servicesCustom.enabled'))
    } else {
      try {
        await ElMessageBox.confirm(
          t('servicesCustom.disableConfirm'),
          t('servicesCustom.confirmTitle'),
          { type: 'warning' },
        )
      } catch {
        await loadDefs()
        return
      }
      const r = await removeServiceUnit(row.key)
      if (r.code !== 0) ElMessage.error(r.message || t('servicesCustom.disableFailed'))
      else ElMessage.success(t('servicesCustom.disabled'))
    }
    await loadDefs()
  } catch (e: unknown) {
    const msg = (e as { response?: { data?: { message?: string } } })?.response?.data?.message
    ElMessage.error(msg || (val ? t('servicesCustom.enableFailed') : t('servicesCustom.disableFailed')))
  } finally {
    busyKeys.value.delete(row.key)
  }
}

/** 根据已保存定义里的 unit 模板，渲染并启用 systemd 单元文件（写 <key>.service + enable）。 */
async function unitGenerate() {
  if (!form.key.trim()) {
    ElMessage.warning(t('servicesCustom.keyRequired'))
    return
  }
  genBusy.value = true
  unitNotes.value = []
  unitError.value = ''
  try {
    const r = await generateServiceUnit(form.key.trim())
    if (r.code !== 0) {
      unitError.value = r.message || t('servicesCustom.unitGenerateFailed')
      ElMessage.error(unitError.value)
      return
    }
    unitNotes.value = r.data.notes || []
    ElMessage.success(t('servicesCustom.unitGenerateSuccess'))
  } catch (e: unknown) {
    const msg = (e as { response?: { data?: { message?: string } } })?.response?.data?.message
    unitError.value = msg || t('servicesCustom.unitGenerateFailed')
    ElMessage.error(unitError.value)
  } finally {
    genBusy.value = false
  }
}

/** 移除由面板生成的单元文件（disable + 删 <key>.service）。 */
async function unitRemove() {
  if (!form.key.trim()) {
    ElMessage.warning(t('servicesCustom.keyRequired'))
    return
  }
  genBusy.value = true
  unitNotes.value = []
  unitError.value = ''
  try {
    const r = await removeServiceUnit(form.key.trim())
    if (r.code !== 0) {
      unitError.value = r.message || t('servicesCustom.unitRemoveFailed')
      ElMessage.error(unitError.value)
      return
    }
    ElMessage.success(t('servicesCustom.unitRemoveSuccess'))
  } catch (e: unknown) {
    const msg = (e as { response?: { data?: { message?: string } } })?.response?.data?.message
    unitError.value = msg || t('servicesCustom.unitRemoveFailed')
    ElMessage.error(unitError.value)
  } finally {
    genBusy.value = false
  }
}

async function remove(row: ServiceConfDef) {
  try {
    await ElMessageBox.confirm(
      t('servicesCustom.deleteConfirm', { key: row.key }),
      t('common.warning'),
      { type: 'warning' },
    )
  } catch {
    return
  }
  try {
    await deleteServiceConfDef(row.key)
    ElMessage.success(t('servicesCustom.deleted'))
    await loadDefs()
    emit('changed')
  } catch (e: unknown) {
    const msg = (e as { response?: { data?: { message?: string } } })?.response?.data?.message
    ElMessage.error(msg || t('servicesCustom.deleteFailed'))
  }
}

// 本页被 NavPillPanels 的 <KeepAlive> 缓存，onMounted 只会跑一次；改用 onActivated，
// 每次切回「自定义服务」都重新拉取定义并通知父级刷新 tab 列表，避免新增 / 手动放入
// /etc/zap/services 后列表与侧边栏不更新。
onActivated(() => {
  loadDefs()
  emit('changed')
})
</script>

<style scoped>
.custom-mgr {
  min-height: 300px;
}
.head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.h-title {
  font-size: 15px;
  font-weight: 600;
}
.h-desc {
  margin-top: 4px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  line-height: 1.6;
  max-width: 640px;
}
.head-actions {
  display: flex;
  gap: 8px;
}
.mt-3 {
  margin-top: 12px;
}
.mono {
  font-family: 'JetBrains Mono', Menlo, Consolas, monospace;
}
.w-full {
  width: 100%;
}
.field-row {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 8px;
}
.fr-key {
  flex: 0 0 160px;
}
.fr-label {
  flex: 1;
}
.fr-kind {
  flex: 0 0 100px;
}
.fr-opt {
  flex: 0 0 160px;
}
.fr-help {
  flex: 0 0 200px;
}
.yaml-tools {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}
.yaml-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.yaml-editor {
  height: 300px;
  border: 1px solid var(--el-border-color-light);
  border-radius: 6px;
  overflow: hidden;
}
.unit-tip {
  margin-bottom: 12px;
}
.unit-notes {
  margin-top: 12px;
}
.field-hint {
  margin-top: 4px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-line;
}
.field-tip {
  margin-top: 8px;
}
.note-list {
  margin: 0;
  padding-left: 18px;
  font-size: 12px;
  line-height: 1.7;
}
.footer-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  gap: 12px;
  flex-wrap: wrap;
}
.footer-left,
.footer-right {
  display: flex;
  gap: 8px;
  align-items: center;
}
</style>
