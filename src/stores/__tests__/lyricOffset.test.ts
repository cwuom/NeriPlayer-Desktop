import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import type { TrackInfo } from '@/types/track'
import type { UserDataSnapshot } from '@/modules/persistence/userData'
import { rememberLyricSource } from '@/modules/lyrics/lyricSource'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'
import { flushPromises } from './storeTestUtils'

interface SettingsStub {
  isHydrated: boolean
  cloudMusicOffset: number
  qqMusicOffset: number
  kugouOffset: number
  lrclibOffset: number
  amllTtmlOffset: number
  biliCcSubtitleOffset: number
}

interface PlayerStub {
  currentTrack: TrackInfo | null
  patchCurrentTrackSyncPayload: (payload: Record<string, unknown>) => void
}

const mocks = vi.hoisted(() => ({
  settings: null as SettingsStub | null,
  player: null as PlayerStub | null,
  preloaded: null as UserDataSnapshot | null,
  persisted: [] as Array<{ command: string; args: Record<string, unknown> }>,
  payloadWrites: [] as Array<Record<string, unknown> | null | undefined>,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (command: string) => { throw new Error(`unexpected invoke: ${command}`) },
}))
vi.mock('@/utils/logger', () => ({
  createLogger: () => ({ trace() {}, debug() {}, info() {}, warn() {}, error() {} }),
}))
vi.mock('@/stores/settings', () => ({ useSettingsStore: () => mocks.settings }))
vi.mock('@/stores/player', () => ({ usePlayerStore: () => mocks.player }))
vi.mock('@/modules/lyrics/syncTrackPayload', () => ({
  persistTrackSyncPayload: async (track: TrackInfo | null) => { mocks.payloadWrites.push(track?.syncPayload) },
}))
vi.mock('@/modules/persistence/userData', () => ({
  LEGACY_LYRIC_OFFSETS_KEY: 'neri.lyric-user-offsets',
  preloadedUserData: () => mocks.preloaded,
  persistUserData: async (command: string, args: Record<string, unknown>) => {
    mocks.persisted.push({ command, args })
  },
}))

const OFFSETS_KEY = 'neri.lyric-user-offsets'
const META_KEY = 'neri:lyric-offset-meta'
const LEGACY_SOURCES_KEY = 'neri:lyric-offset-sources'

function track(id: string, overrides: Partial<TrackInfo> = {}): TrackInfo {
  return { id, title: `title ${id}`, artist: 'artist', album: '', durationMs: 1000, coverUrl: '', audioUrl: '', ...overrides }
}

function snapshot(lyricOffsets: Record<string, number>): UserDataSnapshot {
  return {
    playbackState: null,
    history: { entries: [], deletions: [] },
    lyricOffsets,
    migrated: { playbackState: true, history: true, lyricOffsets: true },
  }
}

describe('lyricOffset store', () => {
  let storage: MemoryStorage
  let settings: SettingsStub
  let player: PlayerStub
  const remembered: TrackInfo[] = []

  const storedOffsets = () => JSON.parse(storage.getItem(OFFSETS_KEY) ?? 'null') as Record<string, number> | null

  function showLyricsFrom(target: TrackInfo, source: string) {
    remembered.push(target)
    rememberLyricSource(target, source)
  }

  async function hydrate() {
    settings.isHydrated = true
    await flushPromises()
  }

  beforeEach(() => {
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    settings = reactive<SettingsStub>({
      isHydrated: false, cloudMusicOffset: 1000, qqMusicOffset: 500, kugouOffset: 0, lrclibOffset: 0, amllTtmlOffset: 200, biliCcSubtitleOffset: 0,
    })
    mocks.settings = settings
    player = reactive<PlayerStub>({
      currentTrack: null,
      patchCurrentTrackSyncPayload: (payload) => {
        if (player.currentTrack) player.currentTrack = { ...player.currentTrack, syncPayload: payload }
      },
    })
    mocks.player = player
    mocks.preloaded = null
    mocks.persisted = []
    mocks.payloadWrites = []
    setActivePinia(createPinia())
  })

  afterEach(() => {
    // 歌词来源表是模块级状态，逐个清掉，避免影响后面的用例
    for (const target of remembered.splice(0)) rememberLyricSource(target, null)
    vi.unstubAllGlobals()
  })

  describe('loading', () => {
    it('sanitizes the stored per-track deltas', () => {
      storage.setItem(OFFSETS_KEY, JSON.stringify({ 'netease:1': 200, 'netease:2': 0, 'qq:3': '150.4', '': 5, 'x:4': 99_999, 'x:5': 'abc' }))

      expect(useLyricOffsetStore().offsets).toEqual({ 'netease:1': 200, 'qq:3': 150, 'x:4': 30_000 })
    })

    it('falls back to an empty table when the stored value is corrupt', () => {
      storage.setItem(OFFSETS_KEY, '{oops')
      expect(useLyricOffsetStore().offsets).toEqual({})
    })

    it('reads the database snapshot instead of localStorage when available', () => {
      storage.setItem(OFFSETS_KEY, JSON.stringify({ 'netease:1': 200 }))
      mocks.preloaded = snapshot({ 'netease:9': 300 })

      expect(useLyricOffsetStore().offsets).toEqual({ 'netease:9': 300 })
    })
  })

  describe('effective offset', () => {
    it('adds the per-track delta to the default of the source', () => {
      storage.setItem(OFFSETS_KEY, JSON.stringify({ 'netease:1': 200 }))
      const offsets = useLyricOffsetStore()

      expect(offsets.effectiveOffsetMs(track('netease:1'))).toBe(1200)
      expect(offsets.effectiveOffsetMs(track('netease:2'))).toBe(1000)
      expect(offsets.effectiveOffsetMs(track('qq:2'))).toBe(500)
    })

    it('follows the lyric source being shown for tracks that were never adjusted', () => {
      const offsets = useLyricOffsetStore()
      const ttml = track('netease:ttml')
      showLyricsFrom(ttml, 'AMLL_TTML')

      expect(offsets.offsetSourceFor(ttml)).toBe('amll_ttml')
      expect(offsets.effectiveOffsetMs(ttml)).toBe(200)
      expect(offsets.offsetSourceIsGuessed(ttml)).toBe(false)
    })

    it('flags the NetEase default as a guess for platforms without known lyric sources', () => {
      const offsets = useLyricOffsetStore()

      expect(offsets.offsetSourceIsGuessed(track('youtube:abc'))).toBe(true)
      expect(offsets.offsetSourceIsGuessed(track('netease:1'))).toBe(false)
      expect(offsets.effectiveOffsetMs(track('youtube:abc'))).toBe(1000)
    })

    it('uses the delta Android wrote into the sync payload unless a local value exists', () => {
      const offsets = useLyricOffsetStore()
      const synced = track('netease:7', { syncPayload: { user_lyric_offset_ms: '400' } })

      expect(offsets.getUserOffsetMs(synced)).toBe(400)
      expect(offsets.effectiveOffsetMs(synced)).toBe(1400)

      offsets.setUserOffsetMs(synced, -100)
      expect(offsets.getUserOffsetMs(synced)).toBe(-100)
    })

    it('keeps an adjusted track on the source it was calibrated against, including legacy records', () => {
      storage.setItem(OFFSETS_KEY, JSON.stringify({ 'netease:9': 100 }))
      storage.setItem(LEGACY_SOURCES_KEY, JSON.stringify({ 'netease:9': 'qq', 'netease:8': 'bogus' }))
      const offsets = useLyricOffsetStore()
      const target = track('netease:9')
      showLyricsFrom(target, 'kugou')

      expect(offsets.offsetSourceFor(target)).toBe('qq')
      expect(offsets.effectiveOffsetMs(target)).toBe(600)
    })
  })

  describe('editing', () => {
    it('stores the difference from the default and forgets it when it matches the default again', () => {
      const offsets = useLyricOffsetStore()
      const target = track('netease:1')

      offsets.setEffectiveOffsetMs(target, 1300.4)
      expect(offsets.getUserOffsetMs(target)).toBe(300)
      expect(storedOffsets()).toEqual({ 'netease:1': 300 })
      expect(JSON.parse(storage.getItem(META_KEY) ?? '{}')['netease:1']).toMatchObject({ source: 'netease' })

      offsets.setEffectiveOffsetMs(target, 1000)
      expect(offsets.offsets).toEqual({})
      expect(storedOffsets()).toEqual({})
    })

    it('clamps extreme values and ignores tracks without an identity', () => {
      const offsets = useLyricOffsetStore()

      offsets.setUserOffsetMs(track('netease:1'), 1e9)
      offsets.setUserOffsetMs(track('', { playlistKey: '  ' }), 100)

      expect(offsets.offsets).toEqual({ 'netease:1': 30_000 })
    })

    it('writes the delta into the playing track payload for sync', () => {
      const current = track('netease:1', { syncPayload: { sync_metadata_version: 0, user_lyric_offset_ms: 9 } })
      player.currentTrack = current
      const offsets = useLyricOffsetStore()

      offsets.setUserOffsetMs(current, 250)

      expect(player.currentTrack?.syncPayload).toEqual({ userLyricOffsetMs: 250, syncMetadataVersion: 1 })
      expect(mocks.payloadWrites).toEqual([{ userLyricOffsetMs: 250, syncMetadataVersion: 1 }])
    })

    it('does not touch the playing track when another track is edited', () => {
      player.currentTrack = track('netease:1')
      const offsets = useLyricOffsetStore()

      offsets.setUserOffsetMs(track('netease:2'), 250)

      expect(player.currentTrack?.syncPayload).toBeUndefined()
      expect(mocks.payloadWrites).toEqual([])
    })

    it('ignores a stale payload delta after the local value was reset on another track', () => {
      const offsets = useLyricOffsetStore()
      const other = track('netease:3', { syncPayload: { userLyricOffsetMs: 700 } })

      offsets.setUserOffsetMs(other, 0)

      expect(offsets.getUserOffsetMs(other)).toBe(0)
      // 同步换上新的载荷值后再采信
      expect(offsets.getUserOffsetMs({ ...other, syncPayload: { userLyricOffsetMs: 800 } })).toBe(800)
    })

    it('saves through the database queue when the backend is available', () => {
      mocks.preloaded = snapshot({})
      const offsets = useLyricOffsetStore()

      offsets.setUserOffsetMs(track('netease:1'), 123.6)
      offsets.clearLocalUserOffset(track('netease:1'))

      expect(mocks.persisted).toEqual([
        { command: 'set_lyric_offset', args: { trackKey: 'netease:1', offsetMs: 124 } },
        { command: 'set_lyric_offset', args: { trackKey: 'netease:1', offsetMs: 0 } },
      ])
      expect(storage.getItem(OFFSETS_KEY)).toBeNull()
    })

    it('replaces the table from sync without writing it back', () => {
      const offsets = useLyricOffsetStore()

      offsets.replaceFromSync({ 'netease:1': 50, 'netease:2': 0 })

      expect(offsets.offsets).toEqual({ 'netease:1': 50 })
      expect(storage.getItem(OFFSETS_KEY)).toBeNull()
    })
  })

  describe('rebasing when a default changes', () => {
    it('keeps the absolute timing of adjusted tracks from the changed source only', async () => {
      storage.setItem(OFFSETS_KEY, JSON.stringify({ 'netease:1': 200, 'qq:2': 100 }))
      const offsets = useLyricOffsetStore()
      await hydrate()

      settings.cloudMusicOffset = 1500
      await flushPromises()

      expect(offsets.offsets).toEqual({ 'netease:1': -300, 'qq:2': 100 })
      expect(offsets.effectiveOffsetMs(track('netease:1'))).toBe(1200)
      expect(storedOffsets()).toEqual({ 'netease:1': -300, 'qq:2': 100 })
    })

    it('does not rebase while settings are being loaded from disk', async () => {
      storage.setItem(OFFSETS_KEY, JSON.stringify({ 'netease:1': 200 }))
      const offsets = useLyricOffsetStore()

      settings.cloudMusicOffset = 1500
      settings.isHydrated = true
      await flushPromises()

      expect(offsets.offsets).toEqual({ 'netease:1': 200 })

      // 读入完成后的修改以读入值为基线
      settings.cloudMusicOffset = 1600
      await flushPromises()
      expect(offsets.offsets).toEqual({ 'netease:1': 100 })
    })

    it('replaces the whole table in the database after a rebase', async () => {
      mocks.preloaded = snapshot({ 'netease:1': 200.4 })
      const offsets = useLyricOffsetStore()
      await hydrate()

      settings.cloudMusicOffset = 900
      await flushPromises()

      expect(offsets.offsets).toEqual({ 'netease:1': 300 })
      expect(mocks.persisted).toEqual([{ command: 'replace_lyric_offsets', args: { offsets: { 'netease:1': 300 } } }])
    })

    it('remembers the payload value when a rebase brings a delta to zero', async () => {
      const synced = track('netease:5', { syncPayload: { userLyricOffsetMs: 500 } })
      const offsets = useLyricOffsetStore()
      offsets.setUserOffsetMs(synced, 500)
      await hydrate()

      settings.cloudMusicOffset = 1500
      await flushPromises()

      expect(offsets.offsets).toEqual({})
      expect(offsets.getUserOffsetMs(synced)).toBe(0)
      expect(offsets.effectiveOffsetMs(synced)).toBe(1500)
    })
  })

  describe('shouldResetOffsetForMatch', () => {
    it('resets only adjusted tracks whose calibration belongs to another source', () => {
      const offsets = useLyricOffsetStore()
      const adjusted = track('netease:1')
      offsets.setUserOffsetMs(adjusted, 200, 'netease')

      expect(offsets.shouldResetOffsetForMatch(track('netease:2'), 'qq')).toBe(false)
      expect(offsets.shouldResetOffsetForMatch(adjusted, 'netease')).toBe(false)
      expect(offsets.shouldResetOffsetForMatch(adjusted, 'qq')).toBe(true)
    })

    it('resets when the displayed lyrics already came from another source', () => {
      const offsets = useLyricOffsetStore()
      const adjusted = track('netease:1', { syncPayload: { matchedLyricSource: 'kugou' } })
      offsets.setUserOffsetMs(adjusted, 200, null)

      expect(offsets.shouldResetOffsetForMatch(adjusted, 'netease')).toBe(true)
      expect(offsets.shouldResetOffsetForMatch(adjusted, 'kugou')).toBe(false)
    })
  })
})
