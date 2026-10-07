<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<template>
  <div class="firewall-page">
    <el-card v-loading="loading">
      <template #header>
        <div class="card-header">
          <span>{{ t('serverFirewall.title') }}</span>
          <span class="sub">{{ t('serverFirewall.subtitle') }}</span>
          <el-button class="header-action" size="small" :icon="Refresh" @click="load">
            {{ t('common.refresh') }}
          </el-button>
        </div>
      </template>

      <el-alert
        type="warning"
        :closable="false"
        show-icon
        :title="t('serverFirewall.panelPortAlert', { port: status.panel_port || '-' })"
        style="margin-bottom: 16px"
      />

      <el-alert
        v-if="status.wsl"
        type="warning"
        :closable="false"
        show-icon
        :title="t('serverFirewall.wslAlert')"
        style="margin-bottom: 12px"
      />

      <el-descriptions :column="3" border size="small" style="max-width: 760px">
        <el-descriptions-item :label="t('serverFirewall.backend')">
          <el-tag v-if="status.backend === 'none'" size="small" type="info">
            {{ t('serverFirewall.notDetected') }}
          </el-tag>
          <el-tag v-else size="small" type="primary">{{ status.backend }}</el-tag>
          <span v-if="status.detected === 'installed'" class="dim" style="margin-left: 6px">
            {{ t('serverFirewall.installedOnly') }}
          </span>
        </el-descriptions-item>
        <el-descriptions-item :label="t('serverFirewall.runState')">
          <el-tag size="small" :type="status.active ? 'success' : 'info'">
            {{ status.active ? t('serverFirewall.running') : t('serverFirewall.stopped') }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="t('serverFirewall.bootEnabled')">
          <el-tag size="small" :type="status.enabled ? 'success' : 'info'">
            {{ status.enabled ? t('serverFirewall.enabled') : t('serverFirewall.notEnabled') }}
          </el-tag>
        </el-descriptions-item>
      </el-descriptions>

      <div class="actions">
        <el-button
          type="success"
          size="small"
          :loading="acting"
          :disabled="status.backend === 'none' || status.active"
          @click="doToggle('start')"
        >
          {{ t('serverFirewall.start') }}
        </el-button>
        <el-button
          type="warning"
          size="small"
          :loading="acting"
          :disabled="status.backend === 'none' || !status.active"
          @click="confirmToggle('stop')"
        >
          {{ t('serverFirewall.stop') }}
        </el-button>
        <el-button
          size="small"
          :loading="acting"
          :disabled="status.backend === 'none' || status.enabled"
          @click="doToggle('enable')"
        >
          {{ t('serverFirewall.enableBoot') }}
        </el-button>
        <el-button
          size="small"
          :loading="acting"
          :disabled="status.backend === 'none' || !status.enabled"
          @click="doToggle('disable')"
        >
          {{ t('serverFirewall.disableBoot') }}
        </el-button>
      </div>

      <el-divider content-position="left">{{ t('serverFirewall.portRules') }}</el-divider>

      <el-form :inline="true" :model="form" class="rule-form" @submit.prevent>
        <el-form-item :label="t('serverFirewall.port')">
          <el-input-number
            v-model="form.port"
            :min="1"
            :max="65535"
            controls-position="right"
            style="width: 140px"
          />
        </el-form-item>
        <el-form-item :label="t('serverFirewall.protocol')">
          <el-select v-model="form.proto" style="width: 100px">
            <el-option label="TCP" value="tcp" />
            <el-option label="UDP" value="udp" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('serverFirewall.action')">
          <el-select v-model="form.action" style="width: 110px">
            <el-option :label="t('serverFirewall.accept')" value="accept" />
            <el-option :label="t('serverFirewall.drop')" value="drop" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('serverFirewall.source')">
          <el-input
            v-model="form.source"
            :placeholder="t('serverFirewall.sourcePlaceholder')"
            style="width: 190px"
            clearable
          />
        </el-form-item>
        <el-form-item :label="t('common.remark')">
          <el-input
            v-model="form.comment"
            :placeholder="t('serverFirewall.commentPlaceholder')"
            style="width: 170px"
            clearable
          />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" :loading="saving" @click="addRule">
            {{ t('serverFirewall.addRule') }}
          </el-button>
        </el-form-item>
      </el-form>

      <el-table :data="status.rules" size="small" stripe>
        <el-table-column prop="port" :label="t('serverFirewall.port')" width="100" />
        <el-table-column :label="t('serverFirewall.protocol')" width="90">
          <template #default="{ row }">{{ (row.proto || '').toUpperCase() }}</template>
        </el-table-column>
        <el-table-column :label="t('serverFirewall.action')" width="100">
          <template #default="{ row }">
            <el-tag
              size="small"
              :type="row.action === 'accept' ? 'success' : 'danger'"
              effect="plain"
            >
              {{ row.action === 'accept' ? t('serverFirewall.accept') : t('serverFirewall.drop') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('serverFirewall.source')" min-width="160">
          <template #default="{ row }">
            {{ row.source || t('serverFirewall.anySource') }}
          </template>
        </el-table-column>
        <el-table-column
          prop="comment"
          :label="t('common.remark')"
          min-width="140"
          show-overflow-tooltip
        >
          <template #default="{ row }">
            <span class="dim">{{ row.comment || '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="120" fixed="right">
          <template #default="{ row }">
            <el-tooltip
              v-if="row.protected"
              :content="t('serverFirewall.protectedTip')"
              placement="top"
            >
              <el-button link type="danger" disabled>
                {{ t('common.delete') }}
              </el-button>
            </el-tooltip>
            <el-button
              v-else
              link
              type="danger"
              :loading="deletingId === row.id"
              @click="removeRule(row)"
            >
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('serverFirewall.empty')" :image-size="70" />
        </template>
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import {
  addFirewallRule,
  deleteFirewallRule,
  getFirewallStatus,
  toggleFirewall,
  type FirewallRule,
  type FirewallStatus,
} from '@/api/serverFirewall'

const { t } = useI18n()

const loading = ref(false)
const saving = ref(false)
const acting = ref(false)
const deletingId = ref('')

const status = reactive<FirewallStatus>({
  backend: 'none',
  detected: 'none',
  active: false,
  enabled: false,
  wsl: false,
  panel_port: 0,
  rules: [],
})

const form = reactive({
  port: 80,
  proto: 'tcp' as 'tcp' | 'udp',
  action: 'accept' as 'accept' | 'drop',
  source: '',
  comment: '',
})

async function load() {
  loading.value = true
  try {
    const res = await getFirewallStatus()
    const d = res.data || {}
    status.backend = d.backend || 'none'
    status.detected = d.detected || 'none'
    status.wsl = !!d.wsl
    status.active = !!d.active
    status.enabled = !!d.enabled
    status.panel_port = d.panel_port || 0
    status.rules = d.rules || []
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

async function addRule() {
  const port = Number(form.port)
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    ElMessage.warning(t('serverFirewall.portRange'))
    return
  }
  saving.value = true
  try {
    const res = await addFirewallRule({
      port,
      proto: form.proto,
      action: form.action,
      source: form.source.trim(),
      comment: form.comment.trim(),
    })
    ElMessage.success(res.message || t('serverFirewall.ruleAdded'))
    form.comment = ''
    await load()
  } catch {
    /* handled */
  } finally {
    saving.value = false
  }
}

async function removeRule(row: FirewallRule) {
  try {
    await ElMessageBox.confirm(
      t('serverFirewall.deleteConfirm', {
        proto: (row.proto || '').toUpperCase(),
        port: row.port,
        action: row.action === 'accept' ? t('serverFirewall.accept') : t('serverFirewall.drop'),
      }),
      t('serverFirewall.deleteTitle'),
      { type: 'warning', confirmButtonText: t('common.delete') },
    )
  } catch {
    return
  }
  deletingId.value = row.id
  try {
    const res = await deleteFirewallRule(row.id)
    ElMessage.success(res.message || t('serverFirewall.ruleDeleted'))
    await load()
  } catch {
    /* handled */
  } finally {
    deletingId.value = ''
  }
}

async function doToggle(action: 'start' | 'stop' | 'enable' | 'disable') {
  acting.value = true
  try {
    const res = await toggleFirewall(action)
    ElMessage.success(res.message || t('serverFirewall.opDone'))
    await load()
  } catch {
    /* handled */
  } finally {
    acting.value = false
  }
}

async function confirmToggle(action: 'stop' | 'start') {
  if (action !== 'stop') {
    await doToggle(action)
    return
  }
  try {
    await ElMessageBox.confirm(t('serverFirewall.stopConfirm'), t('serverFirewall.dangerTitle'), {
      type: 'warning',
      confirmButtonText: t('serverFirewall.confirmStop'),
    })
  } catch {
    return
  }
  await doToggle('stop')
}

onMounted(load)
</script>

<style scoped>
.firewall-page {
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
.actions {
  margin: 14px 0 4px;
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}
.rule-form {
  margin: 6px 0 12px;
}
.dim {
  color: var(--el-text-color-placeholder);
}
</style>
