import type { Ref } from 'vue'
import type { useSettingsStore } from '@/stores/settings'
import type { RepeatMode, TrackInfo } from '@/types/track'

export interface AudioEffectsRuntimeStats {
  active: boolean
  cpuLoadPercent: number
  limiterReductionDb: number
  compressorReductionDb: number
  sampleRate: number
  nativeAvailable: boolean
  inactiveReason: string | null
  outputDevice: string
}

/** 后端 get_decoder_capabilities 的结果 */
export interface DecoderCapabilities {
  ffmpeg: { directory: string; avutil: string; avcodec: string; avformat: string } | null
  ffmpegError?: string | null
  codecs: string[]
}
/** local_safety：睡眠定时、失败跳过、自动推进等内部操作，不受一起听的成员控制限制（对齐 Android LOCAL_SAFETY） */
export type PlaybackCommandSource = 'local' | 'local_safety' | 'remote_sync'

export interface SeekCommandSnapshot {
  seq: number
  positionMs: number
  source: PlaybackCommandSource
  requestGeneration: number
  fromPlaybackStart?: boolean
}

/** seek 发出后等待后端位置收敛的目标 */
export interface PendingSeekState {
  targetMs: number
  issuedAt: number
  expiresAt: number
}

export type PlayerSettings = ReturnType<typeof useSettingsStore>

/** player store 各切片共享的核心状态，全部由 store 创建后注入 */
export interface PlayerCoreState {
  settings: PlayerSettings
  isPlaying: Ref<boolean>
  currentTrack: Ref<TrackInfo | null>
  positionMs: Ref<number>
  durationMs: Ref<number>
  queue: Ref<TrackInfo[]>
  queueIndex: Ref<number>
  queueSelectionNeedsPlayback: Ref<boolean>
  repeatMode: Ref<RepeatMode>
  shuffleEnabled: Ref<boolean>
  shuffleBag: Ref<number[]>
  shuffleHistory: Ref<number[]>
  shuffleFuture: Ref<number[]>
  volume: Ref<number>
  isLoadingAudio: Ref<boolean>
  hasPlaybackSession: Ref<boolean>
}

export type PlayFunction = (
  track: TrackInfo,
  commandSource?: PlaybackCommandSource,
  startPositionMs?: number,
  forceResolve?: boolean,
  allowRememberedPosition?: boolean,
  requestedQueueIndex?: number,
  shuffleNavigation?: boolean,
) => Promise<void>

export type PlaybackControl = (commandSource?: PlaybackCommandSource) => Promise<void>
export type SeekControl = (ms: number, commandSource?: PlaybackCommandSource) => Promise<void>
