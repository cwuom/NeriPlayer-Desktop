<script setup lang="ts">
import { ref, onBeforeUnmount, onMounted } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { SUPPORTED_LOCALES } from '@/i18n'
import { invoke, isTauri } from '@tauri-apps/api/core'
import {
  COVER_BLUR_PX_PER_UNIT,
  UI_SCALE_DEFAULT,
  UI_SCALE_MIN,
  UI_SCALE_STEP,
  normalizeUiScale,
  MAX_COVER_BLUR_AMOUNT,
  MAX_MEDIA_CACHE_SIZE_MB,
  MIN_MEDIA_CACHE_SIZE_MB,
  MAX_DOWNLOAD_PARALLELISM,
  MIN_DOWNLOAD_PARALLELISM,
  ENHANCED_BLUR_RADIUS_MAX,
  ENHANCED_BLUR_RADIUS_MIN,
  ENHANCED_BLUR_RADIUS_STEP,
  useSettingsStore,
} from '@/stores/settings'
import { useAuthStore } from '@/stores/auth'
import { useSyncStore } from '@/stores/sync'
import { useDownloadStore } from '@/stores/download'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { LT_NICKNAME_MAX_LENGTH } from '@/stores/listenTogether/protocol'
import { usePlayerStore } from '@/stores/player'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import StorageManagementDialog from '@/components/StorageManagementDialog.vue'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import ShortcutSettings from '@/components/settings/ShortcutSettings.vue'
import FontFamilyControl from '@/components/settings/FontFamilyControl.vue'
import AudioEffectsSettings from '@/components/settings/AudioEffectsSettings.vue'
import SettingsSidebar from '@/components/settings/SettingsSidebar.vue'
import LyricsSettingsPanels from '@/components/settings/LyricsSettingsPanels.vue'
import SettingsDialogs from '@/components/settings/SettingsDialogs.vue'
import {
  onExpandAfterEnter, onExpandAfterLeave, onExpandEnter, onExpandLeave, settingsLog as log, type SettingsIpc,
} from '@/components/settings/settingsShared'
import { useSettingsSections } from '@/components/settings/useSettingsSections'
import { useSettingsSearch } from '@/components/settings/useSettingsSearch'
import { audioDisplayOptions, useAppearanceSettings } from '@/components/settings/useAppearanceSettings'
import { usePlaybackSettings } from '@/components/settings/usePlaybackSettings'
import { useDownloadSettings } from '@/components/settings/useDownloadSettings'
import { useStorageSettings } from '@/components/settings/useStorageSettings'
import { useNetworkSettings } from '@/components/settings/useNetworkSettings'
import { useAccountSettings } from '@/components/settings/useAccountSettings'
import { useLyricsSettings } from '@/components/settings/useLyricsSettings'
import { FFMPEG_LEGAL_URL, PROJECT_REPOSITORY_URL, useAboutSettings } from '@/components/settings/useAboutSettings'
import { useSyncSettings } from '@/components/settings/useSyncSettings'
import { uiScaleSupported, uiScaleError, uiScaleMax } from '@/modules/settings/uiScale'
import { useEscapeClose } from '@/composables/useEscapeClose'

const { t, locale } = useI18n()
const settings = useSettingsStore()
const auth = useAuthStore()
const syncStore = useSyncStore()
const downloadStore = useDownloadStore()
const player = usePlayerStore()
const lt = useListenTogetherStore()
const {
  darkMode, coverStyle, appFontFamily, lyricFontFamily, uiScale,
  defaultScreen, closeToTray, showCoverBadge, showNowPlayingTitle, showToolbarDock, showQualitySwitch,
  normalizeVolume, multichannelDrc, volumeBalance, audioOutputDevice,
  fadeIn, fadeInDuration, fadeOutDuration,
  crossfadeNext, crossfadeInDuration, crossfadeOutDuration,
  keepProgress, rememberLongFormProgress, keepPlaybackMode, doubleClickPlay,
  advancedLyrics, dynamicBackground, colorMode, audioReactive,
  lowPerformanceMode, coverBlurBg, coverBlurAmount, coverBlurDarken,
  neteaseQuality, youtubeQuality, biliQuality,
  youtubePlaybackSource, neteaseAutoSourceSwitch, neteaseLocalSourceFallback,
  bypassProxy, internationalizationEnabled, exploreSearchHistoryEnabled,
  backgroundImageUri, backgroundImageBlur, backgroundImageAlpha, backgroundImageDim,
  enhancedAdvancedBlur, enhancedAdvancedBlurRadius,
  devModeEnabled, logToFile, logLevel,
  maxCacheSize, downloadNameTemplate, downloadDir,
  downloadParallelism, downloadAutoFillMetadata, downloadEmbedLyrics,
  downloadFollowPlaybackQuality, downloadNeteaseQuality,
  downloadYoutubeQuality, downloadBiliQuality,
  ltServerUrl, ltNickname, ltAllowMemberControl, ltAutoPauseOnMemberChange, ltShareAudioLinks,
} = storeToRefs(settings)

// 设置页的后端命令集中在此（受 ui_invoke_baseline 约束），以端口形式注入各分区 composable
const settingsIpc: SettingsIpc = {
  listAudioOutputDevices: () => invoke('list_audio_output_devices'),
  setAudioOutputDevice: name => invoke('set_audio_output_device', { name }),
  getLogDir: () => invoke<string>('get_log_dir'),
  revealInFileManager: path => invoke('reveal_in_file_manager', { path }),
  setBypassProxy: bypass => invoke('set_bypass_proxy', { bypass }),
  getDefaultDownloadDir: () => invoke<string>('get_default_download_dir'),
  setDownloadDir: path => invoke<string>('set_download_dir', { path }),
  getStorageUsage: dir => invoke('get_storage_usage', { downloadDir: dir }),
  clearStorageCache: (options, dir) => invoke('clear_storage_cache', { options, downloadDir: dir }),
  probePlatformConnectivity: platform => invoke<boolean>('probe_platform_connectivity', { platform }),
  importBackgroundImage: source => invoke<string>('import_background_image', { source }),
  clearBackgroundImages: () => invoke('clear_background_images'),
  getBuildInfo: () => invoke('get_build_info'),
}

const settingsRootRef = ref<HTMLElement | null>(null)

// 调用顺序即侦听器创建顺序，与拆分前保持一致：外观（主题色、动态背景互斥）→ 分区路由 → 同步协议升级
const {
  uiScaleOptions, presetColors, activeColorKey, dynamicBackgroundDialogOpen, onDynamicBackgroundToggle,
  confirmDynamicBackgroundWarning, cancelDynamicBackgroundWarning, handleColorSwitch, darkModeOptions,
  darkModeThumbStyle, colorModeOptions, handleDarkModeSwitch, handleLocaleSwitch, defaultScreenOptions,
  coverStyleOptions, selectBackgroundImage, clearBackgroundImage,
} = useAppearanceSettings(settingsIpc)

const sections = useSettingsSections()
const {
  activeSettingsSection, settingsContentRef, settingsNavGroups, activeSettingsItem, persistSettingsUiState,
  rememberSettingsScrollPosition, restoreSettingsScrollPosition, selectSettingsSection, toggleSection, isExpanded,
  applyRouteSection,
} = sections

const {
  settingsQuery, settingsSearchResults, openSettingsSearchResult, audioEffectsSettingsRef, onPanelEnter,
} = useSettingsSearch(sections)

const {
  youtubePlaybackSourceOptions, youtubePlaybackSourceDescription, changeYouTubePlaybackSource,
  audioOutputOptions, audioOutputSwitching, loadAudioOutputDevices, changeAudioOutputDevice, volumeBalanceLabel,
  neteaseQualityOptions, downloadNeteaseQualityOptions, youtubeQualityOptions, biliQualityOptions,
  qualitySwitching, handleQualityChange,
} = usePlaybackSettings(settingsIpc)

const downloadSettings = useDownloadSettings(settingsIpc)
const {
  activeDownloadCount, completedDownloadCount, activeDownloadTasks, activeDownloadProgressText,
  loadDefaultDownloadDir, displayDownloadDir, selectDownloadDir, resetDownloadDir, showDownloadTemplateDialog,
  openDownloadTemplateDialog, cancelAllDownloads, cancelDownload, goToDownloads,
} = downloadSettings

const {
  logLevelOptions, openLogDir, onLogToFileChange, showStorageManagement, storageLoading, storageClearing,
  storageSummary, openStorageManagement, clearStorageCache,
} = useStorageSettings(settingsIpc)

const networkSettings = useNetworkSettings(settingsIpc)
const {
  handleBypassProxyChange, intlChecking, handleIntlToggle, showResetLtIdentityConfirm, handleLtNicknameChange,
} = networkSettings

const accountSettings = useAccountSettings()
const {
  isAvatarLoadFailed, handleAvatarError, platformAccounts, showLogoutConfirm, showCookieLogin, cookieLoginPlatform,
  requestLogout,
} = accountSettings

const lyricsSettings = useLyricsSettings()

const {
  handleVersionTap, disposeVersionTap, buildInfo, loadBuildInfo, copyBuildValue, ffmpegComponentText, openExternalUrl,
} = useAboutSettings(settingsIpc, toggleSection)

const syncSettings = useSyncSettings(openExternalUrl)
const {
  syncFrequencyOptions, configTransferMode, showGitHubDialog, openGitHubSetup, showWebDavDialog, formatSyncTime,
  showClearGitHubConfirm, hideProtocolUpgrade,
} = syncSettings

// 启动时检查登录状态
onMounted(() => {
  auth.checkStatus()
  syncStore.loadConfigs()
  void downloadStore.initEvents().catch(error => log.error('Download listener failed:', error))
  downloadStore.loadDownloads()
  // 加载构建信息
  loadBuildInfo()
  // 加载默认下载目录
  loadDefaultDownloadDir()
  void loadAudioOutputDevices()
  void restoreSettingsScrollPosition(activeSettingsSection.value)
  lyricsSettings.refreshDesktopLyricsAccent()
  applyRouteSection()
})

onBeforeUnmount(() => {
  rememberSettingsScrollPosition()
  persistSettingsUiState()
  // 开发者模式 7-tap 计时器兜底清理
  disposeVersionTap()
})

// 页内旧式对话框（.dialog-overlay）同样响应 Escape，并与 M3Dialog 共用弹层栈
for (const dialog of [
  showGitHubDialog,
  showWebDavDialog,
  showLogoutConfirm,
  showClearGitHubConfirm,
  showResetLtIdentityConfirm,
  showDownloadTemplateDialog,
]) {
  useEscapeClose(() => dialog.value, () => { dialog.value = false })
}
useEscapeClose(
  () => !!syncStore.pendingProtocolUpgrade && !hideProtocolUpgrade.value,
  () => { hideProtocolUpgrade.value = true },
)
</script>

<template>
  <div ref="settingsRootRef" class="settings-view">
    <div class="settings-layout">
      <SettingsSidebar
        v-model:settings-query="settingsQuery"
        :settings-search-results="settingsSearchResults"
        :settings-nav-groups="settingsNavGroups"
        :active-settings-section="activeSettingsSection"
        @select="selectSettingsSection"
        @open-result="openSettingsSearchResult"
      />

      <main class="settings-content">
        <header class="settings-content-header">
          <div>
            <h2>{{ activeSettingsItem?.label }}</h2>
            <p>{{ activeSettingsItem?.description }}</p>
          </div>
        </header>

        <!-- 头部固定不滚动，仅面板区滚动；切换分组时以分组 id 为 key 做 out-in 交叉淡入 -->
        <div ref="settingsContentRef" class="settings-content-scroll">
        <Transition name="fade" mode="out-in" @enter="onPanelEnter">
        <div :key="activeSettingsSection" class="settings-panels">

        <div v-show="activeSettingsSection === 'accounts'" class="settings-section-panel" data-section="accounts">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">account_circle</span>
      <span>{{ t('settings.accounts') }}</span>
    </div>
    <p class="settings-section-intro">{{ t('settings.accounts_desc') }}</p>

    <div
      v-for="account in platformAccounts"
      :key="account.key"
      class="setting-card account-card"
    >
      <div class="setting-icon-wrap">
        <span class="platform-icon" :style="{ maskImage: `url(${account.iconSvg})` }"></span>
      </div>
      <div class="setting-info">
        <div class="setting-title">{{ account.label }}</div>
        <div class="setting-desc" v-if="account.auth.loggedIn">
          {{ t('settings.signed_in_as', { name: account.auth.nickname || '—' }) }}
        </div>
        <div class="setting-desc" v-else-if="auth.loggingIn === account.key">
          {{ t('settings.signing_in') }}
        </div>
        <div class="account-status" :class="{ connected: account.auth.loggedIn }">
          <span class="account-status-dot"></span>
          {{ account.auth.loggedIn ? t('settings.account_connected') : t('settings.account_not_connected') }}
        </div>
      </div>
      <template v-if="account.auth.loggedIn">
        <BilibiliCoverImage
          v-if="account.key === 'bilibili' && account.auth.avatarUrl"
          :src="account.auth.avatarUrl"
          class="account-avatar"
        >
          <span class="material-symbols-rounded account-avatar account-avatar-fallback">account_circle</span>
        </BilibiliCoverImage>
        <img
          v-else-if="account.auth.avatarUrl && !isAvatarLoadFailed(account.key, account.auth.avatarUrl)"
          :src="account.auth.avatarUrl"
          class="account-avatar"
          referrerpolicy="no-referrer"
          @error="handleAvatarError(account.key, account.auth.avatarUrl)"
        />
        <span v-else class="material-symbols-rounded account-avatar account-avatar-fallback">account_circle</span>
        <button class="account-logout-btn" @click="requestLogout(account.key, account.label)">
          <span class="material-symbols-rounded" style="font-size: 16px">logout</span>
          {{ t('settings.sign_out') }}
        </button>
      </template>
      <template v-else>
        <button
          class="account-login-btn"
          :disabled="auth.loggingIn === account.key"
          @click="account.login()"
        >
          <span v-if="auth.loggingIn === account.key" class="material-symbols-rounded spin" style="font-size: 18px">progress_activity</span>
          <span v-else>{{ t('settings.sign_in') }}</span>
        </button>
      </template>
      <button
        class="account-login-btn"
        :disabled="auth.loggingIn !== null"
        @click="cookieLoginPlatform = account.key; showCookieLogin = true"
      >{{ t('settings.cookie_login') }}</button>
    </div>
        </div>

        <div v-show="activeSettingsSection === 'general'" class="settings-section-panel" data-section="general">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">home</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.default_screen') }}</div>
          <div class="setting-desc">{{ defaultScreenOptions.find(o => o.value === defaultScreen)?.label }}</div>
        </div>
        <div class="chip-row">
          <button v-for="o in defaultScreenOptions" :key="o.value" class="m3-chip" :class="{ active: defaultScreen === o.value }" @click="defaultScreen = o.value">{{ o.label }}</button>
        </div>
      </div>
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tab_inactive</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.close_to_tray') }}</div>
          <div class="setting-desc">{{ t('settings.close_to_tray_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="closeToTray" /><span class="track"><span class="thumb"><span v-if="closeToTray" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

    <!-- YouTube 国际化 -->
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">language</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.internationalization') }}</div>
        <div class="setting-desc">
          <template v-if="intlChecking">{{ t('settings.intl_checking') }}</template>
          <template v-else>{{ t('settings.internationalization_desc') }}</template>
        </div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" :checked="internationalizationEnabled" :disabled="intlChecking" @change="handleIntlToggle(($event.target as HTMLInputElement).checked)" />
        <span class="track"><span class="thumb">
          <span v-if="intlChecking" class="material-symbols-rounded spinning" style="font-size: 14px">progress_activity</span>
          <span v-else-if="internationalizationEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span>
        </span></span>
      </label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">manage_search</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.explore_search_history') }}</div>
        <div class="setting-desc">{{ t('settings.explore_search_history_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="exploreSearchHistoryEnabled" /><span class="track"><span class="thumb"><span v-if="exploreSearchHistoryEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

        </div>

    <!-- 外观 -->
        <div v-show="activeSettingsSection === 'appearance'" class="settings-section-panel" data-section="appearance">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">palette</span>
      <span>{{ t('settings.appearance') }}</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap">
        <span class="material-symbols-rounded filled">dark_mode</span>
      </div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.dark_mode') }}</div>
        <div class="setting-desc">{{ darkModeOptions.find(o => o.value === darkMode)?.label }}</div>
      </div>
      <div class="dark-mode-pills">
        <span
          class="pill-thumb"
          :style="darkModeThumbStyle"
          aria-hidden="true"
        />
        <button
          v-for="opt in darkModeOptions"
          :key="opt.value"
          class="pill"
          :class="{ active: darkMode === opt.value }"
          @click="handleDarkModeSwitch(opt.value, $event)"
        >
          <span class="material-symbols-rounded" style="font-size: 16px">{{ opt.icon }}</span>
        </button>
      </div>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap">
        <span class="material-symbols-rounded filled">format_paint</span>
      </div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.theme_color') }}</div>
        <div class="color-row" :style="colorMode !== 'default' ? { opacity: 0.4, pointerEvents: 'none' } : undefined">
          <button
            v-for="c in presetColors" :key="c.key"
            class="color-dot"
            :class="{ selected: activeColorKey === c.key }"
            :style="{ background: c.color }"
            @click="handleColorSwitch(c.key, $event)"
          >
            <span v-if="activeColorKey === c.key" class="material-symbols-rounded" style="font-size: 16px; color: white">check</span>
          </button>
        </div>
      </div>
    </div>

    <div class="setting-card ui-scale-card">
      <div class="ui-scale-heading">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">display_settings</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.ui_scale') }}</div>
          <div class="setting-desc">{{ t('settings.ui_scale_desc') }}</div>
          <div v-if="uiScaleSupported" class="setting-desc">{{ t('settings.ui_scale_limit', { value: uiScaleMax }) }}</div>
          <div v-if="uiScaleError" class="setting-desc" role="alert">{{ t('settings.ui_scale_failed') }}</div>
          <div v-else-if="!uiScaleSupported" class="setting-desc">{{ t(isTauri() ? 'settings.ui_scale_unsupported' : 'settings.ui_scale_preview') }}</div>
        </div>
      </div>
      <div class="ui-scale-controls">
        <input
          type="range" class="m3-slider" :value="uiScale"
          :min="UI_SCALE_MIN" :max="uiScaleMax" :step="UI_SCALE_STEP"
          :disabled="!uiScaleSupported" :aria-label="t('settings.ui_scale')"
          @change="uiScale = normalizeUiScale(Number(($event.target as HTMLInputElement).value))"
        />
        <CustomSelect
          :model-value="String(uiScale)"
          :options="uiScaleOptions"
          :label="t('settings.ui_scale')" :disabled="!uiScaleSupported"
          @update:model-value="uiScale = normalizeUiScale(Number($event))"
        />
        <button class="m3-chip sm" :disabled="!uiScaleSupported || uiScale === UI_SCALE_DEFAULT" @click="uiScale = UI_SCALE_DEFAULT">{{ t('settings.ui_scale_reset') }}</button>
      </div>
    </div>

    <div class="setting-card font-setting-card">
      <div class="font-setting-heading">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">font_download</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.app_font') }}</div>
          <div class="setting-desc">{{ t('settings.app_font_desc') }}</div>
        </div>
      </div>
      <FontFamilyControl v-model="appFontFamily" :label="t('settings.app_font')" :preview-text="t('settings.font_preview')" />
    </div>

    <div class="setting-card font-setting-card">
      <div class="font-setting-heading">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">lyrics</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.lyric_font') }}</div>
          <div class="setting-desc">{{ t('settings.lyric_font_desc') }}</div>
        </div>
      </div>
      <FontFamilyControl v-model="lyricFontFamily" :label="t('settings.lyric_font')" :preview-text="t('settings.font_preview')" :preview-weight="850" />
    </div>

    <!-- 取色方式 -->
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">colorize</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.color_mode') }}</div>
        <div class="setting-desc">{{ colorModeOptions.find(o => o.value === colorMode)?.desc }}</div>
      </div>
      <div class="chip-row" role="radiogroup" :aria-label="t('settings.color_mode')">
        <button
          v-for="o in colorModeOptions"
          :key="o.value"
          class="m3-chip"
          :class="{ active: colorMode === o.value }"
          role="radio"
          :aria-checked="colorMode === o.value"
          @click="colorMode = o.value"
        >{{ o.label }}</button>
      </div>
    </div>

        </div>

    <!-- 播放页 -->
        <div v-show="activeSettingsSection === 'now_playing'" class="settings-section-panel" data-section="now_playing">
      <!-- 封面样式 -->
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">album</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_style') }}</div>
        </div>
        <div class="chip-row">
          <button v-for="o in coverStyleOptions" :key="o.value" class="m3-chip" :class="{ active: coverStyle === o.value }" @click="coverStyle = o.value">{{ o.label }}</button>
        </div>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">badge</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_badge') }}</div>
          <div class="setting-desc">{{ t('settings.cover_badge_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showCoverBadge" /><span class="track"><span class="thumb"><span v-if="showCoverBadge" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">title</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.np_title') }}</div>
          <div class="setting-desc">{{ t('settings.np_title_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showNowPlayingTitle" /><span class="track"><span class="thumb"><span v-if="showNowPlayingTitle" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">dock_to_bottom</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.np_toolbar') }}</div>
          <div class="setting-desc">{{ t('settings.np_toolbar_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showToolbarDock" /><span class="track"><span class="thumb"><span v-if="showToolbarDock" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">high_quality</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.quality_switch') }}</div>
          <div class="setting-desc">{{ t('settings.quality_switch_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="showQualitySwitch" /><span class="track"><span class="thumb"><span v-if="showQualitySwitch" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div v-for="option in audioDisplayOptions" :key="option.key" class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">{{ option.icon }}</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.' + option.label) }}</div>
          <div class="setting-desc">{{ t('settings.' + option.label + '_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="settings[option.key]" /><span class="track"><span class="thumb"><span v-if="settings[option.key]" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

        </div>

    <!-- 背景 -->
        <div v-show="activeSettingsSection === 'background'" class="settings-section-panel" data-section="background">
      <!-- 自定义背景图 -->
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">wallpaper</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.background_image') }}</div>
          <div class="setting-desc">{{ backgroundImageUri ? backgroundImageUri.split(/[\\/]/).pop() : t('settings.background_image_desc') }}</div>
        </div>
        <div class="chip-row">
          <button class="m3-chip sm" @click="selectBackgroundImage">{{ t('settings.select_image') }}</button>
          <button v-if="backgroundImageUri" class="m3-chip sm" @click="clearBackgroundImage">{{ t('settings.clear_image') }}</button>
        </div>
      </div>

      <template v-if="backgroundImageUri">
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bg_blur') }}</div>
          <EditableRangeValue
            v-model="backgroundImageBlur"
            class="setting-desc"
            :min="0"
            :max="100"
            :step="5"
            :display-value="`${backgroundImageBlur}px`"
            input-suffix="px"
            :aria-label="t('settings.bg_blur')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="backgroundImageBlur" min="0" max="100" step="5" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bg_opacity') }}</div>
          <EditableRangeValue
            v-model="backgroundImageAlpha"
            class="setting-desc"
            :min="0"
            :max="1"
            :step="0.05"
            :inputScale="100"
            :display-value="`${(backgroundImageAlpha * 100).toFixed(0)}%`"
            input-suffix="%"
            :aria-label="t('settings.bg_opacity')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="backgroundImageAlpha" min="0" max="1" step="0.05" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bg_dim') }}</div>
          <EditableRangeValue
            v-model="backgroundImageDim"
            class="setting-desc"
            :min="0"
            :max="0.8"
            :step="0.05"
            :inputScale="100"
            :display-value="`${(backgroundImageDim * 100).toFixed(0)}%`"
            input-suffix="%"
            :aria-label="t('settings.bg_dim')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="backgroundImageDim" min="0" max="0.8" step="0.05" />
        </div>

        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.enhanced_advanced_blur') }}</div>
            <div class="setting-desc">{{ t('settings.enhanced_advanced_blur_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" v-model="enhancedAdvancedBlur" /><span class="track"><span class="thumb"><span v-if="enhancedAdvancedBlur" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
        </div>

        <div v-if="enhancedAdvancedBlur" class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.enhanced_advanced_blur_radius') }}</div>
            <EditableRangeValue
              v-model="enhancedAdvancedBlurRadius"
              class="setting-desc"
              :min="ENHANCED_BLUR_RADIUS_MIN"
              :max="ENHANCED_BLUR_RADIUS_MAX"
              :step="ENHANCED_BLUR_RADIUS_STEP"
              :display-value="`${enhancedAdvancedBlurRadius}px`"
              input-suffix="px"
              :aria-label="t('settings.enhanced_advanced_blur_radius')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="enhancedAdvancedBlurRadius"
            :min="ENHANCED_BLUR_RADIUS_MIN" :max="ENHANCED_BLUR_RADIUS_MAX" :step="ENHANCED_BLUR_RADIUS_STEP" />
        </div>
      </template>
        </div>

    <!-- 快捷键 -->
        <div v-show="activeSettingsSection === 'shortcuts'" class="settings-section-panel" data-section="shortcuts">
          <ShortcutSettings />
        </div>

    <!-- 播放 -->
        <div v-show="activeSettingsSection === 'audio'" class="settings-section-panel" data-section="audio">
    <div class="setting-card setting-card--select">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">speaker</span></div>
      <div class="setting-info">
        <label class="setting-title" for="audio-output-device">{{ t('settings.audio_output') }}</label>
        <div class="setting-desc">{{ t('settings.audio_output_desc') }}</div>
      </div>
      <CustomSelect id="audio-output-device" class="settings-select settings-select--device"
        :model-value="audioOutputDevice" :options="audioOutputOptions" :label="t('settings.audio_output')"
        :disabled="audioOutputSwitching" @open="loadAudioOutputDevices" @update:model-value="changeAudioOutputDevice" />
    </div>
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.normalize') }}</div>
        <div class="setting-desc">{{ t('settings.normalize_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="normalizeVolume" /><span class="track"><span class="thumb"><span v-if="normalizeVolume" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">balance</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.volume_balance') }}</div>
          <EditableRangeValue
            v-model="volumeBalance"
            class="setting-desc"
            :min="-1"
            :max="1"
            :step="0.01"
            :input-scale="100"
            :display-value="volumeBalanceLabel"
            input-suffix="%"
            :aria-label="t('settings.volume_balance')"
          />
        </div>
        <input type="range" class="m3-slider" v-model.number="volumeBalance" min="-1" max="1" step="0.05" :aria-label="t('settings.volume_balance')" :aria-valuetext="volumeBalanceLabel" />
      </div>
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">surround_sound</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.multichannel_drc') }}</div>
          <div class="setting-desc">{{ t('settings.multichannel_drc_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="multichannelDrc" /><span class="track"><span class="thumb"><span v-if="multichannelDrc" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>
        </div>

    <!-- 详细音效 -->
        <div v-show="activeSettingsSection === 'audio_effects'" class="settings-section-panel" data-section="audio_effects">
          <AudioEffectsSettings v-if="activeSettingsSection === 'audio_effects'" ref="audioEffectsSettingsRef" />
        </div>

    <!-- 播放 -->
        <div v-show="activeSettingsSection === 'playback'" class="settings-section-panel" data-section="playback">
    <div class="section-label clickable" @click="toggleSection('playback')">
      <span class="material-symbols-rounded" style="font-size: 18px">play_circle</span>
      <span>{{ t('settings.playback') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('playback') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('playback')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">volume_up</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.fade_in') }}</div>
          <div class="setting-desc">{{ t('settings.fade_in_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="fadeIn" /><span class="track"><span class="thumb"><span v-if="fadeIn" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="fadeIn">
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.fade_in_duration') }}</div>
          <EditableRangeValue
            v-model="fadeInDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${fadeInDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.fade_in_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="fadeInDuration" min="0" max="3000" step="100" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.fade_out_duration') }}</div>
          <EditableRangeValue
            v-model="fadeOutDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${fadeOutDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.fade_out_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="fadeOutDuration" min="0" max="3000" step="100" />
        </div>
      </template>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync_alt</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.crossfade_next') }}</div>
          <div class="setting-desc">{{ t('settings.crossfade_next_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="crossfadeNext" /><span class="track"><span class="thumb"><span v-if="crossfadeNext" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="crossfadeNext">
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.crossfade_in_duration') }}</div>
          <EditableRangeValue
            v-model="crossfadeInDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${crossfadeInDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.crossfade_in_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="crossfadeInDuration" min="0" max="3000" step="100" />
        </div>
        <div class="setting-card sub-card">
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.crossfade_out_duration') }}</div>
          <EditableRangeValue
            v-model="crossfadeOutDuration"
            class="setting-desc"
            :min="0"
            :max="3000"
            :step="100"
            :display-value="`${crossfadeOutDuration}ms`"
            input-suffix="ms"
            :aria-label="t('settings.crossfade_out_duration')"
          />
        </div>
          <input type="range" class="m3-slider" v-model.number="crossfadeOutDuration" min="0" max="3000" step="100" />
        </div>
      </template>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">ads_click</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.double_click_play') }}</div>
          <div class="setting-desc">{{ t('settings.double_click_play_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="doubleClickPlay" /><span class="track"><span class="thumb"><span v-if="doubleClickPlay" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">history</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.keep_progress') }}</div>
          <div class="setting-desc">{{ t('settings.keep_progress_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="keepProgress" /><span class="track"><span class="thumb"><span v-if="keepProgress" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">bookmark</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.remember_long_form_progress') }}</div>
          <div class="setting-desc">{{ t('settings.remember_long_form_progress_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="rememberLongFormProgress" /><span class="track"><span class="thumb"><span v-if="rememberLongFormProgress" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">repeat</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.keep_mode') }}</div>
          <div class="setting-desc">{{ t('settings.keep_mode_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="keepPlaybackMode" /><span class="track"><span class="thumb"><span v-if="keepPlaybackMode" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>
    </div></Transition>
        </div>

    <!-- 网络 -->
        <div v-show="activeSettingsSection === 'network'" class="settings-section-panel" data-section="network">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">wifi</span>
      <span>{{ t('settings.network') }}</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">vpn_lock</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.bypass_proxy') }}</div>
        <div class="setting-desc">{{ t('settings.bypass_proxy_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" :checked="bypassProxy" @change="handleBypassProxyChange(($event.target as HTMLInputElement).checked)" />
        <span class="track"><span class="thumb"><span v-if="bypassProxy" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>
        </div>

    <!-- 一起听 -->
        <div v-show="activeSettingsSection === 'listen_together'" class="settings-section-panel" data-section="listen_together">
    <div class="section-label clickable" @click="toggleSection('listen_together')">
      <span class="material-symbols-rounded" style="font-size: 18px">group</span>
      <span>{{ t('listen_together.title') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('listen_together') }">expand_more</span>
    </div>

    <!-- 摘要卡与展开编辑区互斥，整卡可点击展开 -->
    <div
      v-if="!isExpanded('listen_together')"
      class="setting-card"
      style="cursor: pointer"
      @click="toggleSection('listen_together')"
    >
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">dns</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('listen_together.server_url') }}</div>
        <div class="setting-desc" style="word-break: break-all">{{ ltServerUrl || 'https://neriplayer.hancat.work' }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; color: var(--md-on-surface-variant)">edit</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('listen_together')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">edit</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.server_url') }}</div>
        </div>
        <input
          type="text"
          class="lt-url-input"
          :value="ltServerUrl"
          @change="ltServerUrl = ($event.target as HTMLInputElement).value.trim() || 'https://neriplayer.hancat.work'"
        />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">badge</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.nickname') }}</div>
        </div>
        <input
          type="text"
          class="lt-url-input lt-input-left"
          style="width: 140px"
          :value="ltNickname"
          @change="handleLtNicknameChange"
          :maxlength="LT_NICKNAME_MAX_LENGTH"
          :placeholder="t('listen_together.nickname_placeholder')"
          :aria-label="t('listen_together.nickname')"
        />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tune</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.allow_member_control') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" :checked="ltAllowMemberControl" @change="lt.updateRoomSettings({ allowMemberControl: ($event.target as HTMLInputElement).checked })" />
          <span class="track"><span class="thumb"><span v-if="ltAllowMemberControl" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">pause_circle</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.auto_pause_on_change') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" :checked="ltAutoPauseOnMemberChange" @change="lt.updateRoomSettings({ autoPauseOnMemberChange: ($event.target as HTMLInputElement).checked })" />
          <span class="track"><span class="thumb"><span v-if="ltAutoPauseOnMemberChange" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">link</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.share_audio_links') }}</div>
        </div>
        <label class="m3-switch">
          <input type="checkbox" :checked="ltShareAudioLinks" @change="lt.updateRoomSettings({ shareAudioLinks: ($event.target as HTMLInputElement).checked })" />
          <span class="track"><span class="thumb"><span v-if="ltShareAudioLinks" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
        </label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">restart_alt</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('listen_together.reset_identity') }}</div>
          <div class="setting-desc">{{ t('listen_together.reset_identity_desc') }}</div>
        </div>
        <button
          class="m3-chip sm danger"
          :disabled="lt.isInSession"
          :title="lt.isInSession ? t('listen_together.reset_identity_in_room') : undefined"
          @click="showResetLtIdentityConfirm = true"
        >{{ t('listen_together.reset_btn') }}</button>
      </div>
    </div></Transition>
        </div>

    <!-- 下载管理 -->
        <div v-show="activeSettingsSection === 'downloads'" class="settings-section-panel" data-section="downloads">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">download</span>
      <span>{{ t('settings.download_manage') }}</span>
    </div>

    <template v-if="activeDownloadCount > 0">
      <div class="setting-card download-summary-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">downloading</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('download.active_tasks', { count: activeDownloadCount }) }}</div>
          <div class="setting-desc">{{ completedDownloadCount > 0 ? t('player.track_count', { count: completedDownloadCount }) : t('settings.no_active_downloads') }}</div>
        </div>
        <button class="m3-chip sm" style="color: var(--md-error); border-color: var(--md-error)" @click="cancelAllDownloads">{{ t('settings.cancel_all_downloads') }}</button>
      </div>
      <div
        v-for="task in activeDownloadTasks"
        :key="'settings-download-' + task.trackId"
        class="setting-card sub-card active-download-card"
      >
        <div class="setting-icon-wrap">
          <span class="material-symbols-rounded">{{ task.status === 'resolving' ? 'network_node' : task.status === 'error' ? 'error' : task.status === 'cancelled' ? 'cancel' : 'downloading' }}</span>
        </div>
        <div class="setting-info">
          <div class="setting-title">{{ task.title }}</div>
          <div class="setting-desc">{{ task.artist }} · {{ activeDownloadProgressText(task) }}</div>
          <div
            class="settings-download-progress"
            :class="{
              indeterminate: task.status === 'processing' || task.status === 'resolving' || (task.status === 'downloading' && !task.totalBytes),
              error: task.status === 'error',
              muted: task.status === 'queued' || task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'already_exists',
            }"
          >
            <div
              class="settings-download-progress-fill"
              :style="{ width: `${Math.max(4, task.status === 'cancelled' || task.status === 'already_exists' ? 100 : task.progress ?? 0)}%` }"
            />
          </div>
        </div>
        <button
          class="download-cancel-btn"
          :disabled="task.status === 'cancelling' || task.status === 'cancelled' || task.status === 'error' || task.status === 'already_exists'"
          @click="cancelDownload(task.trackId)"
        >
          <span class="material-symbols-rounded">close</span>
        </button>
      </div>
    </template>
    <div v-else class="setting-card" style="cursor: pointer" @click="goToDownloads">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder_open</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.go_to_downloads') }}</div>
        <div class="setting-desc">{{ completedDownloadCount > 0 ? t('player.track_count', { count: completedDownloadCount }) : t('settings.no_active_downloads') }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
    </div>

    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">tune</span>
      <span>{{ t('settings.download_settings') }}</span>
    </div>
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">downloading</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_parallelism') }}</div>
          <div class="setting-desc">{{ t('settings.download_parallelism_desc') }}</div>
          <EditableRangeValue
            v-model="downloadParallelism"
            class="setting-desc"
            :min="MIN_DOWNLOAD_PARALLELISM"
            :max="MAX_DOWNLOAD_PARALLELISM"
            :step="1"
            :display-value="t('settings.download_parallelism_value', { count: downloadParallelism })"
            :aria-label="t('settings.download_parallelism')"
          />
        </div>
        <input v-model.number="downloadParallelism" type="range" class="m3-slider" :min="MIN_DOWNLOAD_PARALLELISM" :max="MAX_DOWNLOAD_PARALLELISM" step="1" :aria-label="t('settings.download_parallelism')" />
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">audio_file</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_metadata') }}</div>
          <div class="setting-desc">{{ t('settings.download_metadata_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="downloadAutoFillMetadata" /><span class="track"><span class="thumb"><span v-if="downloadAutoFillMetadata" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card sub-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">lyrics</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_embed_lyrics') }}</div>
          <div class="setting-desc">{{ t('settings.download_embed_lyrics_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="downloadEmbedLyrics" :disabled="!downloadAutoFillMetadata" /><span class="track"><span class="thumb"><span v-if="downloadEmbedLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">high_quality</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_follow_playback_quality') }}</div>
          <div class="setting-desc">{{ t('settings.download_follow_playback_quality_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="downloadFollowPlaybackQuality" /><span class="track"><span class="thumb"><span v-if="downloadFollowPlaybackQuality" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="!downloadFollowPlaybackQuality">
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.netease_quality') }}</div></div>
          <CustomSelect v-model="downloadNeteaseQuality" :options="downloadNeteaseQualityOptions" :label="t('settings.netease_quality')" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.youtube_quality') }}</div></div>
          <CustomSelect v-model="downloadYoutubeQuality" :options="youtubeQualityOptions" :label="t('settings.youtube_quality')" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info"><div class="setting-title">{{ t('settings.bili_quality') }}</div></div>
          <CustomSelect v-model="downloadBiliQuality" :options="biliQualityOptions" :label="t('settings.bili_quality')" />
        </div>
      </template>

      <div class="setting-card" style="cursor: pointer" @click="openDownloadTemplateDialog">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">text_fields</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_format') }}</div>
          <div class="setting-desc">{{ downloadNameTemplate }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">edit</span>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.download_dir') }}</div>
          <div class="setting-desc" style="word-break: break-all">{{ displayDownloadDir }}</div>
          <div class="setting-desc">{{ t('settings.download_dir_desc') }}</div>
        </div>
        <div class="chip-row">
          <button class="m3-chip sm" :disabled="activeDownloadCount > 0" @click="selectDownloadDir">{{ t('settings.download_dir_select') }}</button>
          <button v-if="downloadDir" class="m3-chip sm" :disabled="activeDownloadCount > 0" @click="resetDownloadDir">{{ t('settings.download_dir_reset') }}</button>
        </div>
      </div>

        </div>

    <!-- 歌词 / 桌面歌词 -->
        <LyricsSettingsPanels
          :active-settings-section="activeSettingsSection"
          :toggle-section="toggleSection"
          :is-expanded="isExpanded"
          :lyrics="lyricsSettings"
        />

    <!-- 动效 & 视觉 -->
        <div v-show="activeSettingsSection === 'motion'" class="settings-section-panel" data-section="motion">
    <div class="section-label clickable" @click="toggleSection('effects')">
      <span class="material-symbols-rounded" style="font-size: 18px">auto_awesome</span>
      <span>{{ t('settings.effects') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('effects') }">expand_more</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">animation</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.advanced_lyrics') }}</div>
        <div class="setting-desc">{{ t('settings.advanced_lyrics_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="advancedLyrics" /><span class="track"><span class="thumb"><span v-if="advancedLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">wallpaper</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.dynamic_bg') }}</div>
        <div class="setting-desc">{{ t('settings.dynamic_bg_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" :checked="dynamicBackground" @change="onDynamicBackgroundToggle" /><span class="track"><span class="thumb"><span v-if="dynamicBackground" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <M3Dialog
      :open="dynamicBackgroundDialogOpen"
      icon="warning"
      :title="t('settings.dynamic_bg_warning_title')"
      :confirm-text="t('settings.dynamic_bg_warning_confirm')"
      :confirm-danger="true"
      @cancel="cancelDynamicBackgroundWarning"
      @confirm="confirmDynamicBackgroundWarning"
    >{{ t('settings.dynamic_bg_warning_body') }}</M3Dialog>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('effects')">
      <div v-if="dynamicBackground" class="setting-card sub-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.audio_reactive') }}</div>
          <div class="setting-desc">{{ t('settings.audio_reactive_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="audioReactive" /><span class="track"><span class="thumb"><span v-if="audioReactive" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">blur_circular</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cover_blur') }}</div>
          <div class="setting-desc">{{ t('settings.cover_blur_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="coverBlurBg" /><span class="track"><span class="thumb"><span v-if="coverBlurBg" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <template v-if="coverBlurBg">
        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.blur_amount') }}</div>
            <EditableRangeValue
              v-model="coverBlurAmount"
              class="setting-desc"
              :min="0"
              :max="MAX_COVER_BLUR_AMOUNT"
              :step="0.1"
              :display-value="`${Math.round(coverBlurAmount * COVER_BLUR_PX_PER_UNIT)} px`"
              :input-scale="COVER_BLUR_PX_PER_UNIT"
              input-suffix="px"
              :aria-label="t('settings.blur_amount')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="coverBlurAmount" min="0" :max="MAX_COVER_BLUR_AMOUNT" step="0.1" />
        </div>
        <div class="setting-card sub-card">
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.bg_darken') }}</div>
            <EditableRangeValue
              v-model="coverBlurDarken"
              class="setting-desc"
              :min="0"
              :max="0.8"
              :step="0.05"
              :inputScale="100"
              :display-value="`${(coverBlurDarken * 100).toFixed(0)}%`"
              input-suffix="%"
              :aria-label="t('settings.bg_darken')"
            />
          </div>
          <input type="range" class="m3-slider" v-model.number="coverBlurDarken" min="0" max="0.8" step="0.05" />
        </div>
      </template>
    </div></Transition>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">speed</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.low_performance_mode') }}</div>
        <div class="setting-desc">{{ t('settings.low_performance_mode_desc') }}</div>
      </div>
      <label class="m3-switch"><input type="checkbox" v-model="lowPerformanceMode" /><span class="track"><span class="thumb"><span v-if="lowPerformanceMode" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>
        </div>

    <!-- 播放源 -->
        <div v-show="activeSettingsSection === 'playback_sources'" class="settings-section-panel" data-section="playback_sources">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">alt_route</span>
      <span>{{ t('settings.playback_sources') }}</span>
    </div>

    <div class="setting-card setting-card--select">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">smart_display</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.youtube_playback_source') }}</div>
        <div class="setting-desc">{{ t('settings.youtube_playback_source_desc') }}</div>
        <div class="setting-desc">{{ youtubePlaybackSourceDescription }}</div>
      </div>
      <CustomSelect class="settings-select" :model-value="youtubePlaybackSource" :options="youtubePlaybackSourceOptions"
        :label="t('settings.youtube_playback_source')" @update:model-value="changeYouTubePlaybackSource" />
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">library_music</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.netease_local_source_fallback') }}</div>
        <div class="setting-desc">{{ t('settings.netease_local_source_fallback_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" v-model="neteaseLocalSourceFallback" :aria-label="t('settings.netease_local_source_fallback')" />
        <span class="track"><span class="thumb"><span v-if="neteaseLocalSourceFallback" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync_alt</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.netease_auto_source_switch') }}</div>
        <div class="setting-desc">{{ t('settings.netease_auto_source_switch_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input type="checkbox" v-model="neteaseAutoSourceSwitch" :aria-label="t('settings.netease_auto_source_switch')" />
        <span class="track"><span class="thumb"><span v-if="neteaseAutoSourceSwitch" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>
        </div>

    <!-- 音质 -->
        <div v-show="activeSettingsSection === 'quality'" class="settings-section-panel" data-section="quality">
    <div class="section-label clickable" @click="toggleSection('quality')">
      <span class="material-symbols-rounded" style="font-size: 18px">headphones</span>
      <span>{{ t('settings.audio_quality') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('quality') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('quality')">
      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.netease_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in neteaseQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: neteaseQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('netease', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">smart_display</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.youtube_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in youtubeQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: youtubeQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('youtube', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card quality-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">play_circle</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.bili_quality') }}</div>
          <div class="chip-wrap">
            <button v-for="o in biliQualityOptions" :key="o.value" class="m3-chip sm" :class="{ active: biliQuality === o.value }" :disabled="qualitySwitching" @click="handleQualityChange('bilibili', o.value)">{{ o.label }}</button>
          </div>
        </div>
      </div>
    </div></Transition>
        </div>

    <!-- 存储 & 缓存 -->
        <div v-show="activeSettingsSection === 'storage'" class="settings-section-panel" data-section="storage">
    <div class="section-label clickable" @click="toggleSection('storage')">
      <span class="material-symbols-rounded" style="font-size: 18px">folder</span>
      <span>{{ t('settings.storage') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('storage') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('storage')">
      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">sd_storage</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.cache_limit') }}</div>
          <EditableRangeValue
            v-model="maxCacheSize"
            class="setting-desc"
            :min="MIN_MEDIA_CACHE_SIZE_MB"
            :max="MAX_MEDIA_CACHE_SIZE_MB"
            :step="256"
            :display-value="maxCacheSize >= 1024 ? `${(maxCacheSize / 1024).toFixed(1)} GB` : `${maxCacheSize} MB`"
            input-suffix="MB"
            :aria-label="t('settings.cache_limit')"
          />
        </div>
        <input
          v-model.number="maxCacheSize"
          type="range"
          class="m3-slider"
          :min="MIN_MEDIA_CACHE_SIZE_MB"
          :max="MAX_MEDIA_CACHE_SIZE_MB"
          step="256"
        />
      </div>

      <div class="setting-card" style="cursor: pointer" @click="openStorageManagement">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">database</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.storage_usage_title') }}</div>
          <div class="setting-desc">{{ t('settings.storage_usage_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="section-label" style="margin-top: 8px">
        <span class="material-symbols-rounded" style="font-size: 18px">description</span>
        <span>{{ t('settings.logs_title') }}</span>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">edit_note</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.log_to_file') }}</div>
          <div class="setting-desc">{{ t('settings.log_to_file_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="logToFile" @change="onLogToFileChange" /><span class="track"><span class="thumb"><span v-if="logToFile" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">tune</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.log_level') }}</div>
          <div class="setting-desc">{{ t('settings.log_level_desc') }}</div>
          <div class="chip-row" style="margin-top: 8px">
            <button v-for="opt in logLevelOptions" :key="opt.value" class="m3-chip sm" :class="{ active: logLevel === opt.value }" @click="logLevel = opt.value">{{ opt.label }}</button>
          </div>
        </div>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="openLogDir">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder_open</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.open_log_dir') }}</div>
          <div class="setting-desc">{{ t('settings.open_log_dir_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>
    </div></Transition>
        </div>

    <!-- 备份 & 恢复 -->
        <div v-show="activeSettingsSection === 'backup'" class="settings-section-panel" data-section="backup">
    <div class="section-label clickable" @click="toggleSection('backup')">
      <span class="material-symbols-rounded" style="font-size: 18px">cloud_sync</span>
      <span>{{ t('settings.backup') }}</span>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('backup') }">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('backup')">

      <!-- 播放历史同步频率：与 Android 一样作为全局偏好 -->
      <div class="setting-card sub-card sync-frequency-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">timer</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.sync_frequency') }}</div>
          <div class="setting-desc">{{ t('settings.sync_frequency_desc') }}</div>
          <div class="chip-wrap">
            <button
              v-for="option in syncFrequencyOptions"
              :key="option.value"
              class="m3-chip sm"
              :class="{ active: syncStore.syncFrequency === option.value }"
              type="button"
              @click="syncStore.syncFrequency = option.value"
            >{{ option.label }}</button>
          </div>
        </div>
      </div>

      <!-- GitHub 同步 -->
      <template v-if="!syncStore.github.configured">
        <!-- 未配置：单行入口 -->
        <div class="setting-card" style="cursor: pointer" @click="openGitHubSetup">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud_sync</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.github_sync') }}</div>
            <div class="setting-desc">{{ t('settings.github_sync_desc') }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
        </div>
      </template>
      <template v-else>
        <!-- 已配置：完整管理面板 -->
        <div class="setting-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud_sync</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.github_sync') }}</div>
            <div class="setting-desc">{{ syncStore.github.owner }}/{{ syncStore.github.repo }}</div>
          </div>
          <span class="sync-status-pill configured">{{ t('settings.sync_configured') }}</span>
        </div>

        <!-- 自动同步开关 -->
        <div class="setting-card sub-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.auto_sync') }}</div>
            <div class="setting-desc">{{ t('settings.auto_sync_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" v-model="syncStore.github.autoSync" /><span class="track"><span class="thumb"><span v-if="syncStore.github.autoSync" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
        </div>

        <!-- 立即同步 -->
        <div class="setting-card sub-card" style="cursor: pointer" @click="syncStore.syncGitHub()">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud_upload</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.sync_now') }}</div>
            <div class="setting-desc">
              <template v-if="syncStore.github.lastSyncTime">{{ t('settings.last_sync', { time: formatSyncTime(syncStore.github.lastSyncTime) }) }}</template>
              <template v-else>{{ t('settings.not_synced') }}</template>
            </div>
          </div>
          <span v-if="syncStore.isSyncing" class="material-symbols-rounded spinning" style="font-size: 20px">progress_activity</span>
          <span v-else class="sync-action-label">{{ t('settings.sync_action') }}</span>
        </div>

        <!-- 静默同步失败 -->
        <div class="setting-card sub-card">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded">error</span></div>
          <div class="setting-info">
            <div class="setting-title">{{ t('settings.silent_failures') }}</div>
            <div class="setting-desc">{{ t('settings.silent_failures_desc') }}</div>
          </div>
          <label class="m3-switch"><input type="checkbox" v-model="syncStore.github.silentFailures" /><span class="track"><span class="thumb"><span v-if="syncStore.github.silentFailures" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
        </div>

        <!-- 清除配置 -->
        <div class="setting-card sub-card" style="justify-content: center;">
          <button class="clear-config-btn" @click="showClearGitHubConfirm = true">
            <span class="material-symbols-rounded" style="font-size: 16px">delete_outline</span>
            {{ t('settings.clear_config') }}
          </button>
        </div>
      </template>

      <!-- WebDAV 同步 -->
      <div class="setting-card" style="cursor: pointer" @click="syncStore.webdav.configured ? syncStore.syncWebDav() : (showWebDavDialog = true)">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">dns</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.webdav_sync') }}</div>
          <div class="setting-desc">
            <template v-if="syncStore.webdav.configured">
              {{ syncStore.webdav.serverUrl }}
              <span v-if="syncStore.webdav.lastSyncTime"> · {{ formatSyncTime(syncStore.webdav.lastSyncTime) }}</span>
            </template>
            <template v-else>{{ t('settings.webdav_sync_desc') }}</template>
          </div>
        </div>
        <span v-if="syncStore.isSyncing" class="material-symbols-rounded spinning" style="font-size: 20px">progress_activity</span>
        <button v-else-if="syncStore.webdav.configured" class="sync-disconnect-btn" @click.stop="syncStore.disconnectWebDav()">
          <span class="material-symbols-rounded" style="font-size: 18px">link_off</span>
        </button>
        <span v-else class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div v-if="syncStore.webdav.configured" class="setting-card sub-card">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">sync</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.auto_sync') }}</div>
          <div class="setting-desc">{{ t('settings.auto_sync_desc') }}</div>
        </div>
        <label class="m3-switch"><input type="checkbox" v-model="syncStore.webdav.autoSync" /><span class="track"><span class="thumb"><span v-if="syncStore.webdav.autoSync" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="syncStore.exportPlaylists()">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">upload_file</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.export_playlist') }}</div>
          <div class="setting-desc">{{ t('settings.export_playlist_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="syncStore.importPlaylists()">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">download</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.import_playlist') }}</div>
          <div class="setting-desc">{{ t('settings.import_playlist_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="configTransferMode = 'export'">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">settings_backup_restore</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.export_config') }}</div>
          <div class="setting-desc">{{ t('settings.export_config_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>

      <div class="setting-card" style="cursor: pointer" @click="configTransferMode = 'import'">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">restore</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.import_config') }}</div>
          <div class="setting-desc">{{ t('settings.import_config_desc') }}</div>
        </div>
        <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">chevron_right</span>
      </div>
    </div></Transition>
        </div>

    <!-- 语言 -->
        <div v-show="activeSettingsSection === 'language'" class="settings-section-panel" data-section="language">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">language</span>
      <span>{{ t('settings.language') }}</span>
    </div>

    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">translate</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.language') }}</div>
        <div class="setting-desc">{{ t('settings.language_desc') }}</div>
      </div>
      <div class="chip-row">
        <button v-for="loc in SUPPORTED_LOCALES" :key="loc.code" class="m3-chip" :class="{ active: locale === loc.code }" @click="handleLocaleSwitch(loc.code, $event)">{{ loc.label }}</button>
      </div>
    </div>
        </div>

    <!-- 关于 -->
        <div v-show="activeSettingsSection === 'about'" class="settings-section-panel" data-section="about">
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">info</span>
      <span>{{ t('settings.about') }}</span>
    </div>

    <div class="setting-card about-card" @click="handleVersionTap" style="cursor: pointer">
      <div class="setting-icon-wrap accent">
        <img src="/app-icon.png" alt="NeriPlayer" style="width: 24px; height: 24px; border-radius: 4px;" />
      </div>
      <div class="setting-info">
        <div class="setting-title about-wordmark">NeriPlayer <span class="about-wordmark-hand">Desktop</span>{{ devModeEnabled ? ' (dev)' : '' }}</div>
        <div class="setting-desc mono-build-value">
          {{ t('settings.version_info', {
            version: buildInfo?.version || 'dev',
          }) }}
        </div>
      </div>
      <button
        type="button"
        class="about-copy-btn"
        :title="t('settings.copy_build_version')"
        :aria-label="t('settings.copy_build_version')"
        @click="copyBuildValue(buildInfo?.version, $event)"
      >
        <span class="material-symbols-rounded">content_copy</span>
      </button>
      <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('about') }" style="font-size: 20px; opacity: 0.3">expand_more</span>
    </div>

    <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('about') && buildInfo">
      <div
        class="setting-card sub-card copyable-setting-card"
        :title="t('settings.copy_build_uuid')"
        @click="copyBuildValue(buildInfo.build_uuid, $event)"
      >
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">fingerprint</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.build_uuid') }}</div>
          <div class="setting-desc mono-build-value">{{ buildInfo.build_uuid }}</div>
        </div>
        <span class="material-symbols-rounded copy-hint-icon">content_copy</span>
      </div>
      <div
        class="setting-card sub-card copyable-setting-card"
        :title="t('settings.copy_build_time')"
        @click="copyBuildValue(buildInfo.build_timestamp, $event)"
      >
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">schedule</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.build_time') }}</div>
          <div class="setting-desc mono-build-value">{{ buildInfo.build_timestamp }}</div>
        </div>
        <span class="material-symbols-rounded copy-hint-icon">content_copy</span>
      </div>
    </div></Transition>

    <div class="setting-card" style="cursor: pointer" @click="openExternalUrl(PROJECT_REPOSITORY_URL)">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">code</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.github') }}</div>
        <div class="setting-desc">{{ t('settings.github_desc') }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">open_in_new</span>
    </div>
    <div
      class="setting-card"
      style="cursor: pointer"
      :title="player.decoderCapabilities?.ffmpegError ?? player.decoderCapabilities?.ffmpeg?.directory ?? ''"
      @click="openExternalUrl(FFMPEG_LEGAL_URL)"
    >
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.ffmpeg_component') }}</div>
        <div class="setting-desc">{{ ffmpegComponentText }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.3">open_in_new</span>
    </div>
        </div>

        </div>
        </Transition>
        </div>
      </main>
    </div>

    <SettingsDialogs :sync="syncSettings" :accounts="accountSettings" :network="networkSettings" :downloads="downloadSettings" />

    <StorageManagementDialog
      v-model:open="showStorageManagement"
      :loading="storageLoading"
      :clearing="storageClearing"
      :active-download-count="activeDownloadCount"
      :summary="storageSummary"
      @clear="clearStorageCache"
    />
  </div>
</template>

<style scoped lang="scss" src="./SettingsView.scss"></style>
