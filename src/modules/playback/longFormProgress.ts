// 长音频续播策略（对齐 Android LongFormPlaybackProgressPolicy）

export const LONG_FORM_MIN_DURATION_MS = 15 * 60 * 1000
const MIN_RESUME_POSITION_MS = 5_000
const COMPLETION_TOLERANCE_MS = 30_000

/** 起播位置：明确请求的位置优先；否则 15 分钟以上的内容从记住的位置继续 */
export function resolveLongFormResumePosition(
  enabled: boolean,
  durationMs: number,
  requestedPositionMs: number,
  rememberedPositionMs: number,
  allowRememberedPosition = true,
): number {
  const requested = Math.max(0, requestedPositionMs)
  if (requested > 0) return requested
  if (!allowRememberedPosition || !enabled || durationMs < LONG_FORM_MIN_DURATION_MS) return 0
  const remembered = Math.max(0, rememberedPositionMs)
  const latest = Math.max(0, durationMs - COMPLETION_TOLERANCE_MS)
  return remembered >= MIN_RESUME_POSITION_MS && remembered < latest ? remembered : 0
}

/**
 * 要记下的位置：null 表示不记（未开启、不是长内容、刚开头不足 5 秒），
 * 0 表示已经播到结尾附近，清掉之前记住的位置
 */
export function longFormPositionForPersistence(
  enabled: boolean,
  durationMs: number,
  positionMs: number,
): number | null {
  if (!enabled || durationMs < LONG_FORM_MIN_DURATION_MS) return null
  const position = Math.max(0, positionMs)
  const latest = Math.max(0, durationMs - COMPLETION_TOLERANCE_MS)
  if (position >= latest) return 0
  return position >= MIN_RESUME_POSITION_MS ? position : null
}
