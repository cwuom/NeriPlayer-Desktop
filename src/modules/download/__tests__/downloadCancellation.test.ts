import { describe, expect, it } from 'vitest'
import {
  consumeResolvingCancellation,
  markResolvingTasksCancelled,
} from '@/modules/download/downloadCancellation'

const tokens: Record<string, string> = {
  'resolving-1': 'token-1',
  'resolving-2': 'token-2',
}

describe('download resolving cancellation', () => {
  it('marks only resolving tasks cancelled and consumes each token once', () => {
    const cancelled = new Set<string>()
    const resolvingIds = markResolvingTasksCancelled(
      [
        { trackId: 'resolving-1', status: 'resolving' },
        { trackId: 'downloading-1', status: 'downloading' },
        { trackId: 'resolving-2', status: 'resolving' },
      ],
      cancelled,
      trackId => tokens[trackId],
    )
    expect(resolvingIds).toEqual([
      { trackId: 'resolving-1', token: 'token-1' },
      { trackId: 'resolving-2', token: 'token-2' },
    ])
    expect([...cancelled]).toEqual(['token-1', 'token-2'])
    expect(consumeResolvingCancellation(cancelled, 'token-1')).toBe(true)
    expect(consumeResolvingCancellation(cancelled, 'token-1')).toBe(false)
    expect(consumeResolvingCancellation(cancelled, 'downloading-1')).toBe(false)
    expect([...cancelled]).toEqual(['token-2'])
  })
})
