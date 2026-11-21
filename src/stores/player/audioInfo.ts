import type { Ref } from 'vue'
import { loadLocalAudioInfo } from '@/modules/playback/localAudioInfo'
import { loadPlaybackAudioInfo, type AudioInfo, type PlaybackAudioProperties } from '@/modules/playback/playbackAudioInfo'
import { createLogger } from '@/utils/logger'

const log = createLogger('player')

export interface AudioInfoReadersContext {
  audioInfo: Ref<AudioInfo | null>
  hasPlaybackSession: Ref<boolean>
  isLoadingAudio: Ref<boolean>
  /** token 仍是最新的播放请求 */
  isCurrentRequest: (token: number) => boolean
  /** 后端当前装载的正是 token 这次请求 */
  isLoadedRequest: (token: number) => boolean
  /** 恢复出的会话还没装进后端 */
  needsReload: () => boolean
}

/** 当前会话的音质参数：文件属性探测与解码器上报可能先后到达，按请求代际合并 */
export function createAudioInfoReaders(ctx: AudioInfoReadersContext) {
  const { audioInfo, hasPlaybackSession, isLoadingAudio, isCurrentRequest, isLoadedRequest, needsReload } = ctx
  let decodedAudioInfo: { requestGeneration: number; properties: PlaybackAudioProperties } | null = null

  function resetDecodedAudioInfo() {
    decodedAudioInfo = null
  }

  function readLocalAudioInfo(path: string, requestGeneration: number) {
    void loadLocalAudioInfo(
      path,
      () => isCurrentRequest(requestGeneration) && hasPlaybackSession.value,
      info => {
        // 文件属性探测可能晚于解码器信息返回，保留当前会话的真实参数
        const decoded = decodedAudioInfo?.requestGeneration === requestGeneration
          ? decodedAudioInfo.properties
          : undefined
        if (decoded && typeof info.bitrate === 'number' && Number.isFinite(info.bitrate) && info.bitrate > 0) {
          delete decoded.bitrate
        }
        audioInfo.value = { ...info, ...decoded }
      },
    ).catch(error => log.warn('local audio properties unavailable:', error))
  }

  function readPlaybackAudioInfo(requestGeneration: number) {
    void loadPlaybackAudioInfo(
      requestGeneration,
      () => isCurrentRequest(requestGeneration)
        && isLoadedRequest(requestGeneration)
        && hasPlaybackSession.value && !isLoadingAudio.value && !needsReload(),
      info => {
        const properties = { ...info }
        const existingBitrate = audioInfo.value?.bitrate
        if (typeof existingBitrate === 'number' && Number.isFinite(existingBitrate) && existingBitrate > 0) {
          delete properties.bitrate
        }
        decodedAudioInfo = { requestGeneration, properties }
        audioInfo.value = { ...audioInfo.value, ...properties }
      },
    ).catch(error => log.warn('decoded audio properties unavailable:', error))
  }

  return { resetDecodedAudioInfo, readLocalAudioInfo, readPlaybackAudioInfo }
}
