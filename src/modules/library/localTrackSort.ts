export type LocalTrackSortBy = 'default' | 'modified' | 'title' | 'artist' | 'album' | 'duration'

const textOrder = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })

export function sortLocalTracks<T extends {
  title: string
  artist: string
  album: string
  durationMs: number
  fileModifiedMs?: number
}>(tracks: readonly T[], sortBy: LocalTrackSortBy, descending = false): T[] {
  const result = [...tracks]
  if (sortBy === 'default') return descending ? result.reverse() : result

  return result.sort((left, right) => {
    let order: number
    if (sortBy === 'modified') {
      const leftTime = left.fileModifiedMs
      const rightTime = right.fileModifiedMs
      const leftKnown = typeof leftTime === 'number' && Number.isFinite(leftTime) && leftTime > 0
      const rightKnown = typeof rightTime === 'number' && Number.isFinite(rightTime) && rightTime > 0
      if (!leftKnown || !rightKnown) return Number(rightKnown) - Number(leftKnown)
      order = leftTime - rightTime
    } else if (sortBy === 'duration') {
      order = left.durationMs - right.durationMs
    } else {
      order = textOrder.compare(left[sortBy], right[sortBy])
    }
    return descending ? -order : order
  })
}
