import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { openUrl } from '@tauri-apps/plugin-opener'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useSettingsStore } from '@/stores/settings'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { settingsLog as log, type BuildInfo, type SettingsIpc } from './settingsShared'

export const PROJECT_REPOSITORY_URL = 'https://github.com/cwuom/NeriPlayer-Desktop'
export const FFMPEG_LEGAL_URL = 'https://ffmpeg.org/legal.html'

/** 关于分区：版本信息与复制、开发者模式 7-tap 解锁、外部链接 */
export function useAboutSettings(ipc: SettingsIpc, toggleSection: (key: string) => void) {
  const { t } = useI18n()
  const toast = useToastStore()
  const player = usePlayerStore()
  const { devModeEnabled } = storeToRefs(useSettingsStore())

  // 开发者模式：7-tap 解锁
  const versionTapCount = ref(0)
  let tapTimer: ReturnType<typeof setTimeout> | null = null

  function handleVersionTap() {
    toggleSection('about')
    if (devModeEnabled.value) return // 已解锁
    versionTapCount.value++
    if (tapTimer) clearTimeout(tapTimer)
    tapTimer = setTimeout(() => { versionTapCount.value = 0 }, 3000)

    const remaining = 7 - versionTapCount.value
    if (remaining <= 0) {
      devModeEnabled.value = true
      versionTapCount.value = 0
      toast.success(t('settings.dev_mode_toast'))
    } else if (remaining <= 3) {
      toast.success(t('settings.dev_mode_tap_hint', { count: remaining }))
    }
  }

  // 卸载时兜底清理 7-tap 计时器
  function disposeVersionTap() {
    if (tapTimer) clearTimeout(tapTimer)
  }

  // 构建信息
  const buildInfo = ref<BuildInfo | null>(null)

  async function loadBuildInfo() {
    try {
      buildInfo.value = await ipc.getBuildInfo()
    } catch (e) {
      log.error('Failed to load build info:', e)
    }
  }

  async function copyBuildValue(value: string | undefined | null, event?: Event) {
    event?.stopPropagation()
    const text = value?.trim()
    if (!text) return
    try {
      await writeText(text)
      toast.success(t('settings.build_info_copied'))
    } catch (e) {
      log.error('Failed to copy build info:', e)
      toast.error(t('settings.build_info_copy_failed'))
    }
  }

  // 随包的 FFmpeg 是否加载成功、加载的是哪个版本；没加载上时说明哪些格式受影响
  const ffmpegComponentText = computed(() => {
    const capabilities = player.decoderCapabilities
    if (capabilities?.ffmpeg) return t('settings.ffmpeg_component_loaded', { version: capabilities.ffmpeg.avcodec })
    return t('settings.ffmpeg_component_unavailable')
  })

  async function openExternalUrl(url: string) {
    try {
      await openUrl(url)
    } catch (error) {
      log.error('Failed to open external URL', error)
      toast.error(t('settings.open_external_link_failed'))
    }
  }

  return {
    handleVersionTap,
    disposeVersionTap,
    buildInfo,
    loadBuildInfo,
    copyBuildValue,
    ffmpegComponentText,
    openExternalUrl,
  }
}
