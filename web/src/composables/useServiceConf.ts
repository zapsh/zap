// 服务配置（/system/service-conf/*）的取数与写入逻辑。
//
// 只管数据不管界面：MySQL 页、PHP 实例面板各自画自己的 UI，避免出现
// 「一套共享页面套所有服务」—— 服务页要各自长成自己需要的样子。
import { computed, reactive, ref, toValue, type MaybeRefOrGetter } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  controlServiceConf,
  getServiceConfKeys,
  getServiceConfList,
  getServiceConfRead,
  getServiceConfStatus,
  saveServiceConf,
  saveServiceConfKeys,
  type ServiceConfFile,
  type ServiceConfKeysData,
  type ServiceConfListData,
  type ServiceConfStatus,
} from '@/api/servicesConf.ts'

export type ServiceAction = 'start' | 'stop' | 'restart' | 'reload'

/**
 * @param service 服务 key（yaml 里注册的，如 mysql / php74）
 * @param label   服务显示名，仅用于启停确认文案
 */
export function useServiceConf(service: MaybeRefOrGetter<string>, label?: MaybeRefOrGetter<string>) {
  const { t } = useI18n()

  const status = ref<ServiceConfStatus>({})
  const listData = ref<ServiceConfListData>({ installed: false, files: [] })
  const keysData = ref<ServiceConfKeysData>({ installed: false, fields: [], values: {} })

  const busy = ref(false)
  const acting = ref('')

  const running = computed(() => !!status.value.running)

  /** 版本号：各服务输出的头行都不一样，压成 `8.0.36` / `24.0.7` 这种短串 */
  const version = computed(() => {
    const v = status.value.version || ''
    if (!v) return ''
    const m = v.match(/(?:PHP\s+)?(\d+\.\d+(?:\.\d+)?)/)
    return m ? m[1] : v
  })

  /** MySQL / MariaDB 由后端按登记与版本识别，这里只负责显示名 */
  const engineName = computed(() =>
    status.value.engine === 'mariadb'
      ? 'MariaDB'
      : status.value.engine === 'mysql'
        ? 'MySQL'
        : '',
  )

  // ── 关键项表单 ────────────────────────────────
  const visual = reactive<Record<string, string>>({})
  const savingKeys = ref(false)

  function resetVisual(values: Record<string, string | number | boolean | null>) {
    for (const k of Object.keys(visual)) delete visual[k]
    for (const [k, v] of Object.entries(values ?? {})) {
      if (v === null || v === undefined) visual[k] = ''
      else if (typeof v === 'boolean') visual[k] = v ? 'true' : 'false'
      else visual[k] = String(v)
    }
  }

  async function loadKeys() {
    try {
      const res = await getServiceConfKeys(toValue(service))
      keysData.value = res.data
      resetVisual(res.data.values ?? {})
    } catch {
      /* interceptor 已提示 */
    }
  }

  async function saveKeys() {
    const payload: Record<string, string> = {}
    for (const f of keysData.value.fields) {
      payload[f.key] = (visual[f.key] || '').trim()
    }
    savingKeys.value = true
    try {
      const res = await saveServiceConfKeys(toValue(service), payload)
      ElMessage.success(res.data?.reason || t('servicesCommon.keysSaved'))
      await loadKeys()
      await loadStatus()
    } catch {
      /* interceptor 已提示 */
    } finally {
      savingKeys.value = false
    }
  }

  // ── 配置文件编辑 ──────────────────────────────
  const confFiles = ref<ServiceConfFile[]>([])
  const activeFile = ref('')
  const editorContent = ref<string | null>(null)
  const originalContent = ref('')
  const fileLoading = ref(false)
  const savingFile = ref(false)
  /** 选中的文件磁盘上还不存在（探测失败或尚未创建），保存即新建 */
  const fileMissing = ref(false)

  const dirty = computed(
    () => editorContent.value !== null && editorContent.value !== originalContent.value,
  )
  const editorLang = computed(() => {
    const name = activeFile.value.toLowerCase()
    if (name.endsWith('.json')) return 'json'
    if (name.endsWith('.ini') || name.endsWith('.cnf') || name.endsWith('.conf')) return 'ini'
    return 'text'
  })

  async function selectFile(path: string, force = false) {
    if (dirty.value && !force) {
      try {
        await ElMessageBox.confirm(t('servicesCommon.unsavedSwitch'), t('common.tip'), {
          type: 'warning',
        })
      } catch {
        return
      }
    }
    if (!path) return
    activeFile.value = path
    fileLoading.value = true
    try {
      const res = await getServiceConfRead(toValue(service), path)
      editorContent.value = res.data.content
      originalContent.value = res.data.content
      fileMissing.value = !!res.data.missing
    } catch {
      editorContent.value = null
      originalContent.value = ''
    } finally {
      fileLoading.value = false
    }
  }

  async function reloadFile() {
    if (dirty.value) {
      try {
        await ElMessageBox.confirm(t('servicesCommon.reloadConfirm'), t('common.tip'), {
          type: 'warning',
        })
      } catch {
        return
      }
    }
    await selectFile(activeFile.value, true)
  }

  async function saveFile() {
    if (editorContent.value === null || !activeFile.value) return
    savingFile.value = true
    try {
      const res = await saveServiceConf(toValue(service), activeFile.value, editorContent.value)
      ElMessage.success(res.data?.reason || t('servicesCommon.confSaved'))
      originalContent.value = editorContent.value
      await loadList()
    } catch {
      /* interceptor 已提示 */
    } finally {
      savingFile.value = false
    }
  }

  async function loadList() {
    try {
      const res = await getServiceConfList(toValue(service))
      listData.value = res.data
      confFiles.value = res.data.files
      const current = activeFile.value
      if (!current || !res.data.files.some((f) => f.path === current)) {
        const main = res.data.files.find((f) => f.is_main) || res.data.files[0]
        if (main) await selectFile(main.path, true)
      }
    } catch {
      /* interceptor 已提示 */
    }
  }

  // ── 状态与启停 ────────────────────────────────
  async function loadStatus() {
    busy.value = true
    try {
      const res = await getServiceConfStatus(toValue(service))
      status.value = res.data
    } catch {
      /* interceptor 已提示 */
    } finally {
      busy.value = false
    }
  }

  async function doControl(action: ServiceAction) {
    const warn = action === 'restart' || action === 'stop'
    try {
      await ElMessageBox.confirm(
        t('servicesCommon.controlConfirm', {
          label: label ? toValue(label) : '',
          action: t(`servicesCommon.${action}`),
          warn: warn ? t('servicesCommon.restartWarn') : '',
        }),
        t('common.tip'),
        { type: warn ? 'warning' : 'info' },
      )
    } catch {
      return
    }
    acting.value = action
    try {
      const res = await controlServiceConf(toValue(service), action)
      ElMessage.success(res.message || t('servicesCommon.opSuccess'))
      // systemctl 返回后服务未必立刻就绪，等一下再刷新状态
      await new Promise((r) => setTimeout(r, 500))
      await loadStatus()
    } catch {
      /* interceptor 已提示 */
    } finally {
      acting.value = ''
    }
  }

  async function loadAll() {
    await loadStatus()
    if (!status.value.installed) return
    await Promise.all([loadList(), loadKeys()])
  }

  return {
    status,
    listData,
    keysData,
    confFiles,
    busy,
    acting,
    running,
    version,
    engineName,
    visual,
    savingKeys,
    loadKeys,
    saveKeys,
    activeFile,
    editorContent,
    fileLoading,
    savingFile,
    fileMissing,
    dirty,
    editorLang,
    selectFile,
    reloadFile,
    saveFile,
    loadStatus,
    loadList,
    loadAll,
    doControl,
  }
}
