<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="app-container">
    <div class="filter-container">
      <el-input
        v-model="listQuery.name"
        :placeholder="t('content.categories.filterName')"
        style="width: 200px"
        class="filter-item"
        @keyup.enter="handleFilter"
      />
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
      row-key="id"
      style="width: 100%"
    >
      <el-table-column label="ID" prop="id" align="center" width="80">
        <template #default="scope">
          <span>{{ scope.row.id }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.categories.colName')" min-width="150px">
        <template #default="scope">
          <span>{{ scope.row.name }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.categories.colSort')" width="100" align="center">
        <template #default="scope">
          <span>{{ scope.row.sort }}</span>
        </template>
      </el-table-column>

      <el-table-column :label="t('content.categories.colCount')" width="100" align="center">
        <template #default="scope">
          <el-tag type="info">{{ scope.row.articleCount }}</el-tag>
        </template>
      </el-table-column>

      <el-table-column :label="t('common.createdAt')" width="160" align="center">
        <template #default="scope">
          <span>{{ scope.row.createTime }}</span>
        </template>
      </el-table-column>

      <el-table-column
        :label="t('common.operation')"
        align="center"
        width="180"
        class-name="small-padding fixed-width"
      >
        <template #default="scope">
          <el-button type="primary" size="small" @click="handleUpdate(scope.row)">
            {{ t('common.edit') }}
          </el-button>
          <el-button type="danger" size="small" @click="handleDelete(scope.row)">
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

    <!-- 新增/编辑对话框 -->
    <el-dialog
      :title="
        dialogStatus === 'create'
          ? t('content.categories.dialogCreate')
          : t('content.categories.dialogEdit')
      "
      v-model="dialogFormVisible"
      width="500px"
    >
      <el-form
        ref="dataFormRef"
        :model="temp"
        :rules="rules"
        label-position="left"
        label-width="80px"
        style="margin-left: 50px; margin-right: 50px"
        @submit.prevent
      >
        <el-form-item :label="t('content.categories.formName')" prop="name">
          <el-input v-model="temp.name" />
        </el-form-item>
        <el-form-item :label="t('content.categories.formSort')" prop="sort">
          <el-input-number v-model="temp.sort" :min="0" />
        </el-form-item>
      </el-form>
      <template #footer>
        <div class="dialog-footer">
          <el-button @click="dialogFormVisible = false">{{ t('common.cancel') }}</el-button>
          <el-button
            type="primary"
            @click="dialogStatus === 'create' ? createData() : updateData()"
          >
            {{ t('common.confirm') }}
          </el-button>
        </div>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, nextTick, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Search, Plus } from '@/icons'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { FormInstance } from 'element-plus'
import type { Category } from '@/types/categories'

const { t } = useI18n()

// 列表数据
const list = ref<Category[]>([])
const total = ref(0)
const listLoading = ref(true)
const listQuery = reactive({
  page: 1,
  limit: 10,
  name: '',
})

// 表单数据
const dialogFormVisible = ref(false)
const dialogStatus = ref('')
const dataFormRef = ref<FormInstance>()
const temp = reactive({
  id: undefined,
  name: '',
  sort: 0,
})

const rules = computed(() => ({
  name: [{ required: true, message: t('content.categories.nameRule'), trigger: 'blur' }],
  sort: [{ required: true, message: t('content.categories.sortRule'), trigger: 'blur' }],
}))

// 获取列表数据
const getList = () => {
  listLoading.value = true
  // TODO: 调用API获取数据
  // 模拟数据
  setTimeout(() => {
    list.value = [
      {
        id: 1,
        name: 'Tech',
        sort: 1,
        articleCount: 10,
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

// 重置表单
const resetTemp = () => {
  temp.id = undefined
  temp.name = ''
  temp.sort = 0
}

// 新增
const handleCreate = () => {
  resetTemp()
  dialogStatus.value = 'create'
  dialogFormVisible.value = true
  nextTick(() => {
    dataFormRef.value?.clearValidate()
  })
}

// 提交新增
const createData = () => {
  dataFormRef.value?.validate((valid) => {
    if (valid) {
      // TODO: 调用API创建数据
      ElMessage.success(t('common.createSuccess'))
      dialogFormVisible.value = false
      getList()
    }
  })
}

// 编辑
const handleUpdate = (row: any) => {
  temp.id = row.id
  temp.name = row.name
  temp.sort = row.sort
  dialogStatus.value = 'update'
  dialogFormVisible.value = true
  nextTick(() => {
    dataFormRef.value?.clearValidate()
  })
}

// 提交编辑
const updateData = () => {
  dataFormRef.value?.validate((valid) => {
    if (valid) {
      // TODO: 调用API更新数据
      ElMessage.success(t('common.updateSuccess'))
      dialogFormVisible.value = false
      getList()
    }
  })
}

// 删除
const handleDelete = (row: any) => {
  ElMessageBox.confirm(t('content.categories.deleteConfirm'), t('common.tip'), {
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

.dialog-footer {
  text-align: right;
  padding-top: 20px;
}
</style>
