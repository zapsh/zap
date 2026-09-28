<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, onActivated, onDeactivated } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'
import { Search } from '@/icons'

interface ProcessItem {
  pid: number
  user: string
  pcpu: string
  pmem: string
  stat: string
  etime: string
  cmd: string
}

const { t } = useI18n()

const processes = ref<ProcessItem[]>([])
const loading = ref(false)
const actingPid = ref<number | null>(null)
const filter = ref('')
let timer: ReturnType<typeof setInterval> | null = null

const filteredProcesses = computed(() => {
  if (!filter.value) return processes.value
  const f = filter.value.toLowerCase()
  return processes.value.filter(
    (p) =>
      p.pid.toString().includes(f) ||
      p.user.toLowerCase().includes(f) ||
      p.cmd.toLowerCase().includes(f),
  )
})

async function loadProcesses() {
  loading.value = true
  try {
    const res = await http.get<{ code: number; data: { processes: ProcessItem[] } }>(
      '/system/config/processes',
    )
    processes.value = res.data?.processes ?? []
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

function statType(stat: string): 'success' | 'danger' | 'warning' | 'info' {
  // S/R 运行中；D 不可中断；Z 僵尸；T 停止；其余为信息态
  if (stat.startsWith('S') || stat.startsWith('R')) return 'success'
  if (stat.startsWith('Z')) return 'danger'
  if (stat.startsWith('T')) return 'info'
  if (stat.startsWith('D')) return 'warning'
  return 'info'
}

async function killProcess(row: ProcessItem, signal: 'TERM' | 'KILL') {
  const label = signal === 'KILL' ? t('serverProcess.forceKill') : t('serverProcess.terminate')
  const tip =
    signal === 'KILL'
      ? t('serverProcess.killConfirm', { pid: row.pid, cmd: row.cmd })
      : t('serverProcess.termConfirm', { pid: row.pid, cmd: row.cmd })
  try {
    await ElMessageBox.confirm(tip, t('common.tip'), {
      type: 'warning',
      confirmButtonText: label,
    })
  } catch {
    return
  }
  actingPid.value = row.pid
  try {
    const res = await http.post<{ code: number; message: string }>(
      '/system/config/processes/kill',
      {
        pid: row.pid,
        signal: signal === 'KILL' ? '9' : undefined,
      },
    )
    ElMessage.success(res.message ?? t('serverProcess.actionSuccess', { label }))
    await loadProcesses()
  } catch {
    /* handled */
  } finally {
    actingPid.value = null
  }
}

function isProtected(pid: number) {
  return pid <= 1
}

function startPolling() {
  if (timer) return
  timer = setInterval(loadProcesses, 5000)
}

function stopPolling() {
  if (!timer) return
  clearInterval(timer)
  timer = null
}

// 进程表在「系统管理」页里是 KeepAlive 的一个面板：切到别的 pill 时停轮询，
// 切回来立刻拉一次（挂载时的那次不重复拉，避免首屏两次请求）
let firstActivate = true

onMounted(() => {
  loadProcesses()
  startPolling()
})
onActivated(() => {
  startPolling()
  if (firstActivate) {
    firstActivate = false
    return
  }
  loadProcesses()
})
onDeactivated(stopPolling)
onBeforeUnmount(stopPolling)
</script>

<template>
  <div class="process-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>{{ t('serverProcess.title') }}</span>
          <div class="header-actions">
            <el-input
              v-model="filter"
              :placeholder="t('serverProcess.searchPlaceholder')"
              clearable
              style="width: 240px"
            >
              <template #prefix>
                <el-icon><Search /></el-icon>
              </template>
            </el-input>
            <el-button type="primary" :loading="loading" @click="loadProcesses">
              {{ t('common.refresh') }}
            </el-button>
          </div>
        </div>
      </template>

      <el-table
        :data="filteredProcesses"
        v-loading="loading"
        stripe
        style="width: 100%"
        :empty-text="t('serverProcess.empty')"
        size="default"
      >
        <el-table-column prop="pid" label="PID" width="90" align="center" />
        <el-table-column
          prop="user"
          :label="t('serverProcess.user')"
          width="100"
          show-overflow-tooltip
        />
        <el-table-column prop="pcpu" label="CPU%" width="90" align="center">
          <template #default="{ row }">
            <span :class="{ 'high-usage': Number(row.pcpu) > 50 }">{{ row.pcpu }}%</span>
          </template>
        </el-table-column>
        <el-table-column
          prop="pmem"
          :label="t('serverProcess.memPercent')"
          width="90"
          align="center"
        >
          <template #default="{ row }">
            <span>{{ row.pmem }}%</span>
          </template>
        </el-table-column>
        <el-table-column prop="stat" :label="t('common.status')" width="100" align="center">
          <template #default="{ row }">
            <el-tag size="small" :type="statType(row.stat)">{{ row.stat }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column
          prop="etime"
          :label="t('serverProcess.uptime')"
          width="110"
          align="center"
        />
        <el-table-column
          prop="cmd"
          :label="t('serverProcess.command')"
          min-width="320"
          show-overflow-tooltip
        />
        <el-table-column :label="t('common.operation')" width="190" align="center" fixed="right">
          <template #default="{ row }">
            <el-button
              size="small"
              type="danger"
              plain
              :loading="actingPid === row.pid"
              :disabled="isProtected(row.pid) || actingPid !== null"
              @click="killProcess(row, 'TERM')"
            >
              {{ t('serverProcess.terminate') }}
            </el-button>
            <el-button
              size="small"
              type="danger"
              :loading="actingPid === row.pid"
              :disabled="isProtected(row.pid) || actingPid !== null"
              @click="killProcess(row, 'KILL')"
            >
              {{ t('serverProcess.forceKill') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>
  </div>
</template>

<style scoped>
.process-container {
  padding: 0;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}
.high-usage {
  color: #f56c6c;
  font-weight: 600;
}
</style>
