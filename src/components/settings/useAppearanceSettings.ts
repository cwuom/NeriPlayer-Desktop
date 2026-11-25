import { computed, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'
import { setLocaleWithTransition } from '@/i18n'
import { UI_SCALE_MIN, UI_SCALE_STEP, useSettingsStore, type CoverStyle } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { uiScaleMax } from '@/modules/settings/uiScale'
import { isLinuxDesktop } from '@/utils/linuxNotice'
import { dialogSelectionPath, settingsLog as log, type SettingsIpc } from './settingsShared'

export const audioDisplayOptions = [
  { key: 'showAudioBitrate', label: 'audio_bitrate', icon: 'speed' },
  { key: 'showAudioFormat', label: 'audio_format', icon: 'audio_file' },
  { key: 'showAudioChannels', label: 'audio_channels', icon: 'speaker_group' },
  { key: 'showAudioSampleRate', label: 'audio_sample_rate', icon: 'graphic_eq' },
  { key: 'showAudioBitDepth', label: 'audio_bit_depth', icon: 'equalizer' },
] as const

/** 外观、播放页、背景与动效分区：缩放 / 背景图，以及动态背景的互斥与警告流程（主题与配色见 appearance/ThemeAppearanceSettings.vue） */
export function useAppearanceSettings(ipc: SettingsIpc) {
  const { t } = useI18n()
  const toast = useToastStore()
  const {
    locale: settingLocale, backgroundImageUri,
    dynamicBackground, coverBlurBg,
  } = storeToRefs(useSettingsStore())

  const uiScaleOptions = computed(() => Array.from(
    { length: (uiScaleMax.value - UI_SCALE_MIN) / UI_SCALE_STEP + 1 },
    (_, index) => {
      const value = UI_SCALE_MIN + index * UI_SCALE_STEP
      return { value: String(value), label: `${value}%` }
    },
  ))

  // 动态背景与封面模糊互斥（对齐 Android effectiveDynamicBackgroundEnabled = dynamic && !coverBlur）:
  // 启用一方即关闭另一方，避免 coverBlurBg 残留导致动态背景开关看似失效
  watch(dynamicBackground, on => {
    if (on && coverBlurBg.value) coverBlurBg.value = false
  })
  watch(coverBlurBg, on => {
    if (on && dynamicBackground.value) dynamicBackground.value = false
  })

  // 动态背景（WebGL 着色器）在 Linux 下切换背景或页面时会闪烁，对光敏性癫痫用户有风险。
  // 首次运行的提示在 App.vue（会顺带把它默认关掉），这里只负责一件事：用户手动开启时
  // 先拒绝、弹严重警告，确认后才真正写入设置。
  // The WebGL shader background flickers on Linux when the background or page changes, a
  // risk for photosensitive epilepsy. The first-run notice lives in App.vue (which also
  // turns it off by default); this component only handles one path: a manual enable is
  // rejected first, a severe warning is shown, and the setting is written after confirming.
  const dynamicBackgroundAcknowledged = ref(false)
  const dynamicBackgroundDialogOpen = ref(false)

  // 只在用户真正要开的时候才改状态：被拒绝的开启不会落到 dynamicBackground 上，
  // 所以下面那个“动态背景与封面模糊互斥”的 watcher 不会先把 coverBlurBg 清掉，
  // 用户取消时也就不会连原来的封面模糊背景一起丢掉。
  // Only change state when the user is actually allowed to enable it: a rejected
  // enable never reaches `dynamicBackground`, so the mutual-exclusion watcher below
  // cannot clear `coverBlurBg` first and cancelling keeps the previous background.
  //
  // Linux 端一律警告：闪烁是 WebKitGTK 的呈现路径问题，与显卡厂商无关（实测核显渲染时
  // 同样会闪）。这里先回弹复选框、确认后才真正写入设置。
  // Warn on every Linux desktop: the flicker comes from WebKitGTK's presentation path, not
  // from the GPU vendor (it reproduced while rendering on the integrated GPU). Bounce the
  // checkbox first and only write the setting after confirmation.
  function onDynamicBackgroundToggle(event: Event) {
    const input = event.target as HTMLInputElement
    if (!input.checked) {
      dynamicBackground.value = false
      return
    }
    input.checked = false
    if (isLinuxDesktop() && !dynamicBackgroundAcknowledged.value) {
      dynamicBackgroundDialogOpen.value = true
      return
    }
    dynamicBackground.value = true
  }

  function confirmDynamicBackgroundWarning() {
    dynamicBackgroundAcknowledged.value = true
    dynamicBackgroundDialogOpen.value = false
    dynamicBackground.value = true
  }

  // 手动开启被拒时开关本来就是关的，再置一次无害；进入设置页那次提示对应的是“原本开着”，
  // 取消就是把动态背景关掉。
  // After a rejected manual toggle the switch is already off, so setting it again is
  // harmless; the mount-time prompt corresponds to "already on", where cancelling means
  // turning the dynamic background off.
  function cancelDynamicBackgroundWarning() {
    dynamicBackgroundDialogOpen.value = false
    dynamicBackground.value = false
  }

  function handleLocaleSwitch(code: string, event: MouseEvent) {
    settingLocale.value = code
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
    const x = rect.left + rect.width / 2
    const y = rect.top + rect.height / 2
    setLocaleWithTransition(code, x, y, false)
  }

  const defaultScreenOptions = computed(() => [
    { value: 'home', label: t('nav.home') },
    { value: 'explore', label: t('nav.explore') },
    { value: 'library', label: t('nav.library') },
  ])

  // 封面样式选项
  const coverStyleOptions = computed<{ value: CoverStyle; label: string }[]>(() => [
    { value: 'disc', label: t('settings.cover_style_disc') },
    { value: 'card', label: t('settings.cover_style_card') },
  ])

  // 背景图片选择
  async function selectBackgroundImage() {
    try {
      const result = await dialogOpen({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp'] }],
      })
      if (result) {
        const picked = dialogSelectionPath(result)
        backgroundImageUri.value = await ipc.importBackgroundImage(picked)
      }
    } catch (e) {
      log.error('Failed to select image:', e)
      toast.error(t('settings.background_image_failed'))
    }
  }

  function clearBackgroundImage() {
    backgroundImageUri.value = ''
    ipc.clearBackgroundImages().catch((error) => log.warn('Failed to delete background copies:', error))
  }

  return {
    uiScaleOptions,
    dynamicBackgroundDialogOpen,
    onDynamicBackgroundToggle,
    confirmDynamicBackgroundWarning,
    cancelDynamicBackgroundWarning,
    handleLocaleSwitch,
    defaultScreenOptions,
    coverStyleOptions,
    selectBackgroundImage,
    clearBackgroundImage,
  }
}
