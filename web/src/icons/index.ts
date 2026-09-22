/**
 * 统一图标出口（Material Symbols）。
 *
 * 设计约定：
 * 1. 面板可能部署在纯内网环境，图标必须是构建期打进产物的本地资源，不能依赖任何
 *    在线图标接口。`~icons/material-symbols/*` 由 unplugin-icons 在构建期把
 *    @iconify-json/material-symbols（已随 npm 依赖离线安装）编译成 Vue 组件，
 *    按需引入 —— 只有真正用到的图标会进入产物。
 * 2. 视图里统一 `import { Search, Plus } from '@/icons'`，具名导出就是图标组件本身，
 *    名字仅用于阅读，不包含任何图标集兼容逻辑。
 * 3. 路由 meta / 数据库 menus.icon 里保存的是 Material Symbols 图标名字符串，
 *    由 resolveIcon() 解析；无法识别的名字一律回退默认图标，避免空白图标或渲染报错。
 */
import { defineComponent, h, type Component, type PropType } from 'vue'

/* ── Material Symbols 图标组件（构建期按需引入，离线可用） ─────────── */
import IconAccountCircle from '~icons/material-symbols/account-circle'
import IconAdd from '~icons/material-symbols/add'
import IconAlarm from '~icons/material-symbols/alarm'
import IconAnalytics from '~icons/material-symbols/analytics'
import IconAutoFixHigh from '~icons/material-symbols/auto-fix-high'
import IconBadge from '~icons/material-symbols/badge'
import IconBuild from '~icons/material-symbols/build'
import IconCable from '~icons/material-symbols/cable'
import IconCancel from '~icons/material-symbols/cancel'
import IconCheck from '~icons/material-symbols/check'
import IconCheckCircle from '~icons/material-symbols/check-circle'
import IconChevronRight from '~icons/material-symbols/chevron-right'
import IconClose from '~icons/material-symbols/close'
import IconCloud from '~icons/material-symbols/cloud'
import IconCloudDone from '~icons/material-symbols/cloud-done'
import IconCloudDownload from '~icons/material-symbols/cloud-download'
import IconCloudOff from '~icons/material-symbols/cloud-off'
import IconCloudUpload from '~icons/material-symbols/cloud-upload'
import IconConfirmationNumber from '~icons/material-symbols/confirmation-number'
import IconCreateNewFolder from '~icons/material-symbols/create-new-folder'
import IconDarkMode from '~icons/material-symbols/dark-mode'
import IconDatabase from '~icons/material-symbols/database'
import IconDelete from '~icons/material-symbols/delete'
import IconDeployedCode from '~icons/material-symbols/deployed-code'
import IconDescription from '~icons/material-symbols/description'
import IconDns from '~icons/material-symbols/dns'
import IconEdit from '~icons/material-symbols/edit'
import IconFolder from '~icons/material-symbols/folder'
import IconFolderOpen from '~icons/material-symbols/folder-open'
import IconGridView from '~icons/material-symbols/grid-view'
import IconHardDrive from '~icons/material-symbols/hard-drive'
import IconHome from '~icons/material-symbols/home'
import IconInfo from '~icons/material-symbols/info'
import IconKey from '~icons/material-symbols/key'
import IconKeyboardArrowDown from '~icons/material-symbols/keyboard-arrow-down'
import IconLightMode from '~icons/material-symbols/light-mode'
import IconLink from '~icons/material-symbols/link'
import IconLock from '~icons/material-symbols/lock'
import IconLogout from '~icons/material-symbols/logout'
import IconMemory from '~icons/material-symbols/memory'
import IconMenu from '~icons/material-symbols/menu'
import IconMenuBook from '~icons/material-symbols/menu-book'
import IconMonitor from '~icons/material-symbols/monitor'
import IconMonitorHeart from '~icons/material-symbols/monitor-heart'
import IconMonitoring from '~icons/material-symbols/monitoring'
import IconMoreVert from '~icons/material-symbols/more-vert'
import IconContentCopy from '~icons/material-symbols/content-copy'
import IconContentCut from '~icons/material-symbols/content-cut'
import IconDownload from '~icons/material-symbols/download'
import IconDriveFileMove from '~icons/material-symbols/drive-file-move'
import IconInventory2 from '~icons/material-symbols/inventory-2'
import IconNoteAdd from '~icons/material-symbols/note-add'
import IconNotifications from '~icons/material-symbols/notifications'
import IconOpenInNew from '~icons/material-symbols/open-in-new'
import IconPayments from '~icons/material-symbols/payments'
import IconPerson from '~icons/material-symbols/person'
import IconProgressActivity from '~icons/material-symbols/progress-activity'
import IconPublic from '~icons/material-symbols/public'
import IconRefresh from '~icons/material-symbols/refresh'
import IconSchedule from '~icons/material-symbols/schedule'
import IconSearch from '~icons/material-symbols/search'
import IconSettings from '~icons/material-symbols/settings'
import IconSettingsApplications from '~icons/material-symbols/settings-applications'
import IconShoppingCart from '~icons/material-symbols/shopping-cart'
import IconSort from '~icons/material-symbols/sort'
import IconSpeed from '~icons/material-symbols/speed'
import IconStorefront from '~icons/material-symbols/storefront'
import IconTimer from '~icons/material-symbols/timer'
import IconTranslate from '~icons/material-symbols/translate'
import IconTune from '~icons/material-symbols/tune'
import IconUpload from '~icons/material-symbols/upload'
import IconViewList from '~icons/material-symbols/view-list'
import IconVisibility from '~icons/material-symbols/visibility'
import IconWarning from '~icons/material-symbols/warning'
import IconHistory from '~icons/material-symbols/history'
import IconBook from '~icons/material-symbols/book'
import IconHelp from '~icons/material-symbols/help'
import IconHelpOutline from '~icons/material-symbols/help-outline'
import IconUpgrade from '~icons/material-symbols/upgrade'
import IconArrowBack from '~icons/material-symbols/arrow-back'
import IconPlayArrow from '~icons/material-symbols/play-arrow'
import IconElectricBoltOutline from '~icons/material-symbols/electric-bolt-outline' 
import IconShieldWatchOutline from '~icons/material-symbols/shield-watch-outline'
import IconRocketLaunch from '~icons/material-symbols/rocket-launch'

/** 路由 / 数据库菜单里图标名的集合前缀 */
export const ICON_PREFIX = 'material-symbols:'

/** Material Symbols 图标名 → 组件（字符串图标解析表，也是菜单图标可选项来源） */
export const ICON_MAP: Record<string, Component> = {
  'account-circle': IconAccountCircle,
  add: IconAdd,
  alarm: IconAlarm,
  analytics: IconAnalytics,
  archive: IconInventory2,
  'auto-fix-high': IconAutoFixHigh,
  badge: IconBadge,
  build: IconBuild,
  cable: IconCable,
  cancel: IconCancel,
  check: IconCheck,
  'check-circle': IconCheckCircle,
  'chevron-right': IconChevronRight,
  close: IconClose,
  cloud: IconCloud,
  'cloud-done': IconCloudDone,
  'cloud-download': IconCloudDownload,
  'cloud-off': IconCloudOff,
  'cloud-upload': IconCloudUpload,
  'confirmation-number': IconConfirmationNumber,
  content_copy: IconContentCopy,
  content_cut: IconContentCut,
  'create-new-folder': IconCreateNewFolder,
  'dark-mode': IconDarkMode,
  database: IconDatabase,
  delete: IconDelete,
  'deployed-code': IconDeployedCode,
  description: IconDescription,
  dns: IconDns,
  download: IconDownload,
  'drive-file-move': IconDriveFileMove,
  edit: IconEdit,
  folder: IconFolder,
  'folder-open': IconFolderOpen,
  'grid-view': IconGridView,
  'hard-drive': IconHardDrive,
  home: IconHome,
  inventory_2: IconInventory2,
  'open-in-new': IconOpenInNew,
  info: IconInfo,
  key: IconKey,
  'keyboard-arrow-down': IconKeyboardArrowDown,
  'light-mode': IconLightMode,
  link: IconLink,
  lock: IconLock,
  logout: IconLogout,
  memory: IconMemory,
  menu: IconMenu,
  'menu-book': IconMenuBook,
  monitor: IconMonitor,
  'monitor-heart': IconMonitorHeart,
  monitoring: IconMonitoring,
  'more-vert': IconMoreVert,
  'note-add': IconNoteAdd,
  notifications: IconNotifications,
  payments: IconPayments,
  person: IconPerson,
  'progress-activity': IconProgressActivity,
  public: IconPublic,
  refresh: IconRefresh,
  'rocket-launch': IconRocketLaunch,
  schedule: IconSchedule,
  search: IconSearch,
  settings: IconSettings,
  'settings-applications': IconSettingsApplications,
  'shield-watch-outline': IconShieldWatchOutline,
  'shopping-cart': IconShoppingCart,
  sort: IconSort,
  speed: IconSpeed,
  storefront: IconStorefront,
  timer: IconTimer,
  translate: IconTranslate,
  tune: IconTune,
  upload: IconUpload,
  'view-list': IconViewList,
  visibility: IconVisibility,
  warning: IconWarning,
  history: IconHistory,
  book: IconBook,
  help: IconHelp,
  'help-outline': IconHelpOutline,
  upgrade: IconUpgrade,
  'arrow-back': IconArrowBack,
}

/** 菜单图标可选项（按字母序，供后台菜单管理下拉选择） */
export const ICON_NAMES: string[] = Object.keys(ICON_MAP).sort()

/** 图标缺失时的兜底图标 */
export const DEFAULT_ICON: Component = IconMenu

/* ── 具名导出：沿用改造前的 Element Plus 图标名，视图标签无需改动 ───── */
export const Archive = IconInventory2
export const ArrowDown = IconKeyboardArrowDown
export const ArrowRight = IconChevronRight
export const Bell = IconNotifications
export const Box = IconDeployedCode
export const Check = IconCheck
export const CircleCheckFilled = IconCheckCircle
export const CircleCloseFilled = IconCancel
export const Close = IconClose
export const Cloud = IconCloud
export const CloudDone = IconCloudDone
export const CloudDownload = IconCloudDownload
export const CloudOff = IconCloudOff
export const CloudUpload = IconCloudUpload
export const Connection = IconCable
export const Copy = IconContentCopy
export const Cpu = IconMemory
export const Cut = IconContentCut
export const DataLine = IconMonitoring
export const Delete = IconDelete
export const Document = IconDescription
export const Dns = IconDns
export const DocumentAdd = IconNoteAdd
export const Download = IconDownload
export const Edit = IconEdit
export const Fold = IconMenu
export const Folder = IconFolder
export const FolderAdd = IconCreateNewFolder
export const FolderOpened = IconFolderOpen
export const Goods = IconStorefront
export const Grid = IconGridView
export const HardDrive = IconHardDrive
export const Home = IconHome
export const InfoFilled = IconInfo
export const Key = IconKey
export const Link = IconLink
export const List = IconViewList
export const Loading = IconProgressActivity
export const Lock = IconLock
export const MagicStick = IconAutoFixHigh
export const Monitor = IconMonitor
export const Moon = IconDarkMode
export const MoreFilled = IconMoreVert
export const Move = IconDriveFileMove
export const Odometer = IconSpeed
export const Open = IconOpenInNew
export const Plus = IconAdd
export const Refresh = IconRefresh
export const Search = IconSearch
export const Setting = IconSettings
export const Sunny = IconLightMode
export const SwitchButton = IconLogout
export const Timer = IconTimer
export const Translate = IconTranslate
export const Upload = IconUpload
export const User = IconPerson
export const UserFilled = IconAccountCircle
export const Warning = IconWarning
export const WarningFilled = IconWarning
// 「文档」菜单新增图标
export const History = IconHistory
export const Book = IconBook
export const Help = IconHelp
export const HelpOutline = IconHelpOutline
export const Upgrade = IconUpgrade
export const ArrowBack = IconArrowBack


// Logo Icon
export const ElectricBoltOutline = IconElectricBoltOutline

// Zap Pro 菜单（侧栏「Zap Pro」目录）用
export const ShieldWatchOutline = IconShieldWatchOutline

/** 「构建镜像」与「镜像列表行内启动」两个按钮用：Material Symbols 的“扳手”与“播放” */
export const Play = IconPlayArrow
export const Build = IconBuild

/**
 * 解析图标名 → 组件。
 * 只接受 Material Symbols 名，两种写法等价：`material-symbols:search` 或 `search`；
 * 空值 / 无法识别的名字回退默认图标。
 */
export function resolveIcon(icon?: string | null): Component {
  const raw = (icon || '').trim()
  if (!raw) return DEFAULT_ICON
  const name = raw.toLowerCase().startsWith(ICON_PREFIX) ? raw.slice(ICON_PREFIX.length) : raw
  return ICON_MAP[name] ?? DEFAULT_ICON
}

/**
 * 通用图标组件：`<Icon icon="material-symbols:search" />`。
 * 直接传入组件（`:icon="SomeIcon"`）也可，便于调用方自行决定图标来源。
 */
export const Icon = defineComponent({
  name: 'ZapIcon',
  props: {
    icon: {
      type: [String, Object, Function] as PropType<string | Component>,
      default: '',
    },
  },
  setup(props) {
    return () =>
      typeof props.icon === 'string' ? h(resolveIcon(props.icon)) : h(props.icon || DEFAULT_ICON)
  },
})
