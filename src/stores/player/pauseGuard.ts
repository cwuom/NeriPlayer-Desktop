const PAUSE_EVENT_GUARD_MS = 2500
const PAUSE_BACKWARD_TOLERANCE_MS = 250

/** 暂停后冻结显示位置：后端确认暂停前还可能报告更早的位置，不能让进度条回跳 */
export function createPauseGuard(
  currentRenderedPosition: () => number,
  setRenderedPosition: (position: number, durationOverride?: number) => number,
) {
  let pauseGuardUntil = 0
  let pauseFrozenMs: number | null = null

  function clearPauseGuard(): void {
    pauseFrozenMs = null
    pauseGuardUntil = 0
  }

  function freezeRenderedPosition(): number {
    const frozenMs = setRenderedPosition(currentRenderedPosition())
    pauseFrozenMs = frozenMs
    pauseGuardUntil = Date.now() + PAUSE_EVENT_GUARD_MS
    return frozenMs
  }

  function normalizePositionAfterPause(nextPositionMs: number): number | null {
    if (pauseFrozenMs === null) return nextPositionMs

    const now = Date.now()
    if (now >= pauseGuardUntil) {
      clearPauseGuard()
      return nextPositionMs
    }

    if (nextPositionMs < pauseFrozenMs - PAUSE_BACKWARD_TOLERANCE_MS) {
      return null
    }

    return pauseFrozenMs
  }

  return { clearPauseGuard, freezeRenderedPosition, normalizePositionAfterPause }
}
