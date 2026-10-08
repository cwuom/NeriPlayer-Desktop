import type { LyricLine, TrackInfo } from '@/stores/player'

export interface DesktopLyricsUpgrade {
  source?: string | null
  lines: LyricLine[]
}

interface DesktopLyricsDependencies {
  materialize: (track: TrackInfo) => Promise<LyricLine[] | null>
  cached: (track: TrackInfo) => Promise<LyricLine[] | null> | LyricLine[] | null
  fetch: (track: TrackInfo) => Promise<LyricLine[]>
  cache: (track: TrackInfo, lines: LyricLine[]) => void
  canUpgrade: (track: TrackInfo, lines: LyricLine[]) => boolean
  upgrade: (track: TrackInfo) => Promise<DesktopLyricsUpgrade>
  mergeUpgrade: (baseline: LyricLine[], upgrade: LyricLine[]) => LyricLine[]
  /** 升级真正替换显示前调用；被丢弃的升级不能改动歌词来源（来源决定默认偏移量） */
  adoptSource?: (track: TrackInfo, source: string | null) => void
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

    let baseline = existing.length ? existing : (await deps.cached(track)) || []
    if (!isCurrent()) return
    deps.onChange(baseline)
    if (!baseline.length) {
      try {
        baseline = await deps.fetch(track)
        if (!isCurrent()) return
        deps.onChange(baseline)
        if (baseline.length) deps.cache(track, baseline)
      } catch {
        const restored = await deps.cached(track)
        if (isCurrent()) deps.onChange(restored || baseline)
        return
      }
    }
    if (!isCurrent() || !deps.canUpgrade(track, baseline)) return
    void deps.upgrade(track).then(result => {
      if (!isCurrent() || !deps.canUpgrade(track, baseline) || !result.lines.length) return
      const upgraded = deps.mergeUpgrade(baseline, result.lines)
      deps.adoptSource?.(track, result.source ?? null)
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
