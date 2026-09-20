<template>
  <div class="ssl-container">
    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <div class="header-left">
            <span class="title">{{ t('sslCerts.cardTitle') }}</span>
            <el-tag type="warning" size="small" style="margin-left: 8px">SSL/TLS</el-tag>
          </div>
          <div class="header-actions">
            <el-button type="primary" :icon="Plus" @click="openAdd">{{
              t('sslCerts.addCert')
            }}</el-button>
            <el-button :icon="Key" @click="openSelfSign">{{ t('sslCerts.selfSignBtn') }}</el-button>
            <el-button type="success" :icon="MagicStick" @click="openLetsEncrypt">{{
              t('sslCerts.letsEncryptBtn')
            }}</el-button>
            <el-button :icon="IconDns" @click="openDnsDrawer">{{
              t('sslCerts.dnsProviderBtn')
            }}</el-button>
            <el-badge :value="orders.length" :hidden="!orders.length" type="primary">
              <el-button :icon="Loading" @click="openOrders">{{
                t('sslCerts.leOrdersBtn')
              }}</el-button>
            </el-badge>
          </div>
        </div>
      </template>

      <!-- <el-alert type="info" :closable="false" class="tip">
    
      </el-alert> -->

      <el-table :data="tableData" v-loading="loading" stripe style="margin-top: 14px">
        <el-table-column prop="id" label="ID" width="60" />
        <el-table-column v-if="canManageAll" :label="t('sslCerts.colOwner')" width="160">
          <template #default="{ row }">
            <el-tag v-if="!row.user_id" size="small" effect="plain" type="info">{{
              t('sslCerts.tagSystem')
            }}</el-tag>
            <span v-else>{{ certOwnerText(row) }}</span>
          </template>
        </el-table-column>
        <el-table-column
          prop="name"
          :label="t('sslCerts.colName')"
          min-width="130"
          show-overflow-tooltip
        />
        <el-table-column :label="t('sslCerts.colDomains')" min-width="170" show-overflow-tooltip>
          <template #default="{ row }">
            <span v-if="row.domains">{{ row.domains }}</span>
            <span v-else class="never">-</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.colType')" width="150">
          <template #default="{ row }">
            <el-tag :type="certTypeTag(row.cert_type)" size="small">{{
              certTypeLabel(row.cert_type)
            }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.colStatus')" width="90">
          <template #default="{ row }">
            <el-switch
              :model-value="row.status === 1"
              inline-prompt
              :active-text="t('sslCerts.enable')"
              :inactive-text="t('sslCerts.disable')"
              @change="(v: boolean) => toggleStatus(row, v)"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.colNotAfter')" width="170">
          <template #default="{ row }">
            <el-tag v-if="row.not_after > 0 && row.not_after < nowTs" type="danger" size="small">{{
              t('sslCerts.expired')
            }}</el-tag>
            <span v-else>{{ row.not_after ? fmtTime(row.not_after) : '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.colRemark')" min-width="140" show-overflow-tooltip>
          <template #default="{ row }">{{ row.remark || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.colUpdatedAt')" width="160">
          <template #default="{ row }">{{ fmtTime(row.updated_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="180" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link @click="openDetail(row)">{{
              t('sslCerts.detail')
            }}</el-button>
            <el-button type="primary" link @click="openEdit(row)">{{ t('common.edit') }}</el-button>
            <el-button type="danger" link @click="handleDelete(row)">{{
              t('common.delete')
            }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 添加 / 编辑 -->
    <el-dialog
      v-model="editVisible"
      :title="editForm.id ? t('sslCerts.editTitle') : t('sslCerts.addTitle')"
      width="920px"
      top="4vh"
      @closed="resetEdit"
    >
      <el-form :model="editForm" label-width="86px" @submit.prevent>
        <el-form-item v-if="canManageAll" :label="t('sslCerts.ownerLabel')">
          <el-select
            v-model="editForm.user_id"
            filterable
            clearable
            :loading="ownersLoading"
            :placeholder="t('sslCerts.ownerPlaceholder')"
            style="width: 360px"
          >
            <el-option
              v-for="o in ownerOptions"
              :key="o.id"
              :label="`${o.nickname || o.username} (${o.username})`"
              :value="o.id"
            />
          </el-select>
          <span class="form-hint">{{ t('sslCerts.ownerHint') }}</span>
        </el-form-item>
        <el-row :gutter="14">
          <el-col :span="12">
            <el-form-item :label="t('sslCerts.certName')">
              <el-input
                v-model="editForm.name"
                :placeholder="t('sslCerts.certNamePlaceholder')"
                maxlength="80"
                @input="nameManual = true"
              />
            </el-form-item>
          </el-col>
          <el-col :span="12">
            <el-form-item :label="t('sslCerts.domains')">
              <el-input
                v-model="editForm.domains"
                :placeholder="t('sslCerts.domainsPlaceholder')"
                @input="domainsManual = true"
              />
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item :label="t('sslCerts.colRemark')">
          <el-input v-model="editForm.remark" maxlength="200" />
        </el-form-item>
      </el-form>

      <div v-for="g in pemGroups" :key="g.key" class="pem-group">
        <div class="pem-header">
          <div class="pem-title-wrap" @click="togglePem(g)">
            <el-icon class="arrow" :class="{ 'is-open': !collapsed[g.key] }"
              ><ArrowRight
            /></el-icon>
            <span class="pem-title">{{ g.title }}</span>
            <el-tag v-if="g.optional" size="small" effect="plain" type="info">{{
              t('sslCerts.optional')
            }}</el-tag>
            <el-tag v-if="editForm[g.key]" size="small" effect="plain" type="success">{{
              t('sslCerts.filled')
            }}</el-tag>
          </div>
          <div class="pem-actions">
            <el-button size="small" @click="pickFile(g.key)">{{
              t('sslCerts.importFile')
            }}</el-button>
            <el-button
              v-if="editForm[g.key]"
              size="small"
              type="danger"
              link
              @click="editForm[g.key] = ''"
            >
              {{ t('sslCerts.clear') }}
            </el-button>
          </div>
        </div>

        <el-collapse-transition>
          <div v-show="!collapsed[g.key]">
            <el-input
              v-model="editForm[g.key]"
              type="textarea"
              :rows="g.rows || 5"
              class="mono"
              :placeholder="g.placeholder"
            />

            <!-- 证书解析结果（自动读取域名，无需手工填写） -->
            <div v-if="g.key === 'cert_content'" class="parse-box">
              <span v-if="parseState.loading" class="parse-tip">
                <el-icon class="is-loading"><Loading /></el-icon> {{ t('sslCerts.parsing') }}
              </span>
              <div v-else-if="parseState.info" class="parse-info">
                <el-tag size="small" type="success" effect="dark">{{
                  t('sslCerts.parsed')
                }}</el-tag>
                <span>{{ t('sslCerts.domainsCount', { n: parseState.info.domains.length }) }}</span>
                <span v-if="parseState.info.not_after">
                  {{
                    t('sslCerts.validUntilLeft', {
                      time: fmtTime(parseState.info.not_after),
                      days: daysLeft(parseState.info.not_after),
                    })
                  }}
                </span>
                <span v-if="parseState.info.issuer">{{
                  t('sslCerts.issuer', { issuer: parseState.info.issuer })
                }}</span>
                <span v-if="parseState.info.key_type">
                  {{
                    t('sslCerts.keyBits', {
                      type: parseState.info.key_type,
                      bits: parseState.info.key_bits,
                    })
                  }}
                </span>
                <span v-if="parseState.info.fingerprint" class="fp"
                  >SHA256 {{ parseState.info.fingerprint }}</span
                >
              </div>
              <span v-else-if="parseState.error" class="parse-tip is-error">{{
                parseState.error
              }}</span>

              <!-- 证书与私钥配对校验 -->
              <div v-if="parseState.info?.key_match === true" class="parse-pair is-ok">
                <el-icon><CircleCheckFilled /></el-icon> {{ t('sslCerts.keyPairOk') }}
              </div>
              <div v-else-if="parseState.info?.key_match === false" class="parse-pair is-bad">
                <el-icon><CircleCloseFilled /></el-icon>
                {{ t('sslCerts.keyPairBad') }}
              </div>
              <div v-else-if="parseState.info?.key_error" class="parse-pair is-bad">
                <el-icon><WarningFilled /></el-icon> {{ parseState.info.key_error }}
              </div>

              <div
                v-if="parseState.info?.cert_count && parseState.info.cert_count > 1"
                class="parse-chain"
              >
                {{ t('sslCerts.chainDetected', { n: parseState.info.cert_count }) }}
                <el-button size="small" text type="primary" @click="splitChain">{{
                  t('sslCerts.splitChain')
                }}</el-button>
              </div>
            </div>
          </div>
        </el-collapse-transition>
      </div>

      <template #footer>
        <el-button @click="editVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="submitSave">{{
          t('common.save')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 详情 -->
    <el-dialog v-model="detailVisible" :title="t('sslCerts.detailTitle')" width="860px" top="4vh">
      <el-descriptions :column="2" border size="small" style="margin-bottom: 12px">
        <el-descriptions-item :label="t('sslCerts.colName')">{{
          detail?.name
        }}</el-descriptions-item>
        <el-descriptions-item :label="t('sslCerts.colType')">{{
          detail ? certTypeLabel(detail.cert_type) : ''
        }}</el-descriptions-item>
        <el-descriptions-item v-if="canManageAll" :label="t('sslCerts.colOwner')">
          {{ detail ? certOwnerText(detail) : '' }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('sslCerts.colDomains')" :span="2">{{
          detail?.domains || '-'
        }}</el-descriptions-item>
        <el-descriptions-item :label="t('sslCerts.colNotAfter')">
          {{ detail && detail.not_after ? fmtTime(detail.not_after) : '-' }}
        </el-descriptions-item>
        <el-descriptions-item :label="t('sslCerts.colRemark')">{{
          detail?.remark || '-'
        }}</el-descriptions-item>
      </el-descriptions>

      <el-tabs v-if="detail" type="border-card">
        <el-tab-pane v-for="g in pemGroups" :key="g.key" :label="g.title">
          <div class="detail-toolbar">
            <el-button size="small" @click="copyText(detail[g.key] || '', g.title)">{{
              t('sslCerts.copy')
            }}</el-button>
            <el-button
              size="small"
              @click="downloadText(detail[g.key] || '', g.filename(detail))"
              >{{ t('sslCerts.download') }}</el-button
            >
          </div>
          <pre class="pem-view mono">{{ detail[g.key] || t('sslCerts.empty') }}</pre>
        </el-tab-pane>
      </el-tabs>

      <template #footer>
        <el-button @click="detailVisible = false">{{ t('sslCerts.close') }}</el-button>
        <el-button type="primary" @click="openEdit(detail)">{{ t('sslCerts.editThis') }}</el-button>
      </template>
    </el-dialog>

    <!-- 自签名 -->
    <el-dialog v-model="selfSignVisible" :title="t('sslCerts.selfSignTitle')" width="560px">
      <el-alert
        type="warning"
        :closable="false"
        show-icon
        :description="t('sslCerts.selfSignAlert')"
      />
      <el-form label-width="90px" style="margin-top: 12px" @submit.prevent>
        <el-form-item :label="t('sslCerts.certName')">
          <el-input
            v-model="selfSignForm.name"
            :placeholder="t('sslCerts.selfSignNamePlaceholder')"
            maxlength="80"
          />
        </el-form-item>
        <el-form-item :label="t('sslCerts.domainsIp')">
          <el-input
            v-model="selfSignForm.domains"
            placeholder="localhost, 127.0.0.1, my.example.com"
          />
          <span class="form-hint">{{ t('sslCerts.domainsIpHint') }}</span>
        </el-form-item>
        <el-form-item v-if="canManageAll" :label="t('sslCerts.ownerLabel')">
          <el-select
            v-model="selfSignForm.user_id"
            filterable
            :loading="ownersLoading"
            :placeholder="t('sslCerts.ownerLabel')"
            style="width: 100%"
          >
            <el-option
              v-for="o in ownerOptions"
              :key="o.id"
              :label="`${o.nickname || o.username} (${o.username})`"
              :value="o.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('sslCerts.days')">
          <el-input-number v-model="selfSignForm.days" :min="1" :max="3650" />
          <span class="form-hint">{{ t('sslCerts.daysHint') }}</span>
        </el-form-item>
        <el-form-item :label="t('sslCerts.colRemark')">
          <el-input v-model="selfSignForm.remark" maxlength="200" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="selfSignVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="selfSigning" @click="submitSelfSign">{{
          t('sslCerts.generateSave')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- Let's Encrypt 申请向导：填写 → 域名验证 → 结果 -->
    <el-dialog
      v-model="leVisible"
      :title="t('sslCerts.leTitle')"
      width="820px"
      top="4vh"
      :close-on-click-modal="false"
      @closed="onLeClosed"
    >
      <el-steps :active="leStep" finish-status="success" align-center class="le-steps">
        <el-step :title="t('sslCerts.leStepApply')" />
        <el-step :title="t('sslCerts.leStepAuth')" />
        <el-step :title="t('sslCerts.leStepDone')" />
      </el-steps>

      <!-- 第一步：申请信息 -->
      <div v-show="leStep === 0">
        <el-alert type="info" :closable="false" show-icon :description="t('sslCerts.leAlert')" />
        <el-form label-width="118px" class="le-form" @submit.prevent>
          <el-form-item :label="t('sslCerts.domains')">
            <el-input
              v-model="leForm.domains"
              :placeholder="t('sslCerts.leDomainsPlaceholder')"
              @input="leWildcardHint"
            />
            <span class="form-hint">{{ t('sslCerts.leWildcardHint') }}</span>
          </el-form-item>
          <el-form-item :label="t('sslCerts.leEmail')">
            <el-input v-model="leForm.email" :placeholder="t('sslCerts.leEmailPlaceholder')" />
          </el-form-item>
          <el-form-item :label="t('sslCerts.leValidation')">
            <el-radio-group v-model="leForm.validation">
              <el-radio value="dns">{{ t('sslCerts.leValidationDns') }}</el-radio>
              <el-radio value="http">{{ t('sslCerts.leValidationHttp') }}</el-radio>
            </el-radio-group>
            <span class="form-hint">{{ t('sslCerts.leValidationHint') }}</span>
          </el-form-item>
          <el-form-item v-if="leForm.validation === 'dns'" :label="t('sslCerts.leDnsMode')">
            <el-radio-group v-model="leForm.dns_mode">
              <el-radio value="manual">{{ t('sslCerts.leDnsManual') }}</el-radio>
              <el-radio value="auto">{{ t('sslCerts.leDnsAuto') }}</el-radio>
            </el-radio-group>
            <span class="form-hint">{{ t('sslCerts.leDnsModeHint') }}</span>
          </el-form-item>
          <el-form-item
            v-if="leForm.validation === 'dns' && leForm.dns_mode === 'auto'"
            :label="t('sslCerts.leDnsProvider')"
          >
            <el-select
              v-model="leForm.dns_provider_id"
              :placeholder="t('sslCerts.leDnsProviderPlaceholder')"
              :loading="dnsProvidersLoading"
              style="width: 100%"
            >
              <el-option
                v-for="p in dnsProviders"
                :key="p.id"
                :label="`${p.name}（${providerLabel(p.provider)}）`"
                :value="p.id"
              />
            </el-select>
            <span class="form-hint">{{ t('sslCerts.leDnsProviderHint') }}</span>
            <el-button link type="primary" size="small" @click="goDnsProviders">{{
              t('sslCerts.leGoDnsProviders')
            }}</el-button>
          </el-form-item>
          <el-form-item :label="t('sslCerts.certName')">
            <el-input
              v-model="leForm.name"
              :placeholder="t('sslCerts.leNamePlaceholder')"
              maxlength="80"
            />
          </el-form-item>
          <el-form-item v-if="canManageAll" :label="t('sslCerts.ownerLabel')">
            <el-select
              v-model="leForm.user_id"
              filterable
              :loading="ownersLoading"
              :placeholder="t('sslCerts.ownerLabel')"
              style="width: 100%"
            >
              <el-option
                v-for="o in ownerOptions"
                :key="o.id"
                :label="`${o.nickname || o.username} (${o.username})`"
                :value="o.id"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('sslCerts.leStaging')">
            <el-switch v-model="leForm.staging" />
            <span class="form-hint">{{ t('sslCerts.leStagingHint') }}</span>
          </el-form-item>
          <el-form-item :label="t('sslCerts.colRemark')">
            <el-input v-model="leForm.remark" maxlength="200" />
          </el-form-item>
        </el-form>
      </div>

      <!-- 第二步：DNS-01 手动 → 展示待解析记录；其余方式 → 后台自动校验 -->
      <div v-show="leStep === 1">
        <template v-if="leOrder?.dns_mode === 'manual' && leOrder.challenge_type === 'dns-01'">
          <el-alert
            type="warning"
            :closable="false"
            show-icon
            :title="t('sslCerts.leManualTitle')"
            :description="t('sslCerts.leManualAlert')"
          />
          <el-table :data="leOrder?.todos || []" size="small" class="le-todos">
            <el-table-column :label="t('sslCerts.colDomains')" width="180">
              <template #default="{ row }">{{ row.domain }}</template>
            </el-table-column>
            <el-table-column :label="t('sslCerts.leRecordType')" width="90">
              <template #default>TXT</template>
            </el-table-column>
            <el-table-column :label="t('sslCerts.leRecordHost')" min-width="200">
              <template #default="{ row }">
                <span class="mono">{{ row.dns_host }}</span>
                <el-button link type="primary" size="small" @click="copyText(row.dns_host, '')"
                  >{{ t('sslCerts.copy') }}</el-button
                >
              </template>
            </el-table-column>
            <el-table-column :label="t('sslCerts.leRecordValue')" min-width="260">
              <template #default="{ row }">
                <span class="mono">{{ row.dns_value }}</span>
                <el-button link type="primary" size="small" @click="copyText(row.dns_value, '')">{{
                  t('sslCerts.copy')
                }}</el-button>
              </template>
            </el-table-column>
            <el-table-column :label="t('sslCerts.lePropagated')" width="110">
              <template #default="{ row }">
                <el-tag v-if="row.propagated" type="success" size="small">{{
                  t('sslCerts.lePropagatedYes')
                }}</el-tag>
                <el-tag v-else type="info" size="small">{{ t('sslCerts.lePropagatedNo') }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
        </template>

        <!-- 自动模式：展示后端给出的阶段说明 -->
        <div v-else class="le-progress">
          <el-icon v-if="lePolling" class="is-loading le-spin"><Loading /></el-icon>
          <span>{{ leOrder?.stage || leStatusText }}</span>
        </div>

        <el-alert
          v-if="leOrder && leOrder.status === 'failed'"
          type="error"
          :closable="false"
          show-icon
          class="le-error"
          :description="leOrder.error || t('sslCerts.leFailedFallback')"
        />
      </div>

      <!-- 第三步：结果 -->
      <div v-show="leStep === 2" class="le-result">
        <template v-if="leOrder?.status === 'issued'">
          <div class="le-result-line">
            <el-icon class="is-ok"><CircleCheckFilled /></el-icon>
            <span>{{ t('sslCerts.leDoneOk') }}</span>
          </div>
          <el-button type="primary" link @click="openIssuedCert">{{
            t('sslCerts.leViewCert')
          }}</el-button>
        </template>
        <div v-else class="le-result-line">
          <el-icon class="is-bad"><CircleCloseFilled /></el-icon>
          <span>{{ leOrder?.error || leStatusText }}</span>
          <el-button size="small" text type="primary" @click="restartWizard">{{
            t('sslCerts.leRetry')
          }}</el-button>
        </div>
      </div>

      <template #footer>
        <el-button @click="onLeCancel">{{ t('common.cancel') }}</el-button>
        <el-button v-if="leStep === 0" type="success" :loading="leBusy" @click="submitLetsEncrypt">{{
          t('sslCerts.startApply')
        }}</el-button>
        <el-button
          v-else-if="leStep === 1 && leOrder?.dns_mode === 'manual' && leOrder.challenge_type === 'dns-01'"
          type="primary"
          :loading="leBusy"
          @click="submitVerify"
          >{{ t('sslCerts.leVerifyBtn') }}</el-button
        >
        <el-button v-else-if="leStep === 2" type="primary" @click="leVisible = false">{{
          t('sslCerts.close')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 进行中的申请 -->
    <el-dialog v-model="ordersVisible" :title="t('sslCerts.leOrdersTitle')" width="760px">
      <el-alert
        type="info"
        :closable="false"
        show-icon
        :description="t('sslCerts.leOrdersAlert')"
        style="margin-bottom: 10px"
      />
      <el-table :data="orders" v-loading="ordersLoading" size="small">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column :label="t('sslCerts.colDomains')" min-width="200" show-overflow-tooltip>
          <template #default="{ row }">{{ (row.domains || []).join(', ') }}</template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.leChallenge')" width="110">
          <template #default="{ row }">{{ challengeLabel(row) }}</template>
        </el-table-column>
        <el-table-column :label="t('sslCerts.colStatus')" min-width="180">
          <template #default="{ row }">
            <el-tag size="small" :type="orderTagType(row.status)">{{ row.stage }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="170" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link @click="trackOrder(row)">{{
              t('sslCerts.leTrack')
            }}</el-button>
            <el-button type="danger" link @click="cancelOrder(row)">{{
              t('sslCerts.leCancelOrder')
            }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-dialog>

    <!-- DNS 服务商凭据：原二级子菜单，现改为抽屉，侧栏只留「SSL证书」一个入口 -->
    <el-drawer
      v-model="dnsDrawerVisible"
      :title="t('dnsProvider.cardTitle')"
      size="70%"
      destroy-on-close
      @closed="onDnsDrawerClosed"
    >
      <DnsProvidersPane embedded @changed="loadDnsProviders" />
    </el-drawer>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import {
  Plus,
  Key,
  MagicStick,
  ArrowRight,
  Loading,
  CircleCheckFilled,
  CircleCloseFilled,
  WarningFilled,
  Dns as IconDns,
} from '@/icons'
import DnsProvidersPane from '@/views/ssl-tls/dns-providers/DnsProvidersPane.vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { http } from '@/utils/request'
import { useUserStore } from '@/stores/user'
import {
  getCertList,
  getCertDetail,
  addCert,
  updateCert,
  deleteCert,
  selfSignCert,
  letsEncryptCert,
  parseCert,
  getAcmeOrders,
  getAcmeOrderStatus,
  verifyAcmeOrder,
  cancelAcmeOrder,
  getAcmeDnsProviders,
  getAcmeDnsList,
  type SslCertItem,
  type SslCertDetail,
  type SslCertParseResult,
  type OwnerOption,
  type AcmeOrder,
  type AcmeDnsProviderItem,
  type AcmeDnsProviderMeta,
} from '@/api/ssl'

const { t } = useI18n()

const nowTs = ref(Math.floor(Date.now() / 1000))
const loading = ref(false)
const tableData = ref<SslCertItem[]>([])

const userStore = useUserStore()
// admin / reseller 可管理他人（能看到全部或名下证书并可指定归属）；普通用户只能看到自己的
const canManageAll = computed(
  () => userStore.roles.includes('admin') || userStore.roles.includes('reseller'),
)
const myUserId = computed(() => userStore.userInfo.id)

// 归属用户下拉（admin 全部 / reseller 名下客户），与站点归属一致
const ownerOptions = ref<OwnerOption[]>([])
const ownersLoading = ref(false)
async function loadOwners() {
  if (!canManageAll.value) return
  ownersLoading.value = true
  try {
    const res = await http.get<{ code: number; data: OwnerOption[] }>('/site/users')
    ownerOptions.value = res.data || []
  } catch {
    /* handled */
  } finally {
    ownersLoading.value = false
  }
}
const ownerLabel = (id?: number) => {
  const o = ownerOptions.value.find((x) => x.id === id)
  return o ? `${o.nickname || o.username} (${o.username})` : ''
}
/** 列表/详情展示证书归属：0 = 历史系统证书 */
const certOwnerText = (row: { user_id?: number; owner_name?: string }) => {
  if (!row.user_id) return t('sslCerts.tagSystem')
  return row.owner_name || ownerLabel(row.user_id) || `#${row.user_id}`
}

async function loadList() {
  loading.value = true
  try {
    const res = await getCertList()
    tableData.value = res.data ?? []
  } catch {
    /* handled by interceptor */
  } finally {
    loading.value = false
  }
}

type CertTag = 'info' | 'success' | 'warning' | 'primary' | 'danger'

const typeMeta = computed<Record<string, { label: string; tag: CertTag }>>(() => ({
  upload: { label: t('sslCerts.typeUpload'), tag: 'info' },
  'self-signed': { label: t('sslCerts.typeSelfSigned'), tag: 'warning' },
  letsencrypt: { label: t('sslCerts.typeLetsEncrypt'), tag: 'success' },
  'letsencrypt-staging': { label: t('sslCerts.typeLetsEncryptStaging'), tag: 'danger' },
}))

function certTypeLabel(type: string) {
  return typeMeta.value[type]?.label ?? type
}
function certTypeTag(type: string): CertTag {
  return typeMeta.value[type]?.tag ?? 'info'
}

type PemKey = 'cert_content' | 'key_content' | 'ca_bundle' | 'csr'

interface PemField {
  key: PemKey
  title: string
  placeholder: string
  /** 选填项：默认折叠，可展开填写 */
  optional?: boolean
  rows?: number
  filename: (d: SslCertDetail) => string
}

const pemGroups = computed<PemField[]>(() => [
  {
    key: 'cert_content',
    title: t('sslCerts.pemCert'),
    placeholder: t('sslCerts.pemCertPlaceholder'),
    rows: 6,
    filename: (d) => `${d.name}.crt`,
  },
  {
    key: 'key_content',
    title: t('sslCerts.pemKey'),
    placeholder: t('sslCerts.pemKeyPlaceholder'),
    filename: (d) => `${d.name}.key`,
  },
  {
    key: 'ca_bundle',
    title: t('sslCerts.pemCaBundle'),
    placeholder: t('sslCerts.pemCaBundlePlaceholder'),
    optional: true,
    rows: 4,
    filename: (d) => `${d.name}-ca-bundle.crt`,
  },
  {
    key: 'csr',
    title: t('sslCerts.pemCsr'),
    placeholder: t('sslCerts.pemCsrPlaceholder'),
    optional: true,
    rows: 4,
    filename: (d) => `${d.name}.csr`,
  },
])

/** CA 中间链与 CSR 默认折叠，需要时点标题展开 */
const collapsed = reactive<Record<PemKey, boolean>>({
  cert_content: false,
  key_content: false,
  ca_bundle: true,
  csr: true,
})

function resetCollapse() {
  for (const g of pemGroups.value) collapsed[g.key] = !!g.optional
}
function togglePem(g: PemField) {
  collapsed[g.key] = !collapsed[g.key]
}

type EditForm = Pick<SslCertDetail, 'cert_content' | 'key_content' | 'ca_bundle' | 'csr'> &
  Record<string, string | number | undefined>

// ── 添加 / 编辑 ─────────────────────────────────────────────
const editVisible = ref(false)
const saving = ref(false)
const editForm = reactive<EditForm & { id?: number; user_id?: number }>({
  name: '',
  domains: '',
  cert_content: '',
  key_content: '',
  ca_bundle: '',
  csr: '',
  remark: '',
  user_id: undefined,
})

/** 用户是否手工改过名称 / 域名：改过之后不再被自动解析覆盖 */
const nameManual = ref(false)
const domainsManual = ref(false)
const parseState = reactive<{
  loading: boolean
  info?: SslCertParseResult
  error: string
}>({ loading: false, error: '' })

function resetEdit() {
  editForm.id = undefined
  editForm.name = ''
  editForm.domains = ''
  editForm.cert_content = ''
  editForm.key_content = ''
  editForm.ca_bundle = ''
  editForm.csr = ''
  editForm.remark = ''
  editForm.user_id = undefined
  nameManual.value = false
  domainsManual.value = false
  parseState.info = undefined
  parseState.error = ''
  resetCollapse()
}

function openAdd() {
  resetEdit()
  if (canManageAll.value) editForm.user_id = myUserId.value
  editVisible.value = true
}

// ── 证书自动解析（域名 / 有效期 / 名称）──────────────────────
let parseTimer: ReturnType<typeof setTimeout> | undefined

function scheduleParse() {
  if (parseTimer) clearTimeout(parseTimer)
  parseTimer = setTimeout(runParse, 500)
}

async function runParse() {
  if (parseTimer) {
    clearTimeout(parseTimer)
    parseTimer = undefined
  }
  if (!editVisible.value) return
  const pem = String(editForm.cert_content || '').trim() || String(editForm.csr || '').trim()
  if (!pem) {
    parseState.info = undefined
    parseState.error = ''
    return
  }
  parseState.loading = true
  try {
    const res = await parseCert(pem, String(editForm.key_content || ''))
    parseState.info = res.data
    parseState.error = ''
    if (!domainsManual.value && res.data?.domains_str) {
      editForm.domains = res.data.domains_str
    }
    if (!nameManual.value) {
      const auto = res.data?.common_name || res.data?.domains?.[0] || ''
      if (auto) editForm.name = auto
    }
  } catch (e: any) {
    parseState.info = undefined
    parseState.error = t('sslCerts.parseFailed', {
      msg: e?.response?.data?.message || e?.message || t('sslCerts.parseFailedFallback'),
    })
  } finally {
    parseState.loading = false
  }
}

watch([() => editForm.cert_content, () => editForm.csr, () => editForm.key_content], scheduleParse)

/** 粘贴的是 fullchain 时，把叶子证书之外的证书挪到 CA 中间链 */
function splitChain() {
  const raw = String(editForm.cert_content || '')
  const blocks = raw.match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g)
  if (!blocks || blocks.length < 2) return
  editForm.cert_content = blocks[0]
  editForm.ca_bundle = blocks.slice(1).join('\n')
  collapsed.ca_bundle = true
  ElMessage.success(t('sslCerts.splitDone', { n: blocks.length - 1 }))
}

function daysLeft(ts: number) {
  return Math.max(0, Math.ceil((ts - nowTs.value) / 86400))
}

async function openEdit(row: SslCertItem | SslCertDetail | undefined) {
  if (!row) return
  let detail: SslCertDetail | undefined
  try {
    const res = await getCertDetail(row.id)
    detail = res.data
  } catch {
    return
  }
  if (!detail) return
  editForm.id = detail.id
  editForm.name = detail.name
  editForm.domains = detail.domains
  editForm.cert_content = detail.cert_content
  editForm.key_content = detail.key_content
  editForm.ca_bundle = detail.ca_bundle
  editForm.csr = detail.csr
  editForm.remark = detail.remark
  // 归属回填：历史系统证书（0）或归属用户已删除时不预设，便于改选
  editForm.user_id =
    canManageAll.value && ownerOptions.value.some((o) => o.id === detail.user_id)
      ? detail.user_id
      : undefined
  // 有内容的分组默认展开展示；选填项为空则保持折叠
  for (const g of pemGroups.value) {
    collapsed[g.key] = g.optional ? !String(editForm[g.key] ?? '').trim() : false
  }
  nameManual.value = !!detail.name
  domainsManual.value = !!detail.domains
  editVisible.value = true
  runParse()
}

function pickFile(key: string) {
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = '.pem,.crt,.cer,.key,.csr,.txt,text/plain'
  input.onchange = async () => {
    const f = input.files?.[0]
    if (!f) return
    try {
      const text = await f.text()
      ;(editForm as Record<string, string>)[key] = text
      ElMessage.success(t('sslCerts.imported', { name: f.name }))
    } catch {
      ElMessage.error(t('sslCerts.fileReadFailed'))
    }
  }
  input.click()
}

async function submitSave() {
  const name = String(editForm.name || '').trim()
  if (!name) {
    ElMessage.warning(t('sslCerts.nameRequired'))
    return
  }
  const hasMaterial =
    String(editForm.cert_content || '').trim() ||
    String(editForm.key_content || '').trim() ||
    String(editForm.csr || '').trim()
  if (!hasMaterial) {
    ElMessage.warning(t('sslCerts.materialRequired'))
    return
  }

  // 证书与私钥同时存在时，提交前先确认两者配对（后端也会校验，这里只做即时拦截）
  const hasKey = !!String(editForm.key_content || '').trim()
  if (hasKey && (String(editForm.cert_content || '').trim() || String(editForm.csr || '').trim())) {
    if (!parseState.info) await runParse()
    if (parseState.info?.key_match === false) {
      ElMessage.error(t('sslCerts.keyMismatch'))
      return
    }
    if (parseState.info?.key_error) {
      ElMessage.error(parseState.info.key_error)
      return
    }
  }

  saving.value = true
  try {
    const data = {
      name,
      domains: String(editForm.domains || '').trim(),
      cert_content: String(editForm.cert_content || ''),
      key_content: String(editForm.key_content || ''),
      ca_bundle: String(editForm.ca_bundle || ''),
      csr: String(editForm.csr || ''),
      remark: String(editForm.remark || '').trim(),
      ...(canManageAll.value ? { user_id: editForm.user_id || undefined } : {}),
    }
    if (editForm.id) {
      await updateCert({ id: editForm.id, ...data })
      ElMessage.success(t('sslCerts.saved'))
    } else {
      await addCert(data)
      ElMessage.success(t('sslCerts.added'))
    }
    editVisible.value = false
    loadList()
  } catch {
    /* handled */
  } finally {
    saving.value = false
  }
}

// ── 详情 ────────────────────────────────────────────────────
const detailVisible = ref(false)
const detail = ref<SslCertDetail>()

async function openDetail(row: SslCertItem) {
  try {
    const res = await getCertDetail(row.id)
    detail.value = res.data
    detailVisible.value = true
  } catch {
    /* handled */
  }
}

async function copyText(text: string, label: string) {
  try {
    await navigator.clipboard.writeText(text || '')
    ElMessage.success(t('sslCerts.copied', { label }))
  } catch {
    ElMessage.info(t('sslCerts.copyManual'))
  }
}

function downloadText(text: string, filename: string) {
  const blob = new Blob([text || ''], { type: 'application/octet-stream' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}

// ── 状态 / 删除 ─────────────────────────────────────────────
async function toggleStatus(row: SslCertItem, enabled: boolean) {
  try {
    await updateCert({
      id: row.id,
      name: row.name,
      domains: row.domains,
      remark: row.remark,
      status: enabled ? 1 : 0,
    })
    row.status = enabled ? 1 : 0
    ElMessage.success(enabled ? t('sslCerts.enabled') : t('sslCerts.disabled'))
  } catch {
    /* handled */
  }
}

async function handleDelete(row: SslCertItem) {
  try {
    await ElMessageBox.confirm(
      t('sslCerts.deleteConfirm', { name: row.name }),
      t('sslCerts.deleteTitle'),
      { type: 'warning', confirmButtonText: t('sslCerts.confirmDelete') },
    )
  } catch {
    return
  }
  try {
    await deleteCert(row.id)
    ElMessage.success(t('sslCerts.deleteOk'))
    loadList()
  } catch {
    /* handled */
  }
}

// ── 自签名 ──────────────────────────────────────────────────
const selfSignVisible = ref(false)
const selfSigning = ref(false)
const selfSignForm = reactive<{
  name: string
  domains: string
  days: number
  remark: string
  user_id?: number
}>({ name: '', domains: '', days: 365, remark: '', user_id: undefined })

function openSelfSign() {
  selfSignForm.name = ''
  selfSignForm.domains = ''
  selfSignForm.days = 365
  selfSignForm.remark = ''
  selfSignForm.user_id = canManageAll.value ? myUserId.value : undefined
  selfSignVisible.value = true
}

async function submitSelfSign() {
  if (!selfSignForm.name.trim()) {
    ElMessage.warning(t('sslCerts.nameRequired'))
    return
  }
  if (!selfSignForm.domains.trim()) {
    ElMessage.warning(t('sslCerts.selfSignDomainsRequired'))
    return
  }
  selfSigning.value = true
  try {
    await selfSignCert({
      name: selfSignForm.name.trim(),
      domains: selfSignForm.domains.trim(),
      days: selfSignForm.days || 365,
      remark: selfSignForm.remark.trim(),
      ...(canManageAll.value ? { user_id: selfSignForm.user_id || undefined } : {}),
    })
    ElMessage.success(t('sslCerts.selfSignOk'))
    selfSignVisible.value = false
    loadList()
  } catch {
    /* handled */
  } finally {
    selfSigning.value = false
  }
}

// ── Let's Encrypt 申请向导 ──────────────────────────────────
const leVisible = ref(false)
const leBusy = ref(false)
const leStep = ref(0)
const lePolling = ref(false)
const leStatusText = ref('')
const leOrder = ref<AcmeOrder>()
const leForm = reactive<{
  domains: string
  email: string
  name: string
  staging: boolean
  remark: string
  validation: 'dns' | 'http'
  dns_mode: 'manual' | 'auto'
  dns_provider_id?: number
  user_id?: number
}>({
  domains: '',
  email: '',
  name: '',
  staging: false,
  remark: '',
  validation: 'dns',
  dns_mode: 'manual',
  dns_provider_id: undefined,
  user_id: undefined,
})

// DNS 服务商下拉（DNS-01 自动模式）
const dnsProviders = ref<AcmeDnsProviderItem[]>([])
const dnsProviderMetas = ref<AcmeDnsProviderMeta[]>([])
const dnsProvidersLoading = ref(false)
const providerLabel = (kind: string) =>
  dnsProviderMetas.value.find((m) => m.kind === kind)?.label || kind

async function loadDnsProviders() {
  dnsProvidersLoading.value = true
  try {
    const [metas, list] = await Promise.all([getAcmeDnsProviders(), getAcmeDnsList()])
    dnsProviderMetas.value = metas.data ?? []
    dnsProviders.value = list.data ?? []
  } catch {
    /* handled */
  } finally {
    dnsProvidersLoading.value = false
  }
}

// ── DNS 服务商抽屉（侧栏不再给二级入口）────────────────────
const dnsDrawerVisible = ref(false)
/** 从 Let's Encrypt 表单跳进来时记一笔：抽屉关掉后把表单还给 user */
const dnsBackToLe = ref(false)

function openDnsDrawer() {
  dnsBackToLe.value = false
  dnsDrawerVisible.value = true
}

/** 申请表单里的「前往配置」：填了一半跑来配服务商，配完回到原有填写内容 */
function goDnsProviders() {
  dnsBackToLe.value = true
  leVisible.value = false
  dnsDrawerVisible.value = true
}

function onDnsDrawerClosed() {
  loadDnsProviders()
  if (dnsBackToLe.value) {
    dnsBackToLe.value = false
    leVisible.value = true
  }
}

/** 域名里含通配符时强制切到 DNS 验证（HTTP-01 不支持，后端也会拦，这里即时提示） */
function leWildcardHint(v: string) {
  if (String(v || '').includes('*.')) leForm.validation = 'dns'
}

function openLetsEncrypt() {
  leForm.domains = ''
  leForm.email = ''
  leForm.name = ''
  leForm.staging = false
  leForm.remark = ''
  leForm.validation = 'dns'
  leForm.dns_mode = 'manual'
  leForm.dns_provider_id = dnsProviders.value[0]?.id
  leForm.user_id = canManageAll.value ? myUserId.value : undefined
  leStep.value = 0
  leOrder.value = undefined
  leStatusText.value = ''
  if (!dnsProviders.value.length) loadDnsProviders()
  leVisible.value = true
}

let pollTimer: ReturnType<typeof setInterval> | undefined
function stopPoll() {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = undefined
  }
  lePolling.value = false
}

/** 轮询订单直到签发 / 失败：这里的等待可能长达数分钟，不能靠单个请求干等 */
function startPoll(orderId: number) {
  stopPoll()
  lePolling.value = true
  pollTimer = setInterval(async () => {
    try {
      const res = await getAcmeOrderStatus(orderId)
      const o = res.data
      leOrder.value = o
      if (o.status === 'pending' && o.dns_mode === 'manual' && o.challenge_type === 'dns-01') {
        // 仍在等用户解析：停留在第二步
        return
      }
      if (o.status === 'processing') return
      stopPoll()
      if (o.status === 'issued') {
        leStep.value = 2
        loadList()
      } else if (o.status === 'failed' || o.status === 'cancelled') {
        leStep.value = 2
      }
    } catch {
      /* 单次轮询失败不打断整体等待，下轮继续 */
    }
  }, 3000)
}

/** 走完第二步后继续轮询（自动模式与手动模式只是「谁来加 TXT」不同，后续一致） */
function maybeAutoAdvance(o: AcmeOrder) {
  if (o.dns_mode === 'manual' && o.challenge_type === 'dns-01') {
    leStep.value = 1
    lePolling.value = false
    return
  }
  leStep.value = 1
  startPoll(o.id)
}

function onLeClosed() {
  stopPoll()
}
function onLeCancel() {
  stopPoll()
  leVisible.value = false
}

async function submitLetsEncrypt() {
  if (!leForm.domains.trim()) {
    ElMessage.warning(t('sslCerts.leDomainsRequired'))
    return
  }
  if (!leForm.email.trim()) {
    ElMessage.warning(t('sslCerts.leEmailRequired'))
    return
  }
  if (leForm.validation === 'dns' && leForm.dns_mode === 'auto' && !leForm.dns_provider_id) {
    ElMessage.warning(t('sslCerts.leProviderRequired'))
    return
  }
  leBusy.value = true
  try {
    const res = await letsEncryptCert({
      domains: leForm.domains.trim(),
      email: leForm.email.trim(),
      validation: leForm.validation,
      ...(leForm.validation === 'dns' ? { dns_mode: leForm.dns_mode } : {}),
      ...(leForm.validation === 'dns' && leForm.dns_mode === 'auto'
        ? { dns_provider_id: leForm.dns_provider_id }
        : {}),
      name: leForm.name.trim() || undefined,
      staging: leForm.staging,
      remark: leForm.remark.trim() || undefined,
      ...(canManageAll.value ? { user_id: leForm.user_id || undefined } : {}),
    })
    const order = res.data
    leOrder.value = order
    maybeAutoAdvance(order)
    loadOrders()
  } catch {
    /* handled */
  } finally {
    leBusy.value = false
  }
}

/** DNS-01 手动模式下用户点「我已添加解析」 */
async function submitVerify() {
  if (!leOrder.value) return
  leBusy.value = true
  try {
    const res = await verifyAcmeOrder(leOrder.value.id)
    leOrder.value = res.data
    startPoll(res.data.id)
  } catch {
    /* handled */
  } finally {
    leBusy.value = false
  }
}

function restartWizard() {
  stopPoll()
  leStep.value = 0
  leOrder.value = undefined
}

async function openIssuedCert() {
  const id = leOrder.value?.cert_id
  leVisible.value = false
  if (!id) return
  try {
    const res = await getCertDetail(id)
    detail.value = res.data
    detailVisible.value = true
  } catch {
    /* handled */
  }
}

// ── 进行中的申请（跨会话可续）──────────────────────────────
const ordersVisible = ref(false)
const ordersLoading = ref(false)
const orders = ref<AcmeOrder[]>([])

async function loadOrders() {
  ordersLoading.value = true
  try {
    const res = await getAcmeOrders()
    orders.value = res.data ?? []
  } catch {
    /* handled */
  } finally {
    ordersLoading.value = false
  }
}

function openOrders() {
  ordersVisible.value = true
  loadOrders()
}

/** 从列表接管某个订单：DNS 手动模式回到第二步，其余直接开始轮询 */
function trackOrder(row: AcmeOrder) {
  leOrder.value = row
  leForm.domains = (row.domains || []).join(', ')
  leForm.validation = row.challenge_type === 'http-01' ? 'http' : 'dns'
  leForm.dns_mode = row.dns_mode
  ordersVisible.value = false
  maybeAutoAdvance(row)
  if (row.status === 'processing') startPoll(row.id)
  leVisible.value = true
}

async function cancelOrder(row: AcmeOrder) {
  try {
    await ElMessageBox.confirm(t('sslCerts.leCancelConfirm'), t('sslCerts.leCancelTitle'), {
      type: 'warning',
      confirmButtonText: t('sslCerts.confirmDelete'),
    })
  } catch {
    return
  }
  try {
    await cancelAcmeOrder(row.id)
    ElMessage.success(t('sslCerts.leCancelOk'))
    loadOrders()
  } catch {
    /* handled */
  }
}

const orderTagType = (s: string): 'info' | 'primary' | 'success' | 'danger' | 'warning' => {
  if (s === 'issued') return 'success'
  if (s === 'failed') return 'danger'
  if (s === 'processing') return 'primary'
  if (s === 'pending') return 'warning'
  return 'info'
}
const challengeLabel = (row: AcmeOrder) =>
  row.challenge_type === 'http-01'
    ? t('sslCerts.leValidationHttp')
    : row.dns_mode === 'auto'
      ? t('sslCerts.leDnsAuto')
      : t('sslCerts.leDnsManual')

function fmtTime(ts: number) {
  return ts ? new Date(ts * 1000).toLocaleString() : '-'
}

onMounted(() => {
  loadList()
  loadOwners()
  loadOrders()
  setInterval(() => {
    nowTs.value = Math.floor(Date.now() / 1000)
  }, 30000)
})

onBeforeUnmount(() => {
  if (parseTimer) clearTimeout(parseTimer)
  stopPoll()
})
</script>

<style scoped>
.ssl-container {
  padding: 20px;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
}
.header-left {
  display: flex;
  align-items: center;
}
.title {
  font-size: 16px;
  font-weight: 600;
}
.tip code,
.mono {
  font-family: 'JetBrains Mono', Consolas, monospace;
}
.tip code {
  background: var(--el-fill-color);
  border-radius: 3px;
  padding: 1px 5px;
}
.form-hint {
  margin-left: 10px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.never {
  color: var(--el-text-color-placeholder);
}
.pem-group {
  margin-top: 14px;
}
.pem-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.pem-title-wrap {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  user-select: none;
}
.pem-title-wrap:hover .pem-title {
  color: var(--el-color-primary);
}
.pem-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}
.arrow {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  transition: transform 0.2s ease-in-out;
}
.arrow.is-open {
  transform: rotate(90deg);
}
.pem-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.parse-box {
  margin-top: 6px;
  font-size: 12px;
  color: var(--el-text-color-regular);
}
.parse-info {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 14px;
  padding: 7px 10px;
  border-radius: 4px;
  background: var(--el-color-success-light-9);
  border: 1px solid var(--el-color-success-light-7);
}
.parse-info .fp {
  font-family: 'JetBrains Mono', Consolas, monospace;
  font-size: 11px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
.parse-tip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--el-text-color-secondary);
}
.parse-tip.is-error {
  color: var(--el-color-danger);
}
.parse-chain {
  margin-top: 6px;
  display: flex;
  align-items: center;
  gap: 4px;
  color: #e6a23c;
}
.parse-pair {
  margin-top: 6px;
  display: flex;
  align-items: center;
  gap: 5px;
}
.parse-pair.is-ok {
  color: #67c23a;
}
.parse-pair.is-bad {
  color: var(--el-color-danger);
}
.detail-toolbar {
  margin-bottom: 8px;
}
.le-steps {
  margin-bottom: 18px;
}
.le-form {
  margin-top: 14px;
}
.le-todos {
  margin-top: 12px;
}
.le-progress {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 24px 0;
  justify-content: center;
  color: var(--el-text-color-regular);
}
.le-spin {
  font-size: 18px;
  color: var(--el-color-primary);
}
.le-error {
  margin-top: 12px;
}
.le-result {
  padding: 24px 0;
  text-align: center;
}
.le-result-line {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  justify-content: center;
}
.le-result-line .is-ok {
  color: #67c23a;
  font-size: 22px;
}
.le-result-line .is-bad {
  color: var(--el-color-danger);
  font-size: 22px;
}
.pem-view {
  max-height: 300px;
  overflow: auto;
  margin: 0;
  padding: 10px 12px;
  background: var(--el-fill-color-light);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
}
</style>
