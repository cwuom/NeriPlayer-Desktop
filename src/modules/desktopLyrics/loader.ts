import type { LyricLine, TrackInfo } from '@/stores/player'

interface DesktopLyricsDependencies {
  materialize: (track: TrackInfo) => Promise<LyricLine[] | null>
  cached: (track: TrackInfo) => LyricLine[] | null
  fetch: (track: TrackInfo) => Promise<LyricLine[]>
  cache: (track: TrackInfo, lines: LyricLine[]) => void
  canUpgrade: (track: TrackInfo, lines: LyricLine[]) => boolean
  upgrade: (track: TrackInfo) => Promise<LyricLine[]>
  mergeUpgrade: (baseline: LyricLine[], upgrade: LyricLine[]) => LyricLine[]
  onChange: (lines: LyricLine[]) => void
}

export function createDesktopLyricsLoader(deps: DesktopLyricsDependencies) {
  let generation = 0
  let disposed = false

  async function load(track: TrackInfo | null, existing: LyricLine[] = []) {
    if (disposed) return
    const request = ++generation
    const isCurrent = () => !disposed && generation === request
    deps.onChange([])
    if (!track) return

    let synced: LyricLine[] | null
    try {
      synced = await deps.materialize(track)
    } catch {
      // 有同步原文却解析失败时保持空态，避免在线结果覆盖用户编辑
      if (isCurrent()) deps.onChange([])
      return
    }
    if (!isCurrent()) return
    if (synced !== null) {
      deps.onChange(synced)
      if (synced.length) deps.cache(track, synced)
      return
    }

    let baseline = existing.length ? existing : deps.cached(track) || []
    deps.onChange(baseline)
    if (!baseline.length) {
      try {
        baseline = await deps.fetch(track)
        if (!isCurrent()) return
        deps.onChange(baseline)
        if (baseline.length) deps.cache(track, baseline)
      } catch {
        if (isCurrent()) deps.onChange(deps.cached(track) || baseline)
        return
      }
    }
    if (!isCurrent() || !deps.canUpgrade(track, baseline)) return
    void deps.upgrade(track).then(lines => {
      if (!isCurrent() || !deps.canUpgrade(track, baseline) || !lines.length) return
      const upgraded = deps.mergeUpgrade(baseline, lines)
      deps.onChange(upgraded)
      deps.cache(track, upgraded)
    }).catch(() => {
      // 逐字升级失败不撤下已显示的歌词
    })
  }

  return {
    load,
    dispose() { disposed = true; generation++ },
  }
}
