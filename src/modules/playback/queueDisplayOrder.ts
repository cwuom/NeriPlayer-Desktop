export function buildQueueDisplayOrder(
  queueLength: number,
  currentQueueIndex: number,
  shuffle: boolean,
  history: readonly number[],
  future: readonly number[],
  bag: readonly number[],
): { indices: number[]; currentIndex: number } {
  const valid = (index: number) => Number.isInteger(index) && index >= 0 && index < queueLength
  const originalOrder = Array.from({ length: queueLength }, (_, index) => index)
  if (!shuffle) return { indices: originalOrder, currentIndex: valid(currentQueueIndex) ? currentQueueIndex : -1 }

  const upcoming: number[] = []
  const upcomingSlots = new Set<number>()
  for (const index of [...future].reverse()) {
    if (!valid(index)) continue
    upcoming.push(index)
    upcomingSlots.add(index)
  }
  for (const index of bag) {
    if (!valid(index)) continue
    upcoming.push(index)
    upcomingSlots.add(index)
  }

  const played: number[] = []
  const seen = new Set(upcomingSlots)
  if (valid(currentQueueIndex)) seen.add(currentQueueIndex)
  for (const index of [...history, ...originalOrder]) {
    if (!valid(index) || seen.has(index)) continue
    played.push(index)
    seen.add(index)
  }
  const currentIndex = valid(currentQueueIndex) ? played.length : -1
  return {
    indices: [...played, ...(currentIndex >= 0 ? [currentQueueIndex] : []), ...upcoming],
    currentIndex,
  }
}
