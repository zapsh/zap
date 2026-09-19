import { computed, ref, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useUserStore } from '@/stores/user'
import { getRunFiles, retryRun } from '@/api/appstore'
import type AppStoreLogDrawer from '@/components/AppStoreLogDrawer.vue'

/**
 * 应用商店任务的「编辑快照 / 重跑」（仅管理员、仅失败任务）。
 *
 * 任务队列的多个入口（应用商店自己的队列抽屉、系统管理的全局任务页）都要这两个
 * 操作，这里收口：快照探测、确认文案、重跑后打开新日志，避免各写一遍导致行为漂移。
 *
 * @param drawerRef 日志抽屉实例；编辑与重跑都复用它（编辑直接在抽屉里开编辑器）
 * @param onChanged 重跑成功后回调（刷新列表 / 角标）
 */
export function useAppstoreRunOps(
  drawerRef: Ref<InstanceType<typeof AppStoreLogDrawer> | null>,
  onChanged?: () => void,
) {
  const { t } = useI18n()
  const userStore = useUserStore()
  const isAdmin = computed(() => !!userStore.roles?.includes('admin'))
  /** 正在重跑的任务号：用于按钮 loading，也防止连点提交两次 */
  const retryId = ref('')

  function canOperate(row: { status: string }) {
    return isAdmin.value && row.status === 'failed'
  }

  function openEditor(row: { task_id: string; title?: string; pkg?: string; action?: string }) {
    drawerRef.value?.openEditorFor(row.task_id, row.title || row.pkg || row.action)
  }

  async function retry(row: { task_id: string; title?: string; pkg?: string; action?: string }) {
    if (retryId.value) return
    try {
      // 重跑靠的是运行快照；没有快照的任务（如纯 Docker 构建）重跑无意义，先探明
      const files = await getRunFiles(row.task_id)
      if (!(files.data?.files || []).length) {
        ElMessage.warning(t('task.noSnapshot'))
        return
      }
      await ElMessageBox.confirm(t('task.retryConfirm'), t('task.ops.retry'), { type: 'warning' })
    } catch (e: any) {
      if (e !== 'cancel' && e?.message) ElMessage.warning(e.message)
      return
    }
    retryId.value = row.task_id
    try {
      const resp = await retryRun(row.task_id)
      // 重跑安装/升级同样要编译：前面有编译在跑时它只是入队，还没有日志可看
      if (resp.data?.queued) {
        ElMessage.success(t('task.retryQueued', { n: resp.data.position ?? 1 }))
      } else {
        ElMessage.success(t('task.retryStarted'))
        drawerRef.value?.openDrawer(
          resp.data?.run_id,
          `${row.title || row.pkg || row.action}${t('task.retrySuffix')}`,
        )
      }
      onChanged?.()
    } catch (e: any) {
      if (e !== 'cancel') ElMessage.error(e?.message || t('task.retryFailed'))
    } finally {
      retryId.value = ''
    }
  }

  return { isAdmin, retryId, canOperate, openEditor, retry }
}
