<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="app-container">
    <div class="filter-container">
      <el-input
        v-model="listQuery.title"
        :placeholder="t('content.articles.filterTitle')"
        style="width: 200px"
        class="filter-item"
        @keyup.enter="handleFilter"
      />
      <el-select
        v-model="listQuery.status"
        :placeholder="t('content.articles.filterStatus')"
        clearable
        class="filter-item"
        style="width: 130px"
      >
        <el-option
          v-for="item in statusOptions"
          :key="item.value"
          :label="item.label"
          :value="item.value"
        />
      </el-select>
      <el-button class="filter-item" type="primary" :icon="Search" @click="handleFilter">
        {{ t('common.search') }}
      </el-button>
      <el-button class="filter-item" type="primary" :icon="Plus" @click="handleCreate">
        {{ t('common.create') }}
      </el-button>
    </div>

    <el-table
      v-loading="listLoading"
      :data="list"
      border
      fit
      highlight-current-row
      style="width: 100%"
    >
      <el-table-column align="center" label="ID" width="80">
        <template #default="scope">
          <span>{{ scope.row.id }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.articles.colTitle')" min-width="150px">
        <template #default="scope">
          <span>{{ scope.row.title }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.articles.colCategory')" width="110px" align="center">
        <template #default="scope">
          <span>{{ scope.row.category }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.articles.colTags')" width="110px" align="center">
        <template #default="scope">
          <el-tag v-for="tag in scope.row.tags" :key="tag" size="small" class="mx-1">
            {{ tag }}
          </el-tag>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.articles.colStatus')" width="110px" align="center">
        <template #default="scope">
          <el-tag :type="scope.row.status === 1 ? 'success' : 'info'">
            {{
              scope.row.status === 1 ? t('content.articles.published') : t('content.articles.draft')
            }}
          </el-tag>
        </template>
      </el-table-column>

      <el-table-column :label="t('common.createdAt')" width="160px" align="center">
        <template #default="scope">
          <span>{{ scope.row.createTime }}</span>
        </template>
      </el-table-column>

      <el-table-column
        :label="t('common.operation')"
        align="center"
        width="230"
        class-name="small-padding fixed-width"
      >
        <template #default="scope">
          <el-button type="primary" size="small" @click="handleUpdate(scope.row)">
            {{ t('common.edit') }}
          </el-button>
          <el-button
            v-if="scope.row.status !== 1"
            size="small"
            type="success"
            @click="handleModifyStatus(scope.row, 1)"
          >
            {{ t('content.articles.publish') }}
          </el-button>
          <el-button
            v-if="scope.row.status === 1"
            size="small"
            @click="handleModifyStatus(scope.row, 0)"
          >
            {{ t('content.articles.unpublish') }}
          </el-button>
          <el-button size="small" type="danger" @click="handleDelete(scope.row)">
            {{ t('common.delete') }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>

    <el-pagination
      v-show="total > 0"
      v-model:current-page="listQuery.page"
      v-model:page-size="listQuery.limit"
      :total="total"
      :page-sizes="[10, 20, 30, 50]"
      layout="total, sizes, prev, pager, next, jumper"
      @size-change="handleSizeChange"
      @current-change="handleCurrentChange"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Search, Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { Article } from '@/types/articles'

const { t } = useI18n()

// 状态选项
const statusOptions = computed(() => [
  { label: t('content.articles.published'), value: 1 },
  { label: t('content.articles.draft'), value: 0 },
])

// 列表数据
const list = ref<Article[]>([])
const total = ref(0)
const listLoading = ref(true)
const listQuery = reactive({
  page: 1,
  limit: 10,
  title: '',
  status: undefined,
})

// 获取列表数据
const getList = () => {
  listLoading.value = true
  // TODO: 调用API获取数据
  // 模拟数据
  setTimeout(() => {
    list.value = [
      {
        id: 1,
        title: 'Sample Article',
        category: 'Tech',
        tags: ['Vue', 'TypeScript'],
        status: 1,
        createTime: '2024-01-20 12:00:00',
      },
    ]
    total.value = 1
    listLoading.value = false
  }, 500)
}

// 搜索
const handleFilter = () => {
  listQuery.page = 1
  getList()
}

// 新增
const handleCreate = () => {
  // TODO: 跳转到新增页面或打开新增对话框
  ElMessage.info(t('content.articles.createWip'))
}

// 编辑
const handleUpdate = (row: any) => {
  // TODO: 跳转到编辑页面或打开编辑对话框
  ElMessage.info(t('content.articles.editWip'))
}

// 修改状态
const handleModifyStatus = (row: any, status: number) => {
  const action = status === 1 ? t('content.articles.publish') : t('content.articles.unpublish')
  ElMessageBox.confirm(t('content.articles.statusConfirm', { action }), t('common.tip'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning',
  })
    .then(async () => {
      // TODO: 调用API修改状态
      ElMessage.success(t('content.articles.statusDone', { action }))
      getList()
    })
    .catch(() => {
      ElMessage.info(t('content.articles.cancelStatus'))
    })
}

// 删除
const handleDelete = (row: any) => {
  ElMessageBox.confirm(t('content.articles.deleteConfirm'), t('common.tip'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning',
  })
    .then(async () => {
      // TODO: 调用API删除数据
      ElMessage.success(t('common.deleteSuccess'))
      getList()
    })
    .catch(() => {
      ElMessage.info(t('content.cancelDelete'))
    })
}

// 分页
const handleSizeChange = (val: number) => {
  listQuery.limit = val
  getList()
}

const handleCurrentChange = (val: number) => {
  listQuery.page = val
  getList()
}

// 初始化
onMounted(() => {
  getList()
})
</script>

<style scoped>
.filter-container {
  padding-bottom: 10px;
}

.filter-item {
  margin-right: 10px;
}

.el-tag + .el-tag {
  margin-left: 4px;
}
</style>
