import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import type { AudioInfo } from '@/modules/playback/playbackAudioInfo'
import { canSwitchAudioQuality, resolveAudioQualityLabel } from '@/modules/playback/audioQualityDisplay'
import { qualityKeyForSource, qualityOptionsForSource, setQualityKeyForSource } from './audioQuality'
import { playbackSourceOfTrackId } from './nowPlayingHelpers'

/** 当前曲目的音质状态：页面的音质标签与更多面板的音质切换共用同一份 */
export function useAudioQuality() {
  const player = usePlayerStore()
  const settings = useSettingsStore()
  const { t } = useI18n()

  const currentSource = computed(() => playbackSourceOfTrackId(player.currentTrack?.id || ''))
  // 音质列表按播放层报告的可选项过滤（B 站只列这条视频实际提供的音质）；只有一档时不显示切换（对齐 Android）
  const switchableQualities = computed(() => {
    const all = qualityOptionsForSource(currentSource.value)
    const info = player.audioInfo
    const offered = info?.source === currentSource.value ? info.qualityOptions?.map(option => option.key) : undefined
    return offered?.length ? all.filter(option => offered.includes(option.key)) : all
  })
  const canSwitchCurrentAudioQuality = computed(() => canSwitchAudioQuality({
    source: currentSource.value, fromDownload: player.isPlayingFromDownload, info: player.audioInfo,
  }) && switchableQualities.value.length > 1)

  function currentQualityKey(source: string = currentSource.value): string {
    return qualityKeyForSource(settings, source)
  }

  function setQualityKey(source: string, key: string) {
    setQualityKeyForSource(settings, source, key)
  }

  function qualityLabelFor(source: string, key?: string) {
    if (!key || source === 'local') return ''
    const item = qualityOptionsForSource(source).find(q => q.key === key)
    return item ? t(item.label) : key
  }

  function currentAudioQualityLabel(info: AudioInfo | null = player.audioInfo, fromDownload = player.isPlayingFromDownload) {
    return resolveAudioQualityLabel({ source: currentSource.value, fromDownload, info },
      (source, key) => qualityLabelFor(source, key || currentQualityKey(source)))
  }

  return {
    currentSource,
    switchableQualities,
    canSwitchCurrentAudioQuality,
    currentQualityKey,
    setQualityKey,
    currentAudioQualityLabel,
  }
}
