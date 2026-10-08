import type { PlaybackResolution } from './playbackSource'

/** 取流失败的原因（对齐 Android 播放失败提示的几类文案） */
export type PlaybackFailureReason =
  | 'no_permission'
  | 'no_play_url'
  | 'video_info_unavailable'
  | 'url_error'
  | 'requires_login'

/** 解析层抛出的带原因错误，播放层据此选择本地化提示 */
export class PlaybackFailure extends Error {
  constructor(readonly reason: PlaybackFailureReason, message: string) {
    super(message)
    this.name = 'PlaybackFailure'
  }
}

const FAILURE_MESSAGE_KEYS: Record<PlaybackFailureReason, string> = {
  no_permission: 'player.playback_no_permission',
  no_play_url: 'player.playback_no_play_url',
  video_info_unavailable: 'player.playback_video_info_unavailable',
  url_error: 'player.playback_url_error',
  requires_login: 'player.playback_login_required',
}

export function playbackFailureMessageKey(reason: PlaybackFailureReason): string {
  return FAILURE_MESSAGE_KEYS[reason]
}

export const PLAYBACK_FAILURE_REASONS = Object.keys(FAILURE_MESSAGE_KEYS) as PlaybackFailureReason[]

export const BILI_VIDEO_INFO_UNAVAILABLE = 'Bilibili video info unavailable'

// 对齐 Android SongUrlResolutionRetry：失败后最多再试 5 次，间隔 250ms × 次数
export const SONG_URL_RETRY_COUNT = 5
export const SONG_URL_RETRY_DELAY_MS = 250

/** 只重试可能是瞬时的失败；被新请求取代、身份不符、受限资源重试也不会成功 */
export function isRetryablePlaybackFailure(result: PlaybackResolution): boolean {
  return result.type === 'failure'
    && result.retryable
    && !/superseded|identity mismatch/i.test(result.message)
}

export interface SongUrlRetryOptions {
  retryCount?: number
  retryDelayMs?: number
  delay?: (ms: number) => Promise<void>
  /** 请求已作废（缓存清空、被强制刷新取代）时停止重试 */
  shouldContinue?: () => boolean
}

const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms))

export async function retrySongUrlResolution(
  attempt: () => Promise<PlaybackResolution>,
  options: SongUrlRetryOptions = {},
): Promise<PlaybackResolution> {
  const retryCount = options.retryCount ?? SONG_URL_RETRY_COUNT
  const retryDelayMs = options.retryDelayMs ?? SONG_URL_RETRY_DELAY_MS
  const delay = options.delay ?? wait
  const shouldContinue = options.shouldContinue ?? (() => true)
  for (let attemptIndex = 0; ; attemptIndex++) {
    const result = await attempt()
    if (attemptIndex >= retryCount || !isRetryablePlaybackFailure(result) || !shouldContinue()) return result
    await delay(retryDelayMs * (attemptIndex + 1))
    if (!shouldContinue()) return result
  }
}

/** 同一曲目在冷却期内再次需要刷新地址时不再重试（对齐 Android URL_REFRESH_COOLDOWN_MS） */
export const PLAYBACK_REFRESH_COOLDOWN_MS = 10_000

export function shouldThrottlePlaybackRefresh(
  last: { key: string; at: number } | null,
  key: string,
  now: number,
  cooldownMs = PLAYBACK_REFRESH_COOLDOWN_MS,
): boolean {
  return !!last && last.key === key && now - last.at < cooldownMs
}
