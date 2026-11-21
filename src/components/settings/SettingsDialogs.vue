<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { DEFAULT_DOWNLOAD_NAME_TEMPLATE } from '@/stores/settings'
import { useSyncStore } from '@/stores/sync'
import CookieLoginDialog from './CookieLoginDialog.vue'
import ConfigTransferDialogs from './ConfigTransferDialogs.vue'
import type { AccountSettings } from './useAccountSettings'
import type { DownloadSettings } from './useDownloadSettings'
import type { NetworkSettings } from './useNetworkSettings'
import type { SyncSettings } from './useSyncSettings'

// 设置页的页内对话框。开关状态由设置页的各分区 composable 持有（Escape 关闭也在设置页注册），
// 这里只渲染；挂载顺序与拆分前一致，Teleport 到 body 后的叠放次序不变
const props = defineProps<{
  sync: SyncSettings
  accounts: AccountSettings
  network: NetworkSettings
  downloads: DownloadSettings
}>()

const { t } = useI18n()
const syncStore = useSyncStore()
const {
  showGitHubDialog, githubPhase, githubToken, githubUsername, githubIsValidating, githubRepoMode,
  githubNewRepoName, githubExistingRepo, githubIsSettingRepo, githubValidateToken, githubFinishSetup, openGitHubTokenPage,
  showWebDavDialog, webdavUrl, webdavUsername, webdavPassword, webdavBasePath, webdavConfiguring, configureWebDav,
  showClearGitHubConfirm, confirmClearGitHub, hideProtocolUpgrade, upgradeDevicesConfirmed,
  configTransferMode, applyImportedConfig,
} = props.sync
const { showLogoutConfirm, logoutTargetLabel, confirmLogout, showCookieLogin, cookieLoginPlatform } = props.accounts
const { showResetLtIdentityConfirm, confirmResetLtIdentity } = props.network
const {
  showDownloadTemplateDialog, pendingTemplate, templatePreview, resetDownloadTemplate, applyDownloadTemplate,
} = props.downloads
</script>

<template>
  <!-- GitHub 两阶段配置对话框 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="showGitHubDialog" class="dialog-overlay" @click.self="showGitHubDialog = false">
      <div class="dialog-card" style="width: 420px">
        <h3 class="dialog-title">{{ t('settings.github_sync_config') }}</h3>

        <!-- Token 验证 -->
        <div class="phase-section">
          <div class="phase-header">
            <span class="phase-number" :class="{ done: githubPhase === 2 }">{{ githubPhase === 2 ? '✓' : '1' }}</span>
            <span class="phase-label">{{ t('settings.github_step1') }}</span>
          </div>

          <div v-if="githubPhase === 1" class="phase-body">
            <div class="dialog-field">
              <label>GitHub Personal Access Token</label>
              <input v-model="githubToken" type="password" placeholder="ghp_xxxxxxxxxxxx" @keyup.enter="githubValidateToken" />
            </div>
            <p class="field-hint">{{ t('settings.github_token_hint') }}</p>
            <button type="button" class="text-link-btn" @click="openGitHubTokenPage">
              <span class="material-symbols-rounded" style="font-size: 16px">open_in_new</span>
              {{ t('settings.github_create_token') }}
            </button>
          </div>
          <div v-else class="phase-done-info">
            <span class="material-symbols-rounded" style="font-size: 16px; color: var(--md-primary)">check_circle</span>
            <span>{{ githubUsername }}</span>
          </div>
        </div>

        <!-- 仓库选择 -->
        <div v-if="githubPhase === 2" class="phase-section">
          <div class="phase-header">
            <span class="phase-number">2</span>
            <span class="phase-label">{{ t('settings.github_step2') }}</span>
          </div>
          <div class="phase-body">
            <div class="radio-group">
              <label class="radio-option" :class="{ active: githubRepoMode === 'create' }" @click="githubRepoMode = 'create'">
                <span class="radio-dot" :class="{ checked: githubRepoMode === 'create' }"></span>
                {{ t('settings.github_create_repo') }}
              </label>
              <div v-if="githubRepoMode === 'create'" class="dialog-field" style="margin-left: 28px; margin-top: 8px">
                <input v-model="githubNewRepoName" type="text" placeholder="neriplayer-backup" />
              </div>

              <label class="radio-option" :class="{ active: githubRepoMode === 'existing' }" @click="githubRepoMode = 'existing'">
                <span class="radio-dot" :class="{ checked: githubRepoMode === 'existing' }"></span>
                {{ t('settings.github_use_existing') }}
              </label>
              <div v-if="githubRepoMode === 'existing'" class="dialog-field" style="margin-left: 28px; margin-top: 8px">
                <input v-model="githubExistingRepo" type="text" :placeholder="t('settings.github_repo_format_hint')" />
              </div>
            </div>
          </div>
        </div>

        <p v-if="syncStore.dialogError" class="dialog-error">{{ syncStore.dialogError }}</p>

        <div class="dialog-actions">
          <button class="dialog-btn" @click="showGitHubDialog = false">{{ t('settings.cancel') }}</button>
          <button v-if="githubPhase === 1" class="dialog-btn primary" :disabled="githubIsValidating || !githubToken.trim()" @click="githubValidateToken">
            <span v-if="githubIsValidating" class="material-symbols-rounded spinning" style="font-size: 16px">progress_activity</span>
            <span v-else>{{ t('settings.github_verify_token') }}</span>
          </button>
          <button v-else class="dialog-btn primary" :disabled="githubIsSettingRepo" @click="githubFinishSetup">
            <span v-if="githubIsSettingRepo" class="material-symbols-rounded spinning" style="font-size: 16px">progress_activity</span>
            <span v-else>{{ t('settings.github_done') }}</span>
          </button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>

  <!-- WebDAV 配置对话框 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="showWebDavDialog" class="dialog-overlay" @click.self="showWebDavDialog = false">
      <div class="dialog-card">
        <h3 class="dialog-title">{{ t('settings.webdav_sync') }}</h3>
        <div class="dialog-field">
          <label>{{ t('settings.webdav_server') }}</label>
          <input v-model="webdavUrl" type="url" placeholder="https://dav.example.com" />
        </div>
        <div class="dialog-field">
          <label>{{ t('settings.webdav_username') }}</label>
          <input v-model="webdavUsername" type="text" />
        </div>
        <div class="dialog-field">
          <label>{{ t('settings.webdav_password') }}</label>
          <input v-model="webdavPassword" type="password" />
        </div>
        <div class="dialog-field">
          <label>{{ t('settings.webdav_path') }}</label>
          <input v-model="webdavBasePath" type="text" placeholder="/neriplayer" />
        </div>
        <p v-if="syncStore.dialogError" class="dialog-error">{{ syncStore.dialogError }}</p>
        <div class="dialog-actions">
          <button class="dialog-btn" @click="showWebDavDialog = false">{{ t('settings.cancel') }}</button>
          <button class="dialog-btn primary" :disabled="webdavConfiguring || !webdavUrl.trim() || !webdavUsername.trim() || !webdavPassword" @click="configureWebDav">
            <span v-if="webdavConfiguring" class="material-symbols-rounded spinning" style="font-size: 16px">progress_activity</span>
            <span v-else>{{ t('settings.connect') }}</span>
          </button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>

  <!-- 退出登录确认对话框 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="showLogoutConfirm" class="dialog-overlay" @click.self="showLogoutConfirm = false">
      <div class="dialog-card" style="width: 340px">
        <h3 class="dialog-title">{{ t('settings.logout_confirm_title') }}</h3>
        <p class="dialog-desc">{{ t('settings.logout_confirm_msg', { platform: logoutTargetLabel }) }}</p>
        <div class="dialog-actions">
          <button class="dialog-btn" @click="showLogoutConfirm = false">{{ t('settings.cancel') }}</button>
          <button class="dialog-btn danger" @click="confirmLogout">{{ t('settings.sign_out') }}</button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>

  <!-- 清除 GitHub 配置确认 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="showClearGitHubConfirm" class="dialog-overlay" @click.self="showClearGitHubConfirm = false">
      <div class="dialog-card" style="width: 340px">
        <h3 class="dialog-title">{{ t('settings.clear_config_title') }}</h3>
        <p class="dialog-desc">{{ t('settings.clear_config_msg') }}</p>
        <div class="dialog-actions">
          <button class="dialog-btn" @click="showClearGitHubConfirm = false">{{ t('settings.cancel') }}</button>
          <button class="dialog-btn danger" @click="confirmClearGitHub">{{ t('settings.clear_config_confirm') }}</button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>

  <!-- 旧客户端无法读取升级后的归档，写入前需明确确认 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="syncStore.pendingProtocolUpgrade && !hideProtocolUpgrade" class="dialog-overlay" @click.self="hideProtocolUpgrade = true">
      <div class="dialog-card" style="width: 380px">
        <div class="dialog-icon warning">
          <span class="material-symbols-rounded">warning</span>
        </div>
        <h3 class="dialog-title">{{ t('settings.sync_upgrade_title') }}</h3>
        <p class="dialog-desc">{{ t('settings.sync_upgrade_message', { provider: syncStore.pendingProtocolUpgrade.backend === 'github' ? 'GitHub' : 'WebDAV' }) }}</p>
        <label class="dialog-check">
          <input v-model="upgradeDevicesConfirmed" type="checkbox" :disabled="syncStore.isSyncing" />
          <span>{{ t('settings.sync_upgrade_all_devices') }}</span>
        </label>
        <p v-if="syncStore.upgradeError" class="dialog-error">{{ syncStore.upgradeError }}</p>
        <div class="dialog-actions">
          <button class="dialog-btn" :disabled="syncStore.isSyncing" @click="hideProtocolUpgrade = true">{{ t('settings.cancel') }}</button>
          <button class="dialog-btn primary" :disabled="syncStore.isSyncing || !upgradeDevicesConfirmed" @click="syncStore.approveProtocolUpgrade(upgradeDevicesConfirmed)">{{ t('settings.sync_upgrade_confirm') }}</button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>

  <!-- 一起听身份重置确认 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="showResetLtIdentityConfirm" class="dialog-overlay" @click.self="showResetLtIdentityConfirm = false">
      <div class="dialog-card" style="width: 380px">
        <div class="dialog-icon warning">
          <span class="material-symbols-rounded">restart_alt</span>
        </div>
        <h3 class="dialog-title">{{ t('listen_together.reset_identity_confirm_title') }}</h3>
        <p class="dialog-desc">{{ t('listen_together.reset_identity_confirm_desc') }}</p>
        <div class="dialog-actions">
          <button class="dialog-btn" @click="showResetLtIdentityConfirm = false">{{ t('settings.cancel') }}</button>
          <button class="dialog-btn danger" @click="confirmResetLtIdentity">{{ t('listen_together.reset_btn') }}</button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>

  <CookieLoginDialog v-model:open="showCookieLogin" :platform="cookieLoginPlatform" />
  <ConfigTransferDialogs v-model:mode="configTransferMode" @imported="applyImportedConfig" />

  <!-- 下载文件名格式编辑对话框 -->
  <Teleport to="body">
    <Transition name="settings-dialog">
    <div v-if="showDownloadTemplateDialog" class="dialog-overlay" @click.self="showDownloadTemplateDialog = false">
      <div class="dialog-card" style="width: 420px">
        <h3 class="dialog-title">{{ t('settings.download_format') }}</h3>
        <p class="dialog-desc">{{ t('settings.download_format_desc') }}</p>
        <div class="dialog-field">
          <label>{{ t('settings.download_format_template') }}</label>
        <input v-model="pendingTemplate" type="text" :placeholder="DEFAULT_DOWNLOAD_NAME_TEMPLATE" />
        </div>
        <p class="field-hint">{{ t('settings.download_format_supported') }}</p>
        <div class="template-preview">
          <span class="preview-label">{{ t('settings.download_format_preview') }}</span>
          <span class="preview-value">{{ templatePreview }}</span>
        </div>
        <div class="dialog-actions">
          <button class="dialog-btn" @click="resetDownloadTemplate">{{ t('settings.download_format_reset') }}</button>
          <button class="dialog-btn" @click="showDownloadTemplateDialog = false">{{ t('settings.cancel') }}</button>
          <button class="dialog-btn primary" @click="applyDownloadTemplate">{{ t('settings.download_format_apply') }}</button>
        </div>
      </div>
    </div>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
@use '@/components/settings/settingsPrimitives.scss' as primitives;

@include primitives.spinning;

/* 对话框 */
.dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  backdrop-filter: blur(4px);
}

.dialog-card {
  background: var(--md-surface-container-high);
  border-radius: var(--radius-xl);
  padding: 24px;
  width: 380px;
  max-width: 90vw;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
  transform-origin: center;
}

/* 开合都走 transition：关闭有退场，半路重新打开从当前状态反向 */
.settings-dialog-enter-active,
.settings-dialog-leave-active {
  transition: opacity 200ms ease;
  .dialog-card { transition: opacity 200ms ease, transform 250ms cubic-bezier(0.05, 0.7, 0.1, 1); }
}
.settings-dialog-leave-active { transition-duration: 150ms; }
.settings-dialog-leave-active .dialog-card { transition-duration: 150ms; transition-timing-function: var(--ease-accelerate); }
.settings-dialog-enter-from,
.settings-dialog-leave-to {
  opacity: 0;
  .dialog-card { opacity: 0; transform: scale(0.92); }
}

.dialog-title {
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 20px;
}

.dialog-icon {
  width: 42px;
  height: 42px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 14px;
  border-radius: 50%;
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);

  &.warning {
    background: color-mix(in srgb, var(--md-tertiary, #7d5260) 16%, transparent);
    color: var(--md-tertiary, #7d5260);
  }

  &.danger {
    background: color-mix(in srgb, var(--md-error) 16%, transparent);
    color: var(--md-error);
  }
}

.config-warning-dialog {
  width: 430px;
}

.warning-list {
  display: flex;
  flex-direction: column;
  gap: 9px;
  margin-top: 16px;
  padding: 12px 14px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--md-error) 7%, var(--md-surface-container));
  color: var(--md-on-surface-variant);
  font-size: 12px;

  div {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .material-symbols-rounded {
    color: var(--md-error);
    font-size: 17px;
  }
}

.dialog-field {
  margin-bottom: 14px;

  label {
    display: block;
    font-size: 12px;
    font-weight: 500;
    color: var(--md-on-surface-variant);
    margin-bottom: 6px;
  }

  input {
    width: 100%;
    padding: 10px 14px;
    border-radius: var(--radius-md);
    border: 1px solid var(--md-outline-variant);
    background: var(--md-surface-container);
    color: var(--md-on-surface);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color var(--duration-short);

    &:focus {
      border-color: var(--md-primary);
    }

    &::placeholder {
      color: var(--md-on-surface-variant);
      opacity: 0.5;
    }
  }
}

.dialog-error {
  font-size: 12px;
  color: var(--md-error);
  margin-bottom: 12px;
}

.dialog-check {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 12px 0;
  font-size: 13px;
  line-height: 1.4;
  color: var(--md-on-surface);
  cursor: pointer;

  input {
    margin-top: 2px;
    accent-color: var(--md-primary);
  }
}

.dialog-desc {
  font-size: 13px;
  color: var(--md-on-surface-variant);
  line-height: 1.5;
  margin-bottom: 4px;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 20px;
}

.dialog-btn {
  padding: 8px 20px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background var(--duration-short);
  color: var(--md-on-surface);
  display: flex;
  align-items: center;
  gap: 6px;

  &:hover { background: var(--md-surface-container-highest); }

  &.primary {
    background: var(--md-primary);
    color: var(--md-on-primary);

    &:hover { opacity: 0.9; }
    &:disabled { opacity: 0.5; cursor: not-allowed; }
  }

  &.danger {
    background: var(--md-error);
    color: var(--md-on-error, #fff);

    &:hover { opacity: 0.9; }
  }
}

/* 两阶段引导样式 */
.phase-section {
  margin-bottom: 18px;
  padding: 14px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
}

.phase-header {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}

.phase-number {
  width: 24px;
  height: 24px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  font-size: 12px;
  font-weight: 600;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;

  &.done {
    background: color-mix(in srgb, var(--md-primary) 20%, transparent);
    color: var(--md-primary);
  }
}

.phase-label {
  font-size: 14px;
  font-weight: 600;
}

.phase-body {
  padding-left: 34px;
}

.phase-done-info {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-left: 34px;
  font-size: 13px;
  color: var(--md-on-surface-variant);
}

.field-hint {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
  margin: 4px 0 8px;
}

.text-link-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 500;
  color: var(--md-primary);
  cursor: pointer;
  padding: 4px 0;
  transition: opacity var(--duration-short);

  &:hover { opacity: 0.8; }
}

.radio-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.radio-option {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
  cursor: pointer;
  padding: 6px 0;
  color: var(--md-on-surface);

  &.active { font-weight: 500; }
}

.radio-dot {
  width: 18px;
  height: 18px;
  border-radius: var(--radius-full);
  border: 2px solid var(--md-outline);
  position: relative;
  flex-shrink: 0;
  transition: border-color var(--duration-short);

  &.checked {
    border-color: var(--md-primary);

    &::after {
      content: '';
      position: absolute;
      inset: 3px;
      border-radius: var(--radius-full);
      background: var(--md-primary);
    }
  }
}

.template-preview {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 14px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
  margin-bottom: 12px;

  .preview-label {
    font-size: 11px;
    color: var(--md-on-surface-variant);
    opacity: 0.7;
  }
  .preview-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--md-on-surface);
    word-break: break-all;
  }
}
</style>
