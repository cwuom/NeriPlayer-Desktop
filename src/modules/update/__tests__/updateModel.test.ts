import { describe, expect, it } from 'vitest'
import {
  AUTO_CHECK_INTERVAL_MS,
  isIncremental,
  plannedDownloadBytes,
  progressPercent,
  savedPercent,
  shouldAutoCheck,
  shouldPrompt,
  updateErrorCode,
  updateErrorMessage,
  type UpdateCheck,
} from '@/modules/update/updateModel'

const available: UpdateCheck = {
  status: 'available',
  currentVersion: '1.0.0',
  latestVersion: '1.1.0',
  releaseUrl: 'https://example.invalid/r',
  inApp: true,
  unsupportedReason: null,
  summary: { downloadBytes: 5, fullBytes: 100, changedFiles: 1, totalFiles: 9 },
}

describe('update model', () => {
  it('reads structured backend errors and falls back to unknown', () => {
    expect(updateErrorCode({ code: 'network', message: 'offline' })).toBe('network')
    expect(updateErrorCode({ code: 'made-up' })).toBe('unknown')
    expect(updateErrorCode('boom')).toBe('unknown')
    expect(updateErrorCode(null)).toBe('unknown')
    expect(updateErrorMessage({ code: 'io', message: 'disk full' })).toBe('disk full')
    expect(updateErrorMessage(new Error('x'))).toBe('x')
    expect(updateErrorMessage('plain')).toBe('plain')
  })

  it('only advertises incremental updates that save a real share of the download', () => {
    expect(savedPercent(available.summary)).toBe(95)
    expect(isIncremental(available.summary)).toBe(true)
    const almostFull = { downloadBytes: 95, fullBytes: 100, changedFiles: 9, totalFiles: 9 }
    expect(isIncremental(almostFull)).toBe(false)
    expect(savedPercent(null)).toBe(0)
    expect(savedPercent({ downloadBytes: 0, fullBytes: 0, changedFiles: 0, totalFiles: 0 })).toBe(0)
    expect(savedPercent({ downloadBytes: 200, fullBytes: 100, changedFiles: 1, totalFiles: 1 })).toBe(0)
  })

  it('reads the planned download size and falls back to zero without a plan', () => {
    expect(plannedDownloadBytes(available)).toBe(5)
    expect(plannedDownloadBytes({ ...available, summary: null })).toBe(0)
    expect(plannedDownloadBytes(null)).toBe(0)
  })

  it('clamps download progress and treats an empty download as done', () => {
    expect(progressPercent(null)).toBe(0)
    expect(progressPercent({ downloaded: 0, total: 0 })).toBe(100)
    expect(progressPercent({ downloaded: 50, total: 200 })).toBe(25)
    expect(progressPercent({ downloaded: 999, total: 200 })).toBe(100)
  })

  it('rate-limits automatic checks and recovers from a clock moved backwards', () => {
    const now = 10 * AUTO_CHECK_INTERVAL_MS
    expect(shouldAutoCheck(null, now)).toBe(true)
    expect(shouldAutoCheck(now - 1000, now)).toBe(false)
    expect(shouldAutoCheck(now - AUTO_CHECK_INTERVAL_MS, now)).toBe(true)
    expect(shouldAutoCheck(now + 1000, now)).toBe(true)
    expect(shouldAutoCheck(Number.NaN, now)).toBe(true)
  })

  it('keeps quiet about a skipped version unless the user asked', () => {
    expect(shouldPrompt(available, null, false)).toBe(true)
    expect(shouldPrompt(available, '1.1.0', false)).toBe(false)
    expect(shouldPrompt(available, '1.1.0', true)).toBe(true)
    expect(shouldPrompt(available, '1.0.5', false)).toBe(true)
    expect(shouldPrompt({ ...available, status: 'upToDate' }, null, true)).toBe(false)
  })
})
