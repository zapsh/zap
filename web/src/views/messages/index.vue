<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="app-container messages-page">
    <el-card shadow="never">
      <template #header>
        <div class="messages-head">
          <span class="messages-head__title">{{ t('messages.title') }}</span>
          <el-button
            v-if="unreadCount > 0"
            type="primary"
            plain
            size="small"
            :loading="readAllLoading"
            @click="onReadAll"
          >
            {{ t('messages.readAll') }}
          </el-button>
        </div>
      </template>

      <div v-loading="loading">
        <el-empty v-if="!loading && list.length === 0" :description="t('messages.empty')" />

        <div
          v-for="m in list"
          :key="m.id"
          class="msg-item"
          :class="{ unread: !m.is_read }"
          @click="openMessage(m)"
        >
          <span v-if="!m.is_read" class="msg-item__dot"></span>
          <div class="msg-item__main">
            <div class="msg-item__row">
              <span class="msg-item__title">{{ m.title }}</span>
              <span class="msg-item__time">{{ fmtTime(m.created_at) }}</span>
            </div>
            <div class="msg-item__body">{{ m.body }}</div>
          </div>
          <div class="msg-item__ops" @click.stop>
            <el-button v-if="!m.is_read" link type="primary" size="small" @click="onRead(m)">
              {{ t('messages.markRead') }}
            </el-button>
            <el-button link type="danger" size="small" @click="onDelete(m.id)">
              {{ t('common.delete') }}
            </el-button>
          </div>
        </div>

        <el-pagination
          v-if="total > pageSize"
          class="messages-pager"
          background
          layout="prev, pager, next, total"
          :total="total"
          :page-size="pageSize"
          :current-page="page"
          @current-change="load((page = $event))"
        />
      </div>
    </el-card>

    <el-dialog
      v-model="detailVisible"
      :title="active?.title ?? t('messages.detailTitle')"
      width="560px"
    >
      <div class="msg-detail">
        <div class="msg-detail__meta">
          {{ t('messages.sentAt', { time: active ? fmtTime(active.created_at) : '' }) }}
        </div>
        <div class="msg-detail__body">{{ active?.body }}</div>
      </div>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { deleteNotices, getNotices, readAllNotices, readNotice } from '@/api/notice'
import type { NoticeMessage } from '@/api/notice'

const { t } = useI18n()

const loading = ref(false)
const readAllLoading = ref(false)
const list = ref<NoticeMessage[]>([])
const total = ref(0)
const unreadCount = ref(0)
const page = ref(1)
const pageSize = 10

const detailVisible = ref(false)
const active = ref<NoticeMessage | null>(null)

function fmtTime(ts: number) {
  return dayjs(ts * 1000).format('YYYY-MM-DD HH:mm')
}

async function load(p = 1) {
  loading.value = true
  try {
    const res = await getNotices({ page: p, page_size: pageSize })
    const data = res.data
    list.value = data.list ?? []
    total.value = data.total ?? 0
    unreadCount.value = data.unread_count ?? 0
    page.value = p
  } catch {
    // 拦截器已弹窗
  } finally {
    loading.value = false
  }
}

/** 打开消息：未读先标记已读，再展示全文 */
async function openMessage(m: NoticeMessage) {
  if (!m.is_read) {
    await onRead(m, true)
  }
  active.value = m
  detailVisible.value = true
}

async function onRead(m: NoticeMessage, silent = false) {
  try {
    await readNotice(m.id)
    m.is_read = 1
    unreadCount.value = Math.max(0, unreadCount.value - 1)
    if (!silent) ElMessage.success(t('messages.readOk'))
  } catch {
    // 拦截器已弹窗
  }
}

async function onReadAll() {
  readAllLoading.value = true
  try {
    await readAllNotices()
    list.value.forEach((m) => (m.is_read = 1))
    unreadCount.value = 0
    ElMessage.success(t('messages.readAllOk'))
  } catch {
    // 拦截器已弹窗
  } finally {
    readAllLoading.value = false
  }
}

async function onDelete(id: number) {
  try {
    await ElMessageBox.confirm(t('messages.deleteConfirm'), t('messages.deleteTitle'), {
      confirmButtonText: t('common.delete'),
      cancelButtonText: t('common.cancel'),
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await deleteNotices([id])
    ElMessage.success(t('messages.deleteOk'))
    await load(page.value)
  } catch {
    // 拦截器已弹窗
  }
}

onMounted(() => {
  load(1)
})
</script>

<style scoped>
.messages-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.messages-head__title {
  font-size: 15px;
  font-weight: 600;
}

.msg-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 14px 4px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  cursor: pointer;
  transition: background 0.2s;
}

.msg-item:hover {
  background: var(--el-bg-color-page);
}

.msg-item__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #f56c6c;
  flex-shrink: 0;
  margin-top: 7px;
}

.msg-item__main {
  flex: 1;
  min-width: 0;
}

.msg-item__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
}

.msg-item__title {
  font-weight: 600;
  color: var(--el-text-color-primary);
}

.msg-item.unread .msg-item__title {
  color: var(--el-color-primary);
}

.msg-item__time {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  flex-shrink: 0;
}

.msg-item__body {
  margin-top: 6px;
  font-size: 13px;
  color: var(--el-text-color-regular);
  line-height: 1.7;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.msg-item__ops {
  flex-shrink: 0;
  opacity: 0;
  transition: opacity 0.2s;
  display: flex;
  align-items: center;
}

.msg-item:hover .msg-item__ops {
  opacity: 1;
}

.messages-pager {
  margin-top: 16px;
  justify-content: flex-end;
}
</style>

<style>
.msg-detail__meta {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 12px;
}

.msg-detail__body {
  font-size: 14px;
  color: var(--el-text-color-primary);
  line-height: 1.8;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
