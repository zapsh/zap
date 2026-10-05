<script setup lang="ts">
/**
 * 我的 SSH 密钥管理（复用 /terminal/keys 接口，密钥以家目录文件形式存在 ~/.ssh）。
 *
 * 独立成组件，供「个人中心 → SSH 密钥」页签与「终端 → 我的 SSH 密钥」共用同一套逻辑。
 * 终端内原本内联的密钥管理已保留不动，这里是为个人中心新增的入口：
 * 部分用户没有终端权限 / 进不去终端，也能在个人中心自助管理自己的 SSH 密钥。
 */
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Key } from '@/icons'
import {
  getUserSshKeys,
  generateUserKey,
  importUserKey,
  deleteUserKey,
  getUserKeyPublic,
  getUserKeyPrivate,
  type UserSshKey,
} from '@/api/terminal'
import { useUserStore } from '@/stores/user'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const userStore = useUserStore()
const isReadOnly = computed(() => userStore.roles.includes('demo'))

// ── 列表 ──
const myKeys = ref<UserSshKey[]>([])
const keyLoading = ref(false)
const keySaving = ref(false)

async function loadMyKeys() {
  keyLoading.value = true
  try {
    const resp = await getUserSshKeys()
    myKeys.value = resp.data?.items || []
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.loadKeysFailed'))
  } finally {
    keyLoading.value = false
  }
}

onMounted(loadMyKeys)

// 复制公钥 / 查看私钥
async function copyPub(key: UserSshKey) {
  try {
    const resp = await getUserKeyPublic(key.name)
    const pub = resp.data?.public_key
    if (!pub) throw new Error(t('terminal.pubEmpty'))
    await navigator.clipboard.writeText(pub)
    ElMessage.success(t('terminal.pubCopied'))
  } catch (e: any) {
    ElMessage.error(e.message || t('common.copyFailed'))
  }
}

const showPrivateView = ref(false)
const privateViewKey = ref<{ name: string; content: string } | null>(null)

async function viewPrivate(key: UserSshKey) {
  try {
    const resp = await getUserKeyPrivate(key.name)
    const content = resp.data?.private_key
    if (!content) throw new Error(t('terminal.privEmpty'))
    privateViewKey.value = { name: key.name, content }
    showPrivateView.value = true
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.readPrivateFailed'))
  }
}

async function copyPrivate() {
  const k = privateViewKey.value
  if (!k) return
  try {
    await navigator.clipboard.writeText(k.content)
    ElMessage.success(t('terminal.privCopied'))
  } catch {
    ElMessage.error(t('common.copyFailed'))
  }
}

async function removeKey(key: UserSshKey) {
  try {
    await ElMessageBox.confirm(
      t('terminal.delKeyConfirm', { name: key.name }),
      t('terminal.delKeyTitle'),
      {
        type: 'warning',
        confirmButtonText: t('common.delete'),
        cancelButtonText: t('common.cancel'),
      },
    )
  } catch {
    return
  }
  try {
    await deleteUserKey(key.name)
    ElMessage.success(t('terminal.deleted'))
    await loadMyKeys()
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.deleteFailed'))
  }
}

// ── 生成 ──
const showKeyGen = ref(false)
const keyGenForm = ref({ name: '', key_type: 'ed25519', bits: 4096, comment: '' })

function openKeyGen() {
  keyGenForm.value = { name: '', key_type: 'ed25519', bits: 4096, comment: '' }
  showKeyGen.value = true
}

const KEY_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/

async function submitKeyGen() {
  const f = keyGenForm.value
  const name = f.name.trim()
  if (!KEY_NAME_RE.test(name)) {
    ElMessage.warning(t('terminal.keyNameRule'))
    return
  }
  keySaving.value = true
  try {
    await generateUserKey({
      name,
      key_type: f.key_type,
      bits: f.key_type === 'rsa' ? f.bits : undefined,
      comment: f.comment.trim() || undefined,
    })
    ElMessage.success(t('terminal.keyGenerated'))
    showKeyGen.value = false
    await loadMyKeys()
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.genFailed'))
  } finally {
    keySaving.value = false
  }
}

// ── 导入 ──
const showKeyImport = ref(false)
const keyImportForm = ref({ name: '', private_key: '', public_key: '', comment: '' })

function openKeyImport() {
  keyImportForm.value = { name: '', private_key: '', public_key: '', comment: '' }
  showKeyImport.value = true
}

async function submitKeyImport() {
  const f = keyImportForm.value
  const name = f.name.trim()
  if (!KEY_NAME_RE.test(name)) {
    ElMessage.warning(t('terminal.keyNameRule'))
    return
  }
  if (!f.private_key.trim()) {
    ElMessage.warning(t('terminal.privateKeyRequired'))
    return
  }
  keySaving.value = true
  try {
    await importUserKey({
      name,
      private_key: f.private_key,
      public_key: f.public_key.trim() || undefined,
      comment: f.comment.trim() || undefined,
    })
    ElMessage.success(t('terminal.keyImported'))
    showKeyImport.value = false
    await loadMyKeys()
  } catch (e: any) {
    ElMessage.error(e.message || t('terminal.importFailed'))
  } finally {
    keySaving.value = false
  }
}
</script>

<template>
  <div class="ssh-key-mgr">
    <el-alert type="info" :closable="false" show-icon style="margin-bottom: 12px">
      {{ t('terminal.keyAlert') }}
    </el-alert>

    <div class="keymgr-toolbar">
      <el-button type="primary" size="small" :icon="Plus" :disabled="isReadOnly" @click="openKeyGen">
        {{ t('terminal.genKey') }}
      </el-button>
      <el-button size="small" :icon="Key" :disabled="isReadOnly" @click="openKeyImport">
        {{ t('terminal.importKey') }}
      </el-button>
      <div style="flex: 1"></div>
      <el-button size="small" text :loading="keyLoading" @click="loadMyKeys">
        {{ t('common.refresh') }}
      </el-button>
    </div>

    <el-table :data="myKeys" v-loading="keyLoading" size="small" max-height="400" :empty-text="t('terminal.noKeys')">
      <el-table-column prop="name" :label="t('common.name')" min-width="130" />
      <el-table-column prop="comment" :label="t('terminal.comment')" min-width="120" show-overflow-tooltip />
      <el-table-column :label="t('terminal.fingerprint')" min-width="210" show-overflow-tooltip>
        <template #default="{ row }">
          <code class="fp">{{ row.fingerprint }}</code>
        </template>
      </el-table-column>
      <el-table-column :label="t('common.operation')" width="220" align="right">
        <template #default="{ row }">
          <el-button link type="primary" size="small" @click="copyPub(row)">{{
            t('terminal.copyPub')
          }}</el-button>
          <el-button link type="primary" size="small" @click="viewPrivate(row)">
            {{ t('terminal.viewPrivate') }}
          </el-button>
          <el-button
            link
            type="danger"
            size="small"
            :disabled="isReadOnly"
            @click="removeKey(row)"
          >
            {{ t('common.delete') }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>

    <!-- 生成密钥 -->
    <el-dialog v-model="showKeyGen" :title="t('terminal.genKeyTitle')" width="480px" :close-on-click-modal="false">
      <el-form label-width="90px" @submit.prevent>
        <el-form-item :label="t('terminal.keyName')" required>
          <el-input v-model="keyGenForm.name" :placeholder="t('terminal.keyNamePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('terminal.keyType')" required>
          <el-radio-group v-model="keyGenForm.key_type">
            <el-radio value="ed25519">{{ t('terminal.typeEd25519') }}</el-radio>
            <el-radio value="rsa">RSA</el-radio>
            <el-radio value="ecdsa">ECDSA</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item v-if="keyGenForm.key_type === 'rsa'" :label="t('terminal.rsaBits')">
          <el-select v-model="keyGenForm.bits" style="width: 120px">
            <el-option :value="2048" label="2048" />
            <el-option :value="4096" label="4096" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('terminal.comment')">
          <el-input v-model="keyGenForm.comment" :placeholder="t('terminal.commentPlaceholder')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showKeyGen = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="keySaving" @click="submitKeyGen">{{
          t('terminal.genAndSave')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- 导入密钥 -->
    <el-dialog v-model="showKeyImport" :title="t('terminal.importKeyTitle')" width="560px" :close-on-click-modal="false">
      <el-form label-width="90px" @submit.prevent>
        <el-form-item :label="t('terminal.keyName')" required>
          <el-input v-model="keyImportForm.name" :placeholder="t('terminal.keyNamePlaceholder')" />
        </el-form-item>
        <el-form-item :label="t('terminal.privateKey')" required>
          <el-input
            v-model="keyImportForm.private_key"
            type="textarea"
            :rows="9"
            :placeholder="t('terminal.privateKeyPlaceholder')"
            style="font-family: monospace"
          />
        </el-form-item>
        <el-form-item :label="t('terminal.publicKey')">
          <el-input
            v-model="keyImportForm.public_key"
            type="textarea"
            :rows="3"
            :placeholder="t('terminal.publicKeyPlaceholder')"
            style="font-family: monospace"
          />
        </el-form-item>
        <el-form-item :label="t('terminal.comment')">
          <el-input v-model="keyImportForm.comment" :placeholder="t('common.optional')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showKeyImport = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="keySaving" @click="submitKeyImport">
          {{ t('terminal.importAndSave') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 查看私钥 -->
    <el-dialog v-model="showPrivateView" :title="t('terminal.viewPrivateTitle')" width="620px">
      <el-alert type="warning" :closable="false" show-icon style="margin-bottom: 10px">
        {{ t('terminal.privateAlert', { name: privateViewKey?.name }) }}
      </el-alert>
      <el-input
        :model-value="privateViewKey?.content ?? ''"
        type="textarea"
        :rows="12"
        readonly
        style="font-family: monospace"
      />
      <template #footer>
        <el-button @click="showPrivateView = false">{{ t('common.close') }}</el-button>
        <el-button type="primary" @click="copyPrivate">{{ t('terminal.copyPrivate') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.ssh-key-mgr {
  max-width: 920px;
}
.keymgr-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}
.fp {
  font-family: monospace;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
</style>
