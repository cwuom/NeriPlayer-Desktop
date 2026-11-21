import { describe, expect, it, vi } from 'vitest'
import { formatSyncTime, parseGitHubRepo } from '@/components/settings/useSyncSettings'

// 纯函数不碰 store；真实 store 模块在导入时就会读 localStorage
vi.mock('@/stores/settings', () => ({ useSettingsStore: vi.fn() }))
vi.mock('@/stores/sync', () => ({ useSyncStore: vi.fn() }))
vi.mock('@/stores/player', () => ({ usePlayerStore: vi.fn() }))
vi.mock('@/stores/listenTogether', () => ({ useListenTogetherStore: vi.fn() }))

describe('parseGitHubRepo', () => {
  it('splits owner/repo', () => {
    expect(parseGitHubRepo('neri/backup')).toEqual({ owner: 'neri', repo: 'backup' })
  })

  it('rejects anything that is not exactly two non-empty segments', () => {
    for (const value of ['', 'backup', 'neri/', '/backup', 'a/b/c']) expect(parseGitHubRepo(value)).toBeNull()
  })
})

describe('formatSyncTime', () => {
  it('formats with the UI locale and leaves never-synced timestamps empty', () => {
    expect(formatSyncTime(0, 'en')).toBe('')
    const ms = Date.UTC(2026, 0, 2, 3, 4, 5)
    expect(formatSyncTime(ms, 'en')).toBe(new Date(ms).toLocaleString('en'))
  })
})
