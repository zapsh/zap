import { computed, ref } from 'vue'

/** 可选的时间范围（`live` = 实时，5 分钟窗口 + 高频轮询；其余走后端降采样） */
export const MONITOR_RANGES = [
  { value: 'live', hours: 0 },
  { value: '1h', hours: 1 },
  { value: '6h', hours: 6 },
  { value: '24h', hours: 24 },
  { value: '7d', hours: 24 * 7 },
  { value: '30d', hours: 24 * 30 },
] as const

export type MonitorRangeValue = (typeof MONITOR_RANGES)[number]['value']

/**
 * 服务器监控的时间范围状态机（各监控图表页共用）。
 *
 * - `live`：按 `liveSecs` 轮询实时接口（数据每几秒都在变，自动刷新最直观）
 * - 历史范围：一次加载降采样数据后**暂停轮询** —— 历史曲线不会自己变，
 *   反复整窗重取又浪费；用户想看最新点，点「刷新」或切回实时即可
 *
 * `epoch`：每次发起取数递增。fetcher 拿到它后，响应回来时若 epoch 已变
 * （用户又切了范围），**丢弃这次响应** —— 否则慢的旧请求会把新范围的数据
 * 覆盖掉，表现就是「切换后数据跳来跳去」。
 */
export function useMonitorRange(
  fetcher: (range: MonitorRangeValue, epoch: number) => Promise<void>,
  liveSecs = 5,
) {
  const range = ref<MonitorRangeValue>('live')
  const refreshing = ref(false)
  const epoch = ref(0)
  let timer: ReturnType<typeof setInterval> | undefined

  const isLive = computed(() => range.value === 'live')

  async function run() {
    const my = ++epoch.value
    refreshing.value = true
    try {
      await fetcher(range.value, my)
    } finally {
      if (my === epoch.value) {
        refreshing.value = false
      }
    }
  }

  function stopPolling() {
    if (timer) {
      clearInterval(timer)
      timer = undefined
    }
  }

  function startPolling() {
    stopPolling()
    if (!isLive.value) return
    timer = setInterval(run, liveSecs * 1000)
  }

  async function setRange(v: MonitorRangeValue) {
    if (v === range.value) return
    range.value = v
    stopPolling()
    await run()
    startPolling()
  }

  return { range, isLive, refreshing, epoch, run, startPolling, stopPolling, setRange }
}

/** 时间轴标签粒度随范围变粗：实时带秒，天内只到分钟，跨天带上日期 */
export function monitorTimeLabel(ts: number, range: string): string {
  const d = new Date(ts * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  if (range === '7d' || range === '30d') {
    return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
  }
  if (range === 'live') {
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  }
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}
