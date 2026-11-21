import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { YOUTUBE_PLAYBACK_SOURCES, useSettingsStore } from '@/stores/settings'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { settingsLog as log, type AudioOutputDeviceOption, type SettingsIpc, type Translate } from './settingsShared'

/** KDE-style `<card> <role>` for ALSA devices; the server's own text for sinks */
export function audioOutputLabel(device: AudioOutputDeviceOption, t: Translate): string {
  const base = device.role
    ? (() => {
        const role = t(`settings.audio_output_role_${device.role}`, { detail: device.detail ?? '' }).trim()
        return device.card ? `${device.card} ${role}` : role
      })()
    : device.label || device.name
  // A sound server sink can stop being the active output when another desktop
  // changes it, so mark the one the server currently routes to
  return device.kind === 'sink' && device.isDefault
    ? `${base} ${t('settings.audio_output_system_default')}`
    : base
}

export function audioOutputOptionsFor(devices: AudioOutputDeviceOption[], selected: string, t: Translate) {
  const options = devices.map(device => ({
    value: device.name,
    label: audioOutputLabel(device, t),
  }))
  // The same card can expose several PCMs with the same role (e.g. front and
  // surround aliases). Keep the short label when unique, but disambiguate with
  // the PCM name when two entries would otherwise be indistinguishable.
  const labelCounts = new Map<string, number>()
  for (const device of options) {
    labelCounts.set(device.label, (labelCounts.get(device.label) ?? 0) + 1)
  }
  for (const device of options) {
    if ((labelCounts.get(device.label) ?? 0) > 1) device.label = `${device.label} (${device.value})`
  }
  if (selected && !options.some(device => device.value === selected)) {
    options.unshift({ value: selected, label: t('settings.audio_output_unavailable', { name: selected }) })
  }
  return [{ value: '', label: t('settings.audio_output_default') }, ...options]
}

type OnlineQualitySource = 'netease' | 'youtube' | 'bilibili'

/** 音频输出、音量平衡、在线音质与播放源分区 */
export function usePlaybackSettings(ipc: SettingsIpc) {
  const { t } = useI18n()
  const player = usePlayerStore()
  const toast = useToastStore()
  const {
    audioOutputDevice, volumeBalance, neteaseQuality, youtubeQuality, biliQuality, youtubePlaybackSource,
  } = storeToRefs(useSettingsStore())

  const youtubePlaybackSourceOptions = computed(() => YOUTUBE_PLAYBACK_SOURCES.map(value => ({
    value,
    label: t(`settings.youtube_source_${value}`),
  })))
  const youtubePlaybackSourceDescription = computed(() => t(`settings.youtube_source_${youtubePlaybackSource.value}_desc`))

  function changeYouTubePlaybackSource(value: string) {
    const source = YOUTUBE_PLAYBACK_SOURCES.find(source => source === value)
    if (source) youtubePlaybackSource.value = source
  }

  const audioOutputDevices = ref<AudioOutputDeviceOption[]>([])
  const audioOutputSwitching = ref(false)

  const audioOutputOptions = computed(() => audioOutputOptionsFor(audioOutputDevices.value, audioOutputDevice.value, t))

  async function loadAudioOutputDevices() {
    try {
      audioOutputDevices.value = await ipc.listAudioOutputDevices()
    } catch (error) {
      log.warn('failed to list audio output devices:', error)
    }
  }

  async function changeAudioOutputDevice(selected: string) {
    if (audioOutputSwitching.value || selected === audioOutputDevice.value) return
    audioOutputSwitching.value = true
    try {
      await ipc.setAudioOutputDevice(selected || null)
      audioOutputDevice.value = selected
    } catch (error) {
      log.warn('failed to switch audio output:', error)
      toast.error(t('settings.audio_output_failed'))
    } finally {
      audioOutputSwitching.value = false
      void loadAudioOutputDevices()
    }
  }

  const volumeBalanceLabel = computed(() => {
    const percent = Math.round(Math.abs(volumeBalance.value) * 100)
    if (percent === 0) return t('settings.volume_balance_center')
    return t(volumeBalance.value < 0 ? 'settings.volume_balance_left' : 'settings.volume_balance_right', { percent })
  })

  const neteaseQualityOptions = computed(() => [
    { value: 'standard', label: t('settings.q_standard') },
    { value: 'higher', label: t('settings.q_high') },
    { value: 'exhigh', label: t('settings.q_exhigh') },
    { value: 'lossless', label: t('settings.q_lossless') },
    { value: 'hires', label: t('settings.q_hires') },
    { value: 'jyeffect', label: t('settings.q_surround') },
    { value: 'sky', label: t('settings.q_sky') },
    { value: 'jymaster', label: t('settings.q_master') },
  ])

  const youtubeQualityOptions = computed(() => [
    { value: 'low', label: t('settings.q_low') },
    { value: 'medium', label: t('settings.q_medium') },
    { value: 'high', label: t('settings.q_high_yt') },
    { value: 'very_high', label: t('settings.q_very_high') },
  ])

  const biliQualityOptions = computed(() => [
    { value: 'low', label: t('settings.q_smooth') },
    { value: 'medium', label: t('settings.q_standard') },
    { value: 'high', label: t('settings.q_good') },
    { value: 'lossless', label: t('settings.q_lossless') },
    { value: 'hires', label: t('settings.q_hires') },
    { value: 'dolby', label: t('settings.q_dolby') },
  ])

  const qualitySwitching = ref(false)

  function qualityForSource(source: OnlineQualitySource): string {
    if (source === 'netease') return neteaseQuality.value
    if (source === 'youtube') return youtubeQuality.value
    return biliQuality.value
  }

  function setQualityForSource(source: OnlineQualitySource, value: string) {
    if (source === 'netease') neteaseQuality.value = value
    else if (source === 'youtube') youtubeQuality.value = value
    else biliQuality.value = value
  }

  async function handleQualityChange(source: OnlineQualitySource, value: string) {
    const previous = qualityForSource(source)
    if (previous === value || qualitySwitching.value) return

    setQualityForSource(source, value)
    const track = player.currentTrack
    const isCurrentSource = !!track && track.id.startsWith(`${source}:`)
    if (!track || player.isLoadingAudio || player.isPlayingFromDownload || !isCurrentSource) return

    qualitySwitching.value = true
    try {
      await player.replayWithQuality()
    } catch (error) {
      setQualityForSource(source, previous)
      const message = error instanceof Error ? error.message : String(error)
      toast.error(message)
    } finally {
      qualitySwitching.value = false
    }
  }

  return {
    youtubePlaybackSourceOptions,
    youtubePlaybackSourceDescription,
    changeYouTubePlaybackSource,
    audioOutputOptions,
    audioOutputSwitching,
    loadAudioOutputDevices,
    changeAudioOutputDevice,
    volumeBalanceLabel,
    neteaseQualityOptions,
    downloadNeteaseQualityOptions: neteaseQualityOptions,
    youtubeQualityOptions,
    biliQualityOptions,
    qualitySwitching,
    handleQualityChange,
  }
}
