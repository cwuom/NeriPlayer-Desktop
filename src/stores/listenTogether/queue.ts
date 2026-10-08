import type {
  ListenTogetherQueueMutation,
  ListenTogetherQueueOperation,
  ListenTogetherQueueReference,
  ListenTogetherTrack,
} from './protocol'

const MAX_QUEUE_SIZE = 2000
const MAX_MUTATION_OPERATIONS = 64
const trackReferences = new WeakMap<object, ListenTogetherQueueReference>()

type QueueEntry = { track: ListenTogetherTrack; reference: ListenTogetherQueueReference }

export interface ListenTogetherQueueMutationResult {
  queue: ListenTogetherTrack[]
  currentIndex: number
  targetCurrentIndex: number | null
  currentRemoved: boolean
}

export interface ListenTogetherQueueMutationPlan {
  mutation: ListenTogetherQueueMutation
  requiresSnapshotFallback: boolean
  needsCurrentSelectionAfterApply: boolean
}

export function setLtQueueReference(track: ListenTogetherTrack, reference: unknown): void {
  if (validReference(reference) && reference.stableKey === track.stableKey) {
    trackReferences.set(track, { ...reference })
  }
}

export function getLtQueueReference(track: ListenTogetherTrack): ListenTogetherQueueReference | undefined {
  const reference = trackReferences.get(track)
  return reference?.stableKey === track.stableKey ? { ...reference } : undefined
}

export function queueReferences(queue: readonly ListenTogetherTrack[]): ListenTogetherQueueReference[] {
  const counts = new Map<string, number>()
  return queue.map(track => {
    const occurrence = counts.get(track.stableKey) || 0
    counts.set(track.stableKey, occurrence + 1)
    return { stableKey: track.stableKey, occurrence }
  })
}

function referenceKey(reference: ListenTogetherQueueReference): string {
  return JSON.stringify([reference.stableKey, reference.occurrence])
}

function validReference(value: unknown): value is ListenTogetherQueueReference {
  if (!value || typeof value !== 'object') return false
  const reference = value as Partial<ListenTogetherQueueReference>
  return typeof reference.stableKey === 'string' && !!reference.stableKey.trim()
    && typeof reference.occurrence === 'number' && Number.isInteger(reference.occurrence)
    && reference.occurrence >= 0 && reference.occurrence < MAX_QUEUE_SIZE
}

function sameTrackMetadata(left: ListenTogetherTrack, right: ListenTogetherTrack): boolean {
  return left.stableKey === right.stableKey && left.name === right.name
    && left.artist === right.artist && left.album === right.album
    && left.durationMs === right.durationMs && left.coverUrl === right.coverUrl
    && left.mediaUri === right.mediaUri
}

function validOperation(operation: ListenTogetherQueueOperation): boolean {
  if (!operation || typeof operation !== 'object') return false
  if (operation.type === 'remove') return validReference(operation.target)
  if (operation.type === 'remove_many' || operation.type === 'reorder') {
    return Array.isArray(operation.order) && operation.order.length <= MAX_QUEUE_SIZE
      && operation.order.every(validReference)
  }
  if (operation.type !== 'insert' && operation.type !== 'move') return false
  if (!['prepend', 'before', 'append'].includes(operation.placement || '')) return false
  if (operation.placement === 'before' && !validReference(operation.anchor)) return false
  return operation.type === 'move' ? validReference(operation.target) : !!operation.track?.stableKey?.trim()
}

export function findQueueReferenceIndex(
  queue: readonly ListenTogetherTrack[],
  reference?: ListenTogetherQueueReference | null,
): number {
  if (!validReference(reference)) return -1
  return queueReferences(queue).findIndex(candidate => referenceKey(candidate) === referenceKey(reference!))
}

export function applyListenTogetherQueueMutation({
  roomQueue,
  roomCurrentIndex,
  roomVersion,
  mutation,
  targetCurrentStableKey,
}: {
  roomQueue: readonly ListenTogetherTrack[]
  roomCurrentIndex: number
  roomVersion?: number
  mutation: ListenTogetherQueueMutation
  targetCurrentStableKey?: string | null
}): ListenTogetherQueueMutationResult | null {
  if (!Number.isInteger(mutation.baseRoomVersion) || mutation.baseRoomVersion < 0
    || (roomVersion !== undefined && mutation.baseRoomVersion > roomVersion)
    || !Array.isArray(mutation.operations)
    || mutation.operations.length > MAX_MUTATION_OPERATIONS
    || !mutation.operations.every(validOperation)
    || (mutation.targetCurrent != null && !validReference(mutation.targetCurrent))) return null

  // 引用固定到修改前的槽位，删除前一个重复项不会重编号后一个重复项
  const baseReferences = queueReferences(roomQueue)
  const entries = roomQueue.map((track, index): QueueEntry => ({
    track,
    reference: baseReferences[index],
  }))
  const references = new Map(entries.map(entry => [referenceKey(entry.reference), entry]))
  const resolve = (reference?: ListenTogetherQueueReference | null) => validReference(reference)
    ? references.get(referenceKey(reference!)) : undefined
  const previousCurrent = entries[roomCurrentIndex]
  const targetCurrent = resolve(mutation.targetCurrent)
  const next = entries.slice()
  const insertIndex = (operation: ListenTogetherQueueOperation) => {
    const anchor = resolve(operation.anchor)
    const anchorIndex = anchor ? next.indexOf(anchor) : -1
    if (operation.placement === 'prepend') return 0
    if (operation.placement === 'before' && anchorIndex >= 0) return anchorIndex
    return next.length
  }

  for (const operation of mutation.operations) {
    if (operation.type === 'remove' || operation.type === 'move') {
      const target = resolve(operation.target)
      const index = target ? next.indexOf(target) : -1
      if (index < 0) continue
      const [entry] = next.splice(index, 1)
      if (operation.type === 'move') next.splice(insertIndex(operation), 0, entry)
    } else if (operation.type === 'remove_many') {
      for (const reference of operation.order || []) {
        const entry = resolve(reference)
        const index = entry ? next.indexOf(entry) : -1
        if (index >= 0) next.splice(index, 1)
      }
    } else if (operation.type === 'insert') {
      const track = operation.track
      if (!track?.stableKey?.trim() || next.length >= MAX_QUEUE_SIZE) continue
      next.splice(insertIndex(operation), 0, {
        track,
        reference: { stableKey: track.stableKey, occurrence: -1 },
      })
    } else if (operation.type === 'reorder') {
      const requested = [...new Set((operation.order || []).map(resolve))]
        .filter((entry): entry is QueueEntry => !!entry && next.includes(entry))
      const selected = new Set(requested)
      const slots = next.flatMap((entry, index) => selected.has(entry) ? [index] : [])
      requested.forEach((entry, index) => { next[slots[index]] = entry })
    }
  }

  const currentRemoved = !!previousCurrent && !next.includes(previousCurrent)
  const targetCurrentIndex = targetCurrent ? next.indexOf(targetCurrent) : -1
  const retainedCurrentIndex = previousCurrent ? next.indexOf(previousCurrent) : -1
  const preferredIndex = targetCurrentStableKey
    ? next.findIndex(entry => entry.track.stableKey === targetCurrentStableKey) : -1
  const fallbackIndex = Math.min(Math.max(roomCurrentIndex, 0), next.length - 1)
  return {
    queue: next.map(entry => entry.track),
    currentIndex: retainedCurrentIndex >= 0 ? retainedCurrentIndex
      : targetCurrentIndex >= 0 ? targetCurrentIndex
        : preferredIndex >= 0 ? preferredIndex : fallbackIndex,
    targetCurrentIndex: targetCurrentIndex >= 0 ? targetCurrentIndex : null,
    currentRemoved,
  }
}

export function buildListenTogetherQueueMutationPlan(
  baseState: { version: number; queue: readonly ListenTogetherTrack[] },
  targetQueue: readonly ListenTogetherTrack[],
  targetCurrentIndex: number,
): ListenTogetherQueueMutationPlan {
  const mutation: ListenTogetherQueueMutation = {
    baseRoomVersion: Number.isInteger(baseState.version) ? Math.max(0, baseState.version) : 0,
    operations: [],
  }
  if (targetQueue.length > MAX_QUEUE_SIZE
    || [...baseState.queue, ...targetQueue].some(track => !track.stableKey?.trim())) {
    return { mutation, requiresSnapshotFallback: true, needsCurrentSelectionAfterApply: false }
  }

  const references = queueReferences(baseState.queue)
  const available = baseState.queue.map((track, index) => ({ track, reference: references[index] }))
  const matched = new Set<QueueEntry>()
  const targets = targetQueue.map(track => ({
    track,
    explicitReference: getLtQueueReference(track),
    reference: undefined as ListenTogetherQueueReference | undefined,
  }))
  const explicitKeys = new Set<string>()
  for (const target of targets) {
    if (!target.explicitReference) continue
    const key = referenceKey(target.explicitReference)
    if (explicitKeys.has(key)) {
      return { mutation, requiresSnapshotFallback: true, needsCurrentSelectionAfterApply: false }
    }
    explicitKeys.add(key)
    const entry = available.find(candidate => referenceKey(candidate.reference) === key)
    if (entry) {
      matched.add(entry)
      target.reference = entry.reference
    }
  }
  for (const target of targets) {
    // 先预留已知槽位，新插入的同名项不能抢占旧项的基准引用
    if (target.explicitReference) continue
    const track = target.track
    const entry = available.find(candidate => !matched.has(candidate) && candidate.track === track)
      || available.find(candidate => !matched.has(candidate) && sameTrackMetadata(candidate.track, track))
      || available.find(candidate => !matched.has(candidate)
        && candidate.track.stableKey === track.stableKey)
    if (entry) {
      matched.add(entry)
      target.reference = entry.reference
    }
  }
  const removed = available.filter(entry => !matched.has(entry)).map(entry => entry.reference)
  const operations: ListenTogetherQueueOperation[] = removed.length > MAX_MUTATION_OPERATIONS
    ? [{ type: 'remove_many', order: removed }]
    : removed.map(target => ({ type: 'remove', target }))
  const desired = targets.flatMap(entry => entry.reference ? [entry.reference] : [])
  const working = available.filter(entry => matched.has(entry)).map(entry => entry.reference)
  const moves: ListenTogetherQueueOperation[] = []
  for (let index = 0; index < desired.length && moves.length <= MAX_MUTATION_OPERATIONS; index++) {
    if (referenceKey(working[index]) === referenceKey(desired[index])) continue
    const from = working.findIndex(reference => referenceKey(reference) === referenceKey(desired[index]))
    const [target] = working.splice(from, 1)
    const anchor = working[index]
    working.splice(index, 0, target)
    moves.push({ type: 'move', target, anchor, placement: anchor ? 'before' : 'append' })
  }
  const insertions: ListenTogetherQueueOperation[] = targets.flatMap((entry, index) => {
    if (entry.reference) return []
    const anchor = targets.slice(index + 1).find(candidate => candidate.reference)?.reference
    return [{ type: 'insert', track: entry.track, anchor, placement: anchor ? 'before' : 'append' }]
  })
  operations.push(...(operations.length + moves.length + insertions.length <= MAX_MUTATION_OPERATIONS
    ? moves : moves.length ? [{ type: 'reorder', order: desired }] : []))
  operations.push(...insertions)
  mutation.operations = operations
  const current = targets[targetCurrentIndex]
  mutation.targetCurrent = current?.reference
  return {
    mutation,
    requiresSnapshotFallback: operations.length > MAX_MUTATION_OPERATIONS,
    needsCurrentSelectionAfterApply: !!current && !current.reference
      && baseState.queue.some(track => track.stableKey === current.track.stableKey),
  }
}
