<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="security-check">
    <el-card v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('zapCfg.secTitle') }}</span>
          <span class="sub">{{ t('zapCfg.secSubtitle') }}</span>
          <el-button class="header-action" size="small" :loading="scanning" @click="scan">
            {{ t('zapCfg.secRescan') }}
          </el-button>
        </div>
      </template>

      <el-alert
        type="info"
        :closable="false"
        show-icon
        :title="t('zapCfg.secHint')"
        style="margin-bottom: 16px"
      />

      <el-alert
        v-if="checks.length"
        :type="summaryAlertType"
        :closable="false"
        show-icon
        :title="t('zapCfg.secSummary', summary)"
        style="margin-bottom: 16px"
      />

      <div v-for="g in groups" :key="g.cat" class="sec-group">
        <div class="sec-group-title">{{ t(g.key) }}</div>
        <div
          v-for="c in g.items"
          :key="c.id"
          class="sec-item"
          :class="`sec-${c.status}`"
        >
          <el-tag :type="statusType(c.status)" size="small" effect="dark" class="sec-tag">
            {{ statusText(c.status) }}
          </el-tag>
          <div class="sec-body">
            <div class="sec-name">{{ t(c.name_key) }}</div>
            <div class="sec-detail">{{ t(c.detail_key, c.detail_params) }}</div>
            <div v-if="c.suggestion_key" class="sec-suggest">
              <strong>{{ t('zapCfg.secSuggest') }}：</strong
              >{{ t(c.suggestion_key, c.suggestion_params ?? {}) }}
            </div>
          </div>
        </div>
      </div>

      <el-empty
        v-if="!loading && checks.length === 0"
        :description="t('zapCfg.secEmpty')"
      />
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  getZapSecurity,
  type SecurityStatus,
  type ZapSecurityCheck,
} from '@/api/systemZap'

const { t } = useI18n()

const checks = ref<ZapSecurityCheck[]>([])
const loading = ref(false)
const scanning = ref(false)

const summary = computed(() => {
  const s = { total: checks.value.length, pass: 0, warn: 0, fail: 0, info: 0 }
  for (const c of checks.value) s[c.status]++
  return s
})

const summaryAlertType = computed<'success' | 'warning' | 'error'>(() => {
  if (summary.value.fail > 0) return 'error'
  if (summary.value.warn > 0) return 'warning'
  return 'success'
})

/** 按 category 分组（保持出现顺序），每组带分类标题 i18n key。 */
const groups = computed(() => {
  const order: string[] = []
  const map = new Map<string, { key: string; items: ZapSecurityCheck[] }>()
  for (const c of checks.value) {
    if (!map.has(c.category)) {
      map.set(c.category, { key: c.category_key, items: [] })
      order.push(c.category)
    }
    map.get(c.category)!.items.push(c)
  }
  return order.map((cat) => ({ cat, ...map.get(cat)! }))
})

function statusType(s: SecurityStatus) {
  switch (s) {
    case 'pass':
      return 'success'
    case 'warn':
      return 'warning'
    case 'fail':
      return 'danger'
    default:
      return 'info'
  }
}

function statusText(s: SecurityStatus) {
  return t(`zapCfg.secStatus${s.charAt(0).toUpperCase()}${s.slice(1)}`)
}

async function scan() {
  scanning.value = true
  loading.value = true
  try {
    const res = await getZapSecurity()
    checks.value = res.data.checks ?? []
  } catch {
    /* handled */
  } finally {
    scanning.value = false
    loading.value = false
  }
}

onMounted(scan)
</script>

<script lang="ts">
export default { name: 'SecurityCheckPanel' }
</script>

<style scoped>
.security-check {
  padding: 4px;
}
.card-header {
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.card-header .sub {
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.card-header .header-action {
  margin-left: auto;
}
.sec-group {
  margin-bottom: 18px;
}
.sec-group-title {
  font-weight: 600;
  font-size: 14px;
  margin-bottom: 10px;
  color: var(--el-text-color-primary);
}
.sec-item {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  padding: 10px 12px;
  border: 1px solid var(--el-border-color-lighter);
  border-left-width: 3px;
  border-radius: 6px;
  margin-bottom: 8px;
  background: var(--el-fill-color-blank);
}
.sec-pass {
  border-left-color: var(--el-color-success);
}
.sec-warn {
  border-left-color: var(--el-color-warning);
}
.sec-fail {
  border-left-color: var(--el-color-danger);
}
.sec-info {
  border-left-color: var(--el-color-info);
}
.sec-tag {
  flex: 0 0 auto;
  min-width: 44px;
  text-align: center;
}
.sec-body {
  flex: 1 1 auto;
  min-width: 0;
}
.sec-name {
  font-weight: 500;
  margin-bottom: 2px;
}
.sec-detail {
  color: var(--el-text-color-regular);
  font-size: 13px;
  word-break: break-all;
}
.sec-suggest {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
</style>
