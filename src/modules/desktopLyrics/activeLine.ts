export interface DesktopLyricLineLike {
  startMs: number
  text?: string
  translation?: string
}

export function resolveDesktopLyricLine<T extends DesktopLyricLineLike>(
  lines: readonly T[],
  positionMs: number,
  lyricOffsetMs: number,
): T | null {
  const effectiveTimeMs = Math.max(
    0,
    (Number.isFinite(positionMs) ? positionMs : 0)
      + (Number.isFinite(lyricOffsetMs) ? lyricOffsetMs : 0),
  )
  let current: T | null = null
  let currentStartMs = Number.NEGATIVE_INFINITY

  for (const line of lines) {
    const startMs = Number(line.startMs)
    if (!Number.isFinite(startMs) || startMs > effectiveTimeMs || startMs < currentStartMs) continue
    current = line
    currentStartMs = startMs
  }

  return current
}

export function resolveNextDesktopLyricLine<T extends DesktopLyricLineLike>(
  lines: readonly T[],
  currentLine: T | null,
): T | null {
  if (!currentLine) return null
  const currentIndex = lines.indexOf(currentLine)
  return currentIndex >= 0 ? lines[currentIndex + 1] ?? null : null
}
