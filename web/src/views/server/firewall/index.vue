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

      <el-tabs v-model="tab" style="margin-top: 8px">
        <!-- ── 端口规则 ─────────────────────────────────── -->
        <el-tab-pane :label="t('serverFirewall.portRules')" name="ports">
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
                  {{
                    row.action === 'accept' ? t('serverFirewall.accept') : t('serverFirewall.drop')
                  }}
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
                <el-tag
                  v-if="lockRemarkOf(row)"
                  size="small"
                  type="warning"
                  effect="plain"
                >
                  {{ lockRemarkOf(row) }}
                </el-tag>
                <span v-else class="dim">{{ row.comment || '-' }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="160" fixed="right">
              <template #default="{ row }">
                <el-button link type="primary" @click="openEdit(row)">
                  {{ t('common.edit') }}
                </el-button>
                <el-tooltip
                  v-if="row.port === status.panel_port"
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

          <el-dialog v-model="editVisible" :title="t('serverFirewall.editTitle')" width="460px">
            <el-form :model="editForm" label-width="80px" @submit.prevent>
              <el-form-item :label="t('serverFirewall.port')">
                <el-input-number v-model="editForm.port" :min="1" :max="65535" controls-position="right" style="width: 150px" />
              </el-form-item>
              <el-form-item :label="t('serverFirewall.protocol')">
                <el-select v-model="editForm.proto" style="width: 110px">
                  <el-option label="TCP" value="tcp" />
                  <el-option label="UDP" value="udp" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('serverFirewall.action')">
                <el-select v-model="editForm.action" style="width: 110px">
                  <el-option :label="t('serverFirewall.accept')" value="accept" />
                  <el-option :label="t('serverFirewall.drop')" value="drop" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('serverFirewall.source')">
                <el-input
                  v-model="editForm.source"
                  :placeholder="t('serverFirewall.sourcePlaceholder')"
                  style="width: 220px"
                  clearable
                />
              </el-form-item>
              <el-form-item :label="t('common.remark')">
                <el-input
                  v-model="editForm.comment"
                  :placeholder="t('serverFirewall.commentPlaceholder')"
                  style="width: 220px"
                  clearable
                />
              </el-form-item>
            </el-form>
            <template #footer>
              <el-button @click="editVisible = false">{{ t('common.cancel') }}</el-button>
              <el-button type="primary" :loading="editing" @click="saveEdit">{{ t('common.confirm') }}</el-button>
            </template>
          </el-dialog>
        </el-tab-pane>

        <!-- ── 端口转发 ─────────────────────────────────── -->
        <el-tab-pane :label="t('serverFirewall.forwardTab')" name="forwards">
          <el-alert
            v-if="status.forward_supported && !status.ip_forward && status.active"
            type="info"
            :closable="false"
            show-icon
            :title="t('serverFirewall.ipForwardAlert')"
            style="margin-bottom: 12px"
          />
          <el-form :inline="true" :model="forwardForm" class="rule-form" @submit.prevent>
            <el-form-item :label="t('serverFirewall.protocol')">
              <el-select v-model="forwardForm.proto" style="width: 100px">
                <el-option label="TCP" value="tcp" />
                <el-option label="UDP" value="udp" />
              </el-select>
            </el-form-item>
            <el-form-item :label="t('serverFirewall.listenPort')">
              <el-input-number
                v-model="forwardForm.listen_port"
                :min="1"
                :max="65535"
                controls-position="right"
                style="width: 140px"
              />
            </el-form-item>
            <el-form-item :label="t('serverFirewall.target')">
              <el-input
                v-model="forwardForm.target_addr"
                :placeholder="t('serverFirewall.targetAddrPlaceholder')"
                style="width: 180px"
                clearable
              />
              <span class="to">:</span>
              <el-input-number
                v-model="forwardForm.target_port"
                :min="1"
                :max="65535"
                controls-position="right"
                style="width: 130px"
              />
            </el-form-item>
            <el-form-item>
              <el-button
                type="primary"
                :loading="savingForward"
                :disabled="!status.forward_supported"
                @click="addForward"
              >
                {{ t('serverFirewall.addForward') }}
              </el-button>
            </el-form-item>
          </el-form>

          <el-alert
            v-if="!status.forward_supported && status.backend !== 'none'"
            type="warning"
            :closable="false"
            show-icon
            :title="
              t('serverFirewall.forwardUnsupported', { backend: status.backend })
            "
            style="margin-bottom: 12px"
          />

          <el-table :data="status.forwards" size="small" stripe>
            <el-table-column :label="t('serverFirewall.listenPort')" width="160">
              <template #default="{ row }">
                {{ (row.proto || '').toUpperCase() }}/{{ row.listen_port }}
              </template>
            </el-table-column>
            <el-table-column :label="t('serverFirewall.target')" min-width="180">
              <template #default="{ row }">→ {{ row.target_addr }}:{{ row.target_port }}</template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="120" fixed="right">
              <template #default="{ row }">
                <el-button
                  link
                  type="danger"
                  :loading="deletingId === `fw:${row.id}`"
                  @click="removeForward(row)"
                >
                  {{ t('common.delete') }}
                </el-button>
              </template>
            </el-table-column>
            <template #empty>
              <el-empty :description="t('serverFirewall.forwardEmpty')" :image-size="70" />
            </template>
          </el-table>
        </el-tab-pane>

        <!-- ── IP 黑名单 ────────────────────────────────── -->
        <el-tab-pane :label="t('serverFirewall.blacklistTab')" name="blacklist">
          <div class="tab-desc">{{ t('serverFirewall.blacklistDesc') }}</div>
          <el-form :inline="true" :model="blackForm" class="rule-form" @submit.prevent>
            <el-form-item :label="t('serverFirewall.ipLabel')">
              <el-input
                v-model="blackForm.ip"
                :placeholder="t('serverFirewall.ipPlaceholder')"
                style="width: 240px"
                clearable
              />
            </el-form-item>
            <el-form-item>
              <el-button type="danger" :loading="savingBlack" @click="addBlack">
                {{ t('serverFirewall.addBlack') }}
              </el-button>
            </el-form-item>
          </el-form>

          <el-table :data="status.blacklist" size="small" stripe>
            <el-table-column prop="ip" :label="t('serverFirewall.ipLabel')" min-width="200" />
            <el-table-column :label="t('common.operation')" width="120" fixed="right">
              <template #default="{ row }">
                <el-button
                  link
                  type="danger"
                  :loading="deletingId === `bl:${row.id}`"
                  @click="removeBlack(row)"
                >
                  {{ t('common.delete') }}
                </el-button>
              </template>
            </el-table-column>
            <template #empty>
              <el-empty :description="t('serverFirewall.blacklistEmpty')" :image-size="70" />
            </template>
          </el-table>
        </el-tab-pane>

        <!-- ── IP 白名单 ────────────────────────────────── -->
        <el-tab-pane :label="t('serverFirewall.whitelistTab')" name="whitelist">
          <div class="tab-desc">{{ t('serverFirewall.whitelistDesc') }}</div>
          <el-form :inline="true" :model="whiteForm" class="rule-form" @submit.prevent>
            <el-form-item :label="t('serverFirewall.ipLabel')">
              <el-input
                v-model="whiteForm.ip"
                :placeholder="t('serverFirewall.ipPlaceholder')"
                style="width: 240px"
                clearable
              />
            </el-form-item>
            <el-form-item :label="t('common.remark')">
              <el-input
                v-model="whiteForm.comment"
                :placeholder="t('serverFirewall.commentPlaceholder')"
                style="width: 160px"
                clearable
              />
            </el-form-item>
            <el-form-item>
              <el-button type="success" :loading="savingWhite" @click="addWhite">
                {{ t('serverFirewall.addWhite') }}
              </el-button>
            </el-form-item>
          </el-form>

          <el-alert type="info" :closable="false" show-icon :title="t('serverFirewall.whiteTip')" style="margin-bottom: 12px" />

          <el-table :data="whitelist" size="small" stripe>
            <el-table-column prop="ip" :label="t('serverFirewall.ipLabel')" min-width="200" />
            <el-table-column :label="t('common.remark')" min-width="140">
              <template #default="{ row }">
                <span class="dim">{{ row.comment || '-' }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('serverFirewall.whiteRuleCol')" width="110">
              <template #default="{ row }">
                <el-tag size="small" :type="row.inRules ? 'success' : 'info'" effect="plain">
                  {{ row.inRules ? t('serverFirewall.whiteRuleOk') : t('serverFirewall.whiteRuleMissing') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="120" fixed="right">
              <template #default="{ row }">
                <el-button
                  link
                  type="danger"
                  :loading="deletingId === `wl:${row.ip}`"
                  @click="removeWhite(row)"
                >
                  {{ t('common.delete') }}
                </el-button>
              </template>
            </el-table-column>
            <template #empty>
              <el-empty :description="t('serverFirewall.whitelistEmpty')" :image-size="70" />
            </template>
          </el-table>
        </el-tab-pane>

        <!-- ── SSH 爆破防护（fail2ban） ─────────────────── -->
        <el-tab-pane :label="t('serverFirewall.banTab')" name="ban">
          <div class="tab-desc">{{ t('serverFirewall.banDesc') }}</div>

          <el-alert
            v-if="!ban.settings.enabled"
            type="info"
            :closable="false"
            show-icon
            :title="t('serverFirewall.banOffTip')"
            style="margin-bottom: 12px"
          />
          <el-alert
            v-else-if="ban.log_file"
            type="success"
            :closable="false"
            show-icon
            :title="t('serverFirewall.banOnTip', { log: ban.log_file, n: ban.bans.length })"
            style="margin-bottom: 12px"
          />
          <el-alert
            v-if="ban.settings.enabled && !ban.log_file"
            type="warning"
            :closable="false"
            show-icon
            :title="t('serverFirewall.banNoLogTip')"
            style="margin-bottom: 12px"
          />
          <el-alert
            v-if="ban.ignored.length"
            type="warning"
            :closable="false"
            show-icon
            style="margin-bottom: 12px"
          >
            <template #title>
              <span>{{ t('serverFirewall.banIgnoredTip', { n: ban.ignored.length }) }}</span>
              <el-button link type="primary" size="small" @click="ban.settings.ignore_private = false; saveBan()">
                {{ t('serverFirewall.banDisableIgnorePrivate') }}
              </el-button>
            </template>
          </el-alert>

          <el-form :model="ban.settings" label-width="180px" class="ban-form">
            <el-form-item :label="t('serverFirewall.banEnable')">
              <el-switch v-model="ban.settings.enabled" />
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banMaxRetry')">
              <el-input-number v-model="ban.settings.max_retry" :min="1" :max="100" controls-position="right" />
              <span class="hint">{{ t('serverFirewall.banMaxRetryTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banFindTime')">
              <el-input-number v-model="ban.settings.find_time" :min="30" :max="86400" controls-position="right" />
              <span class="hint">{{ t('serverFirewall.seconds') }}</span>
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banTime')">
              <el-input-number v-model="ban.settings.ban_time" :min="60" :max="604800" controls-position="right" />
              <span class="hint">{{ t('serverFirewall.banTimeTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banInterval')">
              <el-input-number v-model="ban.settings.interval" :min="10" :max="3600" controls-position="right" />
              <span class="hint">{{ t('serverFirewall.seconds') }}</span>
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banIgnorePrivate')">
              <el-switch v-model="ban.settings.ignore_private" />
              <span class="hint">{{ t('serverFirewall.banIgnorePrivateTip') }}</span>
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banIgnoredIps')">
              <el-input
                v-model="banIgnoredText"
                :placeholder="t('serverFirewall.banIgnoredIpsTip')"
                style="width: 380px"
                clearable
              />
            </el-form-item>
            <el-form-item :label="t('serverFirewall.banLogFile')">
              <el-input
                v-model="ban.settings.log_file"
                :placeholder="t('serverFirewall.banLogFileTip')"
                style="width: 380px"
                clearable
              />
              <span class="hint">{{ t('serverFirewall.banLogFileHint') }}</span>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="savingBan" @click="saveBan">
                {{ t('serverFirewall.banSave') }}
              </el-button>
              <el-button :loading="banLoading" @click="loadBan">{{ t('common.refresh') }}</el-button>
              <span v-if="ban.last_error" class="ban-err">{{ ban.last_error }}</span>
            </el-form-item>
          </el-form>

          <el-divider content-position="left">{{ t('serverFirewall.banListTitle') }}</el-divider>
          <el-table :data="ban.bans" size="small" stripe>
            <el-table-column prop="ip" :label="t('serverFirewall.ipLabel')" min-width="170" />
            <el-table-column :label="t('serverFirewall.banReason')" min-width="200">
              <template #default="{ row }">{{ row.reason }}</template>
            </el-table-column>
            <el-table-column :label="t('serverFirewall.banHits')" width="90">
              <template #default="{ row }">{{ row.hits }}</template>
            </el-table-column>
            <el-table-column :label="t('serverFirewall.banRemaining')" width="120">
              <template #default="{ row }">{{ fmtRemaining(row.remaining) }}</template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="110" fixed="right">
              <template #default="{ row }">
                <el-button
                  link
                  type="primary"
                  :loading="unbanning === row.ip"
                  @click="doUnban(row.ip)"
                >
                  {{ t('serverFirewall.banUnban') }}
                </el-button>
              </template>
            </el-table-column>
            <template #empty>
              <el-empty :description="t('serverFirewall.banEmpty')" :image-size="70" />
            </template>
          </el-table>

          <template v-if="ban.watching.length">
            <el-divider content-position="left">{{ t('serverFirewall.banWatchTitle') }}</el-divider>
            <el-table :data="ban.watching" size="small" stripe>
              <el-table-column prop="ip" :label="t('serverFirewall.ipLabel')" min-width="170" />
              <el-table-column :label="t('serverFirewall.banHits')" width="90">
                <template #default="{ row }">
                  <el-tag size="small" :type="row.hits >= ban.settings.max_retry ? 'danger' : 'info'" effect="plain">
                    {{ row.hits }} / {{ ban.settings.max_retry }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column :label="t('serverFirewall.banLastSeen')" min-width="160">
                <template #default="{ row }">{{ fmtTime(row.last) }}</template>
              </el-table-column>
            </el-table>
          </template>

          <template v-if="ban.ignored.length">
            <el-divider content-position="left">{{ t('serverFirewall.banIgnoredTitle') }}</el-divider>
            <el-table :data="ban.ignored" size="small" stripe>
              <el-table-column prop="ip" :label="t('serverFirewall.ipLabel')" min-width="170" />
              <el-table-column :label="t('serverFirewall.banHits')" width="90">
                <template #default="{ row }">{{ row.hits }}</template>
              </el-table-column>
              <el-table-column :label="t('serverFirewall.banSkipReason')" min-width="220">
                <template #default="{ row }">
                  <el-tag size="small" type="warning" effect="plain">{{ row.reason }}</el-tag>
                </template>
              </el-table-column>
            </el-table>
          </template>
        </el-tab-pane>

        <!-- ── 导入导出 ─────────────────────────────────── -->
        <el-tab-pane :label="t('serverFirewall.backupTab')" name="backup">
          <div class="backup-pane">
            <div class="backup-block">
              <div class="backup-title">{{ t('serverFirewall.exportBtn') }}</div>
              <div class="backup-tip">{{ t('serverFirewall.exportTip') }}</div>
              <el-button type="primary" :loading="exporting" @click="doExport">
                {{ t('serverFirewall.exportBtn') }}
              </el-button>
            </div>
            <el-divider />
            <div class="backup-block">
              <div class="backup-title">{{ t('serverFirewall.importBtn') }}</div>
              <div class="backup-tip">{{ t('serverFirewall.importTip') }}</div>
              <el-button :loading="importing" @click="fileInput?.click()">
                {{ t('serverFirewall.importSelectFile') }}
              </el-button>
              <input
                ref="fileInput"
                type="file"
                accept=".json,application/json"
                style="display: none"
                @change="onImportFile"
              />
            </div>
          </div>
        </el-tab-pane>

        <!-- ── 安全加固（锁服） ─────────────────────────── -->
        <el-tab-pane :label="t('serverFirewall.hardenTab')" name="harden">
          <el-alert
            v-if="status.locked_down"
            type="success"
            :closable="false"
            show-icon
            :title="t('serverFirewall.lockedDown')"
            style="margin-bottom: 12px"
          />
          <el-alert
            v-else
            type="info"
            :closable="false"
            show-icon
            :title="t('serverFirewall.notLockedDown')"
            style="margin-bottom: 12px"
          />
          <div class="harden-block">
            <div class="backup-title">{{ t('serverFirewall.hardenTitle') }}</div>
            <div class="backup-tip">{{ t('serverFirewall.hardenDesc') }}</div>
            <div class="allow-list">
              <el-tag
                v-for="p in allowPorts"
                :key="p"
                size="small"
                type="success"
                effect="plain"
                style="margin: 0 6px 6px 0"
              >
                {{ p }}
              </el-tag>
            </div>
            <div style="margin-top: 12px">
              <el-button
                v-if="!status.locked_down"
                type="danger"
                :loading="lockdowning"
                @click="doLockdown(false)"
              >
                {{ t('serverFirewall.lockBtn') }}
              </el-button>
              <el-button
                v-else
                type="primary"
                :loading="lockdowning"
                @click="doLockdown(true)"
              >
                {{ t('serverFirewall.unlockBtn') }}
              </el-button>
            </div>
          </div>
        </el-tab-pane>
      </el-tabs>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@/icons'
import {
  addFirewallBlacklist,
  addFirewallForward,
  addFirewallRule,
  addFirewallWhitelist,
  deleteFirewallBlacklist,
  deleteFirewallForward,
  deleteFirewallRule,
  deleteFirewallWhitelist,
  updateFirewallRule,
  exportFirewall,
  getBanStatus,
  getFirewallStatus,
  getFirewallWhitelist,
  importFirewall,
  lockdownFirewall,
  saveBanSettings,
  toggleFirewall,
  unbanIp,
  type BanRecord,
  type BanSettings,
  type FirewallBlackEntry,
  type FirewallExport,
  type FirewallForward,
  type FirewallRule,
  type FirewallStatus,
} from '@/api/serverFirewall'

const { t } = useI18n()

const loading = ref(false)
const saving = ref(false)
const acting = ref(false)
const savingForward = ref(false)
const savingBlack = ref(false)
const deletingId = ref('')
const exporting = ref(false)
const importing = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)
const lockdowning = ref(false)

const editVisible = ref(false)
const editing = ref(false)
const editForm = reactive({
  id: '',
  port: 80,
  proto: 'tcp' as 'tcp' | 'udp',
  action: 'accept' as 'accept' | 'drop',
  source: '',
  comment: '',
})

function openEdit(row: FirewallRule) {
  editForm.id = row.id
  editForm.port = row.port
  editForm.proto = (row.proto || 'tcp') as 'tcp' | 'udp'
  editForm.action = (row.action || 'accept') as 'accept' | 'drop'
  editForm.source = row.source || ''
  editForm.comment = row.comment || ''
  editVisible.value = true
}

async function saveEdit() {
  const port = Number(editForm.port)
  if (!port || port < 1 || port > 65535) {
    ElMessage.warning(t('serverFirewall.portRange'))
    return
  }
  editing.value = true
  try {
    const res = await updateFirewallRule({
      id: editForm.id,
      port,
      proto: editForm.proto,
      action: editForm.action,
      source: editForm.source.trim(),
      comment: editForm.comment.trim(),
    })
    ElMessage.success(res.message || t('serverFirewall.ruleUpdated'))
    editVisible.value = false
    await load()
  } catch {
    /* handled */
  } finally {
    editing.value = false
  }
}

const tab = ref('ports')

/** 锁服后保持放通的端口（SSH 必含，面板端口强制包含） */
const allowPorts = computed(() => {
  const set = new Set<number>([22, 80, 443])
  if (status.panel_port) set.add(status.panel_port)
  return Array.from(set).sort((a, b) => a - b)
})

/** 该行的加固备注（后端以 zap:lock: 前缀标记，去掉前缀即显示 Ssh/http/https/zap） */
function lockRemarkOf(row: FirewallRule): string {
  if (typeof row.comment === 'string' && row.comment.startsWith('zap:lock:')) {
    return row.comment.slice('zap:lock:'.length)
  }
  return ''
}

const status = reactive<FirewallStatus>({
  backend: 'none',
  detected: 'none',
  active: false,
  enabled: false,
  wsl: false,
  panel_port: 0,
  rules: [],
  forwards: [],
  blacklist: [],
  ip_forward: false,
  forward_supported: false,
  locked_down: false,
})

const form = reactive({
  port: 80,
  proto: 'tcp' as 'tcp' | 'udp',
  action: 'accept' as 'accept' | 'drop',
  source: '',
  comment: '',
})

const forwardForm = reactive({
  proto: 'tcp' as 'tcp' | 'udp',
  listen_port: 8080,
  target_addr: '',
  target_port: 80,
})

const blackForm = reactive({ ip: '' })

// ── IP 白名单（放行优先） ────────────────────────────────────
const whiteForm = reactive({ ip: '', comment: '' })
const whitelist = ref<Array<{ ip: string; comment?: string; inRules: boolean }>>([])
const savingWhite = ref(false)

async function loadWhitelist() {
  try {
    const res = await getFirewallWhitelist()
    const rules = res.data?.rules || []
    whitelist.value = (res.data?.whitelist || []).map((x) => ({
      ip: x.ip,
      comment: x.comment,
      // 面板清单里有、但防火墙后端没有对应 accept 规则时给出提示
      inRules: rules.some((r) => r.ip === x.ip),
    }))
  } catch {
    /* handled */
  }
}

async function addWhite() {
  const ip = whiteForm.ip.trim()
  if (!ip) {
    ElMessage.warning(t('serverFirewall.ipPlaceholder'))
    return
  }
  savingWhite.value = true
  try {
    const res = await addFirewallWhitelist(ip, whiteForm.comment.trim())
    ElMessage.success(res.message || t('serverFirewall.whiteAdded'))
    whiteForm.ip = ''
    whiteForm.comment = ''
    await loadWhitelist()
  } catch {
    /* handled */
  } finally {
    savingWhite.value = false
  }
}

async function removeWhite(row: { ip: string }) {
  try {
    await ElMessageBox.confirm(
      t('serverFirewall.deleteWhiteConfirm', { ip: row.ip }),
      t('serverFirewall.deleteTitle'),
      { type: 'warning', confirmButtonText: t('common.delete') },
    )
  } catch {
    return
  }
  deletingId.value = `wl:${row.ip}`
  try {
    const res = await deleteFirewallWhitelist(row.ip)
    ElMessage.success(res.message || t('serverFirewall.whiteDeleted'))
    await loadWhitelist()
  } catch {
    /* handled */
  } finally {
    deletingId.value = ''
  }
}

// ── SSH 爆破防护（内置 fail2ban） ────────────────────────────
const banLoading = ref(false)
const savingBan = ref(false)
const unbanning = ref('')
const banIgnoredText = ref('')

const ban = reactive<{
  settings: BanSettings
  bans: BanRecord[]
  watching: Array<{ ip: string; hits: number; last: number }>
  ignored: Array<{ ip: string; hits: number; reason: string }>
  log_file: string
  last_run: number
  total_banned: number
  last_error: string
}>({
  settings: {
    enabled: false,
    max_retry: 5,
    find_time: 600,
    ban_time: 3600,
    interval: 60,
    ignore_private: true,
    ignored_ips: [],
    log_file: '',
  },
  bans: [],
  watching: [],
  ignored: [],
  log_file: '',
  last_run: 0,
  total_banned: 0,
  last_error: '',
})

async function loadBan() {
  banLoading.value = true
  try {
    const res = await getBanStatus()
    const d = res.data
    if (!d) return
    Object.assign(ban, d)
    banIgnoredText.value = (d.settings?.ignored_ips || []).join(',')
  } catch {
    /* handled */
  } finally {
    banLoading.value = false
  }
}

async function saveBan() {
  savingBan.value = true
  try {
    await saveBanSettings({
      ...ban.settings,
      ignored_ips: banIgnoredText.value
        .split(/[,;\s]+/)
        .map((s) => s.trim())
        .filter(Boolean),
    })
    ElMessage.success(t('serverFirewall.banSaved'))
    await loadBan()
  } catch {
    /* handled */
  } finally {
    savingBan.value = false
  }
}

async function doUnban(ip: string) {
  unbanning.value = ip
  try {
    const res = await unbanIp(ip)
    ElMessage.success(res.message || t('serverFirewall.banUnbanned'))
    await loadBan()
  } catch {
    /* handled */
  } finally {
    unbanning.value = ''
  }
}

function fmtRemaining(sec: number): string {
  if (sec <= 0) return '-'
  const h = Math.floor(sec / 3600)
  const m = Math.floor((sec % 3600) / 60)
  if (h > 0) return `${h}h ${m}m`
  return `${m}m ${sec % 60}s`
}

function fmtTime(ts: number): string {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

async function load() {
  loading.value = true
  try {
    const res = await getFirewallStatus()
    const d = res.data || ({} as Partial<FirewallStatus>)
    status.backend = d.backend || 'none'
    status.detected = d.detected || 'none'
    status.wsl = !!d.wsl
    status.active = !!d.active
    status.enabled = !!d.enabled
    status.panel_port = d.panel_port || 0
    status.rules = d.rules || []
    status.forwards = d.forwards || []
    status.blacklist = d.blacklist || []
    status.ip_forward = !!d.ip_forward
    status.forward_supported = !!d.forward_supported
    status.locked_down = !!d.locked_down
  } catch {
    /* handled */
  } finally {
    loading.value = false
  }
}

function validPort(p: number): boolean {
  return Number.isInteger(p) && p >= 1 && p <= 65535
}

/** 轻量 IP 校验（IPv4 / IPv6 粗检），精确校验由后端完成 */
function looksLikeIp(s: string): boolean {
  if (!s) return false
  if (s.includes(':')) return /^[0-9a-fA-F:.]+$/.test(s)
  return /^(\d{1,3}\.){3}\d{1,3}$/.test(s)
}

async function addRule() {
  const port = Number(form.port)
  if (!validPort(port)) {
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

async function addForward() {
  if (!validPort(Number(forwardForm.listen_port)) || !validPort(Number(forwardForm.target_port))) {
    ElMessage.warning(t('serverFirewall.portRange'))
    return
  }
  const addr = forwardForm.target_addr.trim()
  if (!looksLikeIp(addr)) {
    ElMessage.warning(t('serverFirewall.targetAddrPlaceholder'))
    return
  }
  savingForward.value = true
  try {
    const res = await addFirewallForward({
      proto: forwardForm.proto,
      listen_port: Number(forwardForm.listen_port),
      target_addr: addr,
      target_port: Number(forwardForm.target_port),
    })
    ElMessage.success(res.message || t('serverFirewall.forwardAdded'))
    forwardForm.target_addr = ''
    await load()
  } catch {
    /* handled */
  } finally {
    savingForward.value = false
  }
}

async function removeForward(row: FirewallForward) {
  const label = `${row.proto?.toUpperCase()}/${row.listen_port} → ${row.target_addr}:${row.target_port}`
  try {
    await ElMessageBox.confirm(
      t('serverFirewall.deleteForwardConfirm', { label }),
      t('serverFirewall.deleteTitle'),
      { type: 'warning', confirmButtonText: t('common.delete') },
    )
  } catch {
    return
  }
  deletingId.value = `fw:${row.id}`
  try {
    const res = await deleteFirewallForward(row.id)
    ElMessage.success(res.message || t('serverFirewall.forwardDeleted'))
    await load()
  } catch {
    /* handled */
  } finally {
    deletingId.value = ''
  }
}

async function addBlack() {
  const ip = blackForm.ip.trim()
  if (!ip) {
    ElMessage.warning(t('serverFirewall.ipPlaceholder'))
    return
  }
  savingBlack.value = true
  try {
    const res = await addFirewallBlacklist(ip)
    ElMessage.success(res.message || t('serverFirewall.blackAdded'))
    blackForm.ip = ''
    await load()
  } catch {
    /* handled */
  } finally {
    savingBlack.value = false
  }
}

async function removeBlack(row: FirewallBlackEntry) {
  try {
    await ElMessageBox.confirm(
      t('serverFirewall.deleteBlackConfirm', { ip: row.ip }),
      t('serverFirewall.deleteTitle'),
      { type: 'warning', confirmButtonText: t('common.delete') },
    )
  } catch {
    return
  }
  deletingId.value = `bl:${row.id}`
  try {
    const res = await deleteFirewallBlacklist(row.id)
    ElMessage.success(res.message || t('serverFirewall.blackDeleted'))
    await load()
  } catch {
    /* handled */
  } finally {
    deletingId.value = ''
  }
}

function downloadJson(name: string, data: unknown) {
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  URL.revokeObjectURL(url)
}

async function doExport() {
  exporting.value = true
  try {
    const res = await exportFirewall()
    const data = res.data || {
      version: 1,
      port_rules: [],
      forwards: [],
      blacklist: [],
    }
    const day = new Date().toISOString().slice(0, 10)
    downloadJson(`zap-firewall-rules-${day}.json`, data)
    ElMessage.success(t('serverFirewall.exportDone'))
  } catch {
    /* handled */
  } finally {
    exporting.value = false
  }
}

async function onImportFile(ev: Event) {
  const input = ev.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return
  let payload: FirewallExport
  try {
    payload = JSON.parse(await file.text())
  } catch {
    ElMessage.error(t('serverFirewall.importInvalid'))
    return
  }
  if (!payload || typeof payload !== 'object' || !payload.version) {
    ElMessage.error(t('serverFirewall.importInvalid'))
    return
  }
  const ports = payload.port_rules?.length || 0
  const forwards = payload.forwards?.length || 0
  const black = payload.blacklist?.length || 0
  if (ports + forwards + black === 0) {
    ElMessage.warning(t('serverFirewall.importInvalid'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('serverFirewall.importConfirm', { ports, forwards, black }),
      t('serverFirewall.importConfirmTitle'),
      { type: 'warning', confirmButtonText: t('common.confirm') },
    )
  } catch {
    return
  }
  importing.value = true
  try {
    const res = await importFirewall(payload)
    const failed: string[] = res.data?.failed || []
    if (failed.length) {
      await ElMessageBox.alert(failed.join('\n'), res.message || t('common.operationSuccess'), {
        type: 'warning',
        customStyle: { whiteSpace: 'pre-line' } as never,
      })
    } else {
      ElMessage.success(res.message || t('common.operationSuccess'))
    }
    await load()
  } catch {
    /* handled */
  } finally {
    importing.value = false
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

async function doLockdown(unlock: boolean) {
  const title = unlock ? t('serverFirewall.unlockTitle') : t('serverFirewall.lockTitle')
  const body = unlock
    ? t('serverFirewall.unlockConfirm')
    : t('serverFirewall.lockConfirm', { ports: allowPorts.value.join(' / ') })
  try {
    await ElMessageBox.confirm(body, title, {
      type: 'warning',
      confirmButtonText: unlock ? t('serverFirewall.unlockBtn') : t('serverFirewall.lockBtn'),
    })
  } catch {
    return
  }
  lockdowning.value = true
  try {
    const res = await lockdownFirewall(unlock)
    ElMessage.success(res.message || t('common.operationSuccess'))
    await load()
  } catch {
    /* handled */
  } finally {
    lockdowning.value = false
  }
}

onMounted(() => {
  load()
  loadWhitelist()
  loadBan()
})
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
.to {
  margin: 0 6px;
  color: var(--el-text-color-secondary);
}
.tab-desc {
  color: var(--el-text-color-secondary);
  font-size: 13px;
  margin: 4px 0 10px;
}
.backup-pane {
  max-width: 640px;
  padding: 8px 0;
}
.backup-block .backup-title {
  font-weight: 600;
  margin-bottom: 6px;
}
.backup-block .backup-tip {
  color: var(--el-text-color-secondary);
  font-size: 13px;
  margin-bottom: 12px;
}
.harden-block {
  max-width: 680px;
  padding: 8px 0;
}
.harden-block .allow-list {
  margin-top: 10px;
}
.ban-form {
  max-width: 720px;
}
.ban-form .hint {
  margin-left: 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.ban-err {
  margin-left: 10px;
  font-size: 12px;
  color: var(--el-color-danger);
}
</style>
