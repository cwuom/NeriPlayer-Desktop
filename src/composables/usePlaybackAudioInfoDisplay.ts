import { shallowRef, watch } from 'vue'
import type { AudioInfo } from '@/stores/player'

interface PlaybackAudioInfoState {
  info: AudioInfo | null
  fromDownload: boolean
  loading: boolean
  hasSession: boolean
}

export function usePlaybackAudioInfoDisplay(readState: () => PlaybackAudioInfoState) {
  const display = shallowRef<{ info: AudioInfo | null; fromDownload: boolean }>({
    info: null,
    fromDownload: false,
  })
  watch(readState, (state) => {
    // 加载期间保留上一帧，仅缓冲显示，不把旧规格写回新曲目的播放状态
    if (state.loading) return
    display.value = {
      info: state.hasSession ? state.info : null,
      fromDownload: state.hasSession && state.fromDownload,
    }
  }, { immediate: true })
  return display
}
