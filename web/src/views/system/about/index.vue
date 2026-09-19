<script setup lang="ts">
/**
 * 「系统设置 → About ZAP」。
 *
 * 原侧栏「文档」一级菜单（更新日志 / 用户手册 / FAQ / 升级指南）已整合到这里：
 * 版本信息 + 文档入口同页展示，侧栏只留一个入口。
 * 文档正文仍在 `/docs/<id>`（constantRoutes，hidden），故四张卡片照旧跳路由。
 */
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { History, Book, Help, Upgrade, ArrowRight } from '@/icons'
import { getSystemAbout } from '@/api/dashboard'

const { t } = useI18n()
const router = useRouter()

const loading = ref(true)
const about = ref<Record<string, any>>({})

interface DocCard {
  /** URL 末段，与后端白名单 DOCS 的 id 对齐 */
  id: string
  /** i18n key（不带 namespace 前缀） */
  i18nKey: string
  icon: any
}

const cards: DocCard[] = [
  { id: 'changelog', i18nKey: 'docs.changelog', icon: History },
  { id: 'manual', i18nKey: 'docs.manual', icon: Book },
  { id: 'faq', i18nKey: 'docs.faq', icon: Help },
  { id: 'upgrade', i18nKey: 'docs.upgrade', icon: Upgrade },
]

/** 构建信息缺失时 vergen 会返回 unknown，统一不展示内部占位串 */
function meta(value: any) {
  return !value || value === 'unknown' ? '' : String(value)
}

const buildDateText = computed(() => {
  const date = meta(about.value.build_date)
  if (!date) return '-'
  const ts = meta(about.value.build_timestamp)
  const time = ts.length >= 19 ? ts.slice(11, 19) : ''
  return time ? `${date} ${time}` : date
})

const commitText = computed(() => {
  const sha = meta(about.value.git_sha)
  if (!sha) return '-'
  const short = sha.slice(0, 7)
  return about.value.git_dirty === 'true' ? `${short} (dirty)` : short
})

const rustText = computed(() => {
  const version = meta(about.value.rustc_version)
  if (!version) return '-'
  const channel = meta(about.value.rustc_channel)
  return channel ? `${version} (${channel})` : version
})

const targetText = computed(() => {
  const triple = meta(about.value.target_triple)
  if (!triple) return '-'
  const profile = meta(about.value.profile)
  return profile ? `${triple} / ${profile}` : triple
})

function open(id: string) {
  router.push(`/docs/${id}`)
}

onMounted(async () => {
  try {
    const resp = await getSystemAbout()
    if (resp.code === 0) about.value = resp.data || {}
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <div class="system-about">
    <el-row :gutter="16">
      <!-- 版本信息 -->
      <el-col :xs="24" :md="10" :lg="10">
        <el-card shadow="never" class="about-card" v-loading="loading">
          <template #header>
            <div class="card-header">
              <span>{{ t('dashboardAdmin.aboutZap') }}</span>
            </div>
          </template>
          <el-descriptions :column="1" size="small" border>
            <el-descriptions-item :label="t('dashboardAdmin.version')">
              {{ about.version || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.buildDate')">
              {{ buildDateText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.commit')">
              {{ commitText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.branch')">
              {{ about.git_branch || '-' }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.rust')">
              {{ rustText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.target')">
              {{ targetText }}
            </el-descriptions-item>
            <el-descriptions-item :label="t('dashboardAdmin.license')">
              {{ about.license || '-' }}
            </el-descriptions-item>
          </el-descriptions>
          <div class="about-actions">
            <el-link type="primary" underline="never" target="_blank" href="https://github.com/zapsh/zap">
              Github
            </el-link>
          </div>
        </el-card>
      </el-col>

      <!-- 文档入口 -->
      <el-col :xs="24" :md="14" :lg="14">
        <el-card shadow="never" class="about-card">
          <template #header>
            <div class="card-header">
              <span>{{ t('menu.docs') }}</span>
            </div>
          </template>
          <p class="docs-intro">{{ t('docs.intro') }}</p>
          <el-row :gutter="12">
            <el-col v-for="c in cards" :key="c.id" :xs="24" :sm="12">
              <el-card shadow="hover" class="docs-card" @click="open(c.id)">
                <div class="docs-card__inner">
                  <el-icon class="docs-card__icon"><component :is="c.icon" /></el-icon>
                  <span class="docs-card__title">{{ t(c.i18nKey) }}</span>
                  <el-icon class="docs-card__arrow"><ArrowRight /></el-icon>
                </div>
              </el-card>
            </el-col>
          </el-row>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<style scoped>
.system-about {
  padding: 16px;
  max-width: 1200px;
  margin: 0 auto;
}
.about-card {
  margin-bottom: 16px;
  border-radius: 8px;
}
.card-header {
  font-weight: 600;
}
.about-actions {
  display: flex;
  gap: 16px;
  margin-top: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--el-border-color-lighter);
}
.docs-intro {
  margin: 0 0 16px;
  color: var(--el-text-color-regular);
  line-height: 1.7;
}
.docs-card {
  margin-bottom: 12px;
  cursor: pointer;
  border-radius: 8px;
  transition: transform 0.15s ease;
}
.docs-card:hover {
  transform: translateY(-2px);
}
.docs-card__inner {
  display: flex;
  align-items: center;
  gap: 14px;
}
.docs-card__icon {
  font-size: 24px;
  color: var(--el-color-primary);
}
.docs-card__title {
  flex: 1;
  font-size: 15px;
  font-weight: 500;
}
.docs-card__arrow {
  font-size: 18px;
  color: var(--el-text-color-secondary);
}
</style>
