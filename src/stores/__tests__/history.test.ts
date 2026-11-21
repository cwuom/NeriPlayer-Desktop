import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TrackInfo } from '@/types/track'
import type { UserDataSnapshot } from '@/modules/persistence/userData'
import { historyEntryKey, useHistoryStore, type HistorySyncPayload } from '@/stores/history'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'

const mocks = vi.hoisted(() => ({
  preloaded: null as UserDataSnapshot | null,
  persisted: [] as Array<{ command: string; args: Record<string, unknown> }>,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (command: string) => { throw new Error(`unexpected invoke: ${command}`) },
}))
vi.mock('@/modules/persistence/userData', () => ({
  LEGACY_HISTORY_KEY: 'neri:play-history',
  LEGACY_HISTORY_DELETIONS_KEY: 'neri:play-history-deletions',
  preloadedUserData: () => mocks.preloaded,
  persistUserData: async (command: string, args: Record<string, unknown>) => {
    mocks.persisted.push({ command, args })
  },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))

function track(id: string, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return {
    id, title: `title ${id}`, artist: 'artist', album: 'album', durationMs: 180_000,
    coverUrl: 'https://img/cover.jpg', audioUrl: '', ...overrides,
  }
}

function snapshot(entries: unknown[], deletions: unknown[] = []): UserDataSnapshot {
  return {
    playbackState: null,
    history: { entries: entries as UserDataSnapshot['history']['entries'], deletions: deletions as UserDataSnapshot['history']['deletions'] },
    lyricOffsets: {},
    migrated: { playbackState: true, history: true, lyricOffsets: true },
  }
}

const ids = (entries: ReadonlyArray<{ track: TrackInfo }>) => entries.map(entry => entry.track.id)

describe('history store', () => {
  beforeEach(() => {
    mocks.preloaded = null
    mocks.persisted = []
    vi.stubGlobal('localStorage', new MemoryStorage())
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  describe('hydration', () => {
    it('normalizes snake_case records, drops invalid rows and keeps the newest duplicate', () => {
      mocks.preloaded = snapshot([
        { track: { id: 'netease:1', title: 'One', duration_ms: 1000, cover_url: 'c1', sync_payload: { a: 1 } }, played_at: 300, resume_position_ms: 1500.4 },
        { track: { id: 'netease:1', title: 'Older copy' }, playedAt: 200 },
        { track: { id: '' }, playedAt: 100 },
        { track: { id: 'netease:2' }, playedAt: 0 },
        null,
        'garbage',
        { track: { id: 'netease:3', source: 'netease', playlist_key: 42 }, playedAt: 50, resumePositionMs: -5 },
      ], [
        { track: { id: 'netease:9' }, deleted_at: 400 },
        { track: { id: 'netease:8' }, deletedAt: 0 },
      ])

      const history = useHistoryStore()

      expect(ids(history.entries)).toEqual(['netease:1', 'netease:3'])
      expect(history.entries[0]).toMatchObject({
        playedAt: 300,
        resumePositionMs: 1500,
        track: { title: 'One', durationMs: 1000, coverUrl: 'c1', syncPayload: { a: 1 } },
      })
      // 非字符串的 playlist_key 不再原样塞进 TrackInfo，负的续播位置钳到 0
      expect(history.entries[1].track.playlistKey).toBeUndefined()
      expect(history.entries[1].track.source).toBe('netease')
      expect(history.entries[1].resumePositionMs).toBe(0)
      expect(history.deletions.map(deletion => deletion.track.id)).toEqual(['netease:9'])
    })

    it('reads the legacy localStorage copy without a database', () => {
      localStorage.setItem('neri:play-history', JSON.stringify([{ track: { id: 'qq:1' }, playedAt: 10 }]))

      const history = useHistoryStore()
      history.record(track('qq:2'))

      expect(ids(history.entries)).toEqual(['qq:2', 'qq:1'])
      // 没有数据库时整表写回 localStorage，而不是发数据库命令
      expect(mocks.persisted).toEqual([])
      expect(JSON.parse(localStorage.getItem('neri:play-history') ?? '[]')).toHaveLength(2)
    })

    it('treats a corrupted legacy copy as empty history', () => {
      localStorage.setItem('neri:play-history', JSON.stringify([{ track: { id: 'qq:1' }, playedAt: 10 }]))
      localStorage.setItem('neri:play-history-deletions', '{not json')

      const history = useHistoryStore()

      expect(history.entries).toEqual([])
      expect(history.deletions).toEqual([])
    })
  })

  describe('local mutations', () => {
    it('records a play at the front, keeps the resume position and lifts a deletion', () => {
      mocks.preloaded = snapshot(
        [{ track: { id: 'netease:1' }, playedAt: 100, resumePositionMs: 9000 }, { track: { id: 'netease:2' }, playedAt: 50 }],
        [{ track: { id: 'netease:3' }, deletedAt: 40 }],
      )
      const history = useHistoryStore()
      const before = history.syncSnapshotEpoch()

      history.record(track('netease:2'))
      history.record(track('netease:3'))

      expect(ids(history.entries)).toEqual(['netease:3', 'netease:2', 'netease:1'])
      expect(history.deletions).toEqual([])
      expect(history.rememberedPosition(track('netease:1'))).toBe(9000)
      expect(history.syncSnapshotEpoch()).toBe(before + 2)
      expect(mocks.persisted.map(call => call.command)).toEqual(['record_play_history', 'record_play_history'])
    })

    it('keeps local files out of the sync snapshot', () => {
      mocks.preloaded = snapshot([
        { track: { id: 'local:/music/a.flac' }, playedAt: 3 },
        { track: { id: 'netease:1', source: 'netease' }, playedAt: 2 },
      ])
      const history = useHistoryStore()

      const sent = history.getSyncSnapshot()

      expect(sent.entries.map(entry => entry.track.id)).toEqual(['netease:1'])
      expect(sent.entries[0].track).toMatchObject({ source: 'netease', duration_ms: 0, cover_url: null })
    })
  })

  describe('applySyncPayload', () => {
    const remote: HistorySyncPayload = {
      entries: [
        { track: { id: 'netease:10', title: 'Remote' }, playedAt: 500, resumePositionMs: 1200 },
        { track: { id: 'netease:11' }, playedAt: 100 },
      ],
      deletions: [{ songId: '11', album: '', deletedAt: 200 }],
    }

    it('skips empty or superseded results', async () => {
      const history = useHistoryStore()

      await expect(history.applySyncPayload(null)).resolves.toBe('skipped')
      await expect(history.applySyncPayload(remote, () => false)).resolves.toBe('skipped')
      expect(history.entries).toEqual([])
    })

    it('replaces remote history, honours newer deletions and keeps local files', async () => {
      mocks.preloaded = snapshot([
        { track: { id: 'local:/a.mp3' }, playedAt: 50 },
        { track: { id: 'netease:99' }, playedAt: 40 },
      ])
      const history = useHistoryStore()

      const outcome = await history.applySyncPayload(remote, () => true, history.syncSnapshotEpoch())

      expect(outcome).toBe('applied')
      // netease:99 不在合并结果里，合并结果就是远端的权威列表；netease:11 的删除晚于播放
      expect(ids(history.entries)).toEqual(['netease:10', 'local:/a.mp3'])
      expect(history.entries[0].resumePositionMs).toBe(1200)
      expect(history.deletions.map(deletion => [deletion.track.id, deletion.deletedAt])).toEqual([['netease:11', 200]])
      const replaced = mocks.persisted.find(call => call.command === 'replace_play_history')
      expect(replaced?.args).toMatchObject({ history: { entries: [{ playedAt: 500, resumePositionMs: 1200 }, { playedAt: 50 }] } })
    })

    it('defers when the user changed history after the snapshot was taken', async () => {
      mocks.preloaded = snapshot([{ track: { id: 'netease:1' }, playedAt: 10 }])
      const history = useHistoryStore()
      const epoch = history.syncSnapshotEpoch()
      history.record(track('netease:2'))

      await expect(history.applySyncPayload(remote, () => true, epoch)).resolves.toBe('deferred')

      expect(ids(history.entries)).toEqual(['netease:2', 'netease:1'])
      expect(mocks.persisted.some(call => call.command === 'replace_play_history')).toBe(false)
    })

    it('defers when a local play lands while deletions are still being matched', async () => {
      const history = useHistoryStore()

      const applying = history.applySyncPayload(remote, () => true, history.syncSnapshotEpoch())
      history.record(track('netease:7'))

      await expect(applying).resolves.toBe('deferred')
      expect(ids(history.entries)).toEqual(['netease:7'])
    })

    it('gives up without changes when a newer sync takes over mid-way', async () => {
      const history = useHistoryStore()
      let checks = 0

      const outcome = await history.applySyncPayload(remote, () => ++checks <= 1)

      expect(outcome).toBe('skipped')
      expect(history.entries).toEqual([])
      expect(mocks.persisted).toEqual([])
    })
  })

  it('uses the Bilibili page id as part of the identity', () => {
    const part = track('bilibili:BV1xx', { album: 'Bilibili|123' })
    expect(historyEntryKey(part)).toBe('bilibili:BV1xx#123')
    expect(historyEntryKey(track('bilibili:BV1xx', { syncPayload: { subAudioId: ' 456 ' } }))).toBe('bilibili:BV1xx#456')
    expect(historyEntryKey(track('netease:1'))).toBe('netease:1')
  })
})
