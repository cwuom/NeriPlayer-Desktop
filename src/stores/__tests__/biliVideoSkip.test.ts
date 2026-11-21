import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { UserDataSnapshot } from '@/modules/persistence/userData'
import type { BiliVideoSkipRule } from '@/modules/playback/biliVideoSkip'
import { useBiliVideoSkipStore } from '@/stores/biliVideoSkip'
import { MemoryStorage } from '@/modules/__tests__/memoryStorage'

const mocks = vi.hoisted(() => ({
  preloaded: null as UserDataSnapshot | null,
  persist: null as ((command: string, args: Record<string, unknown>) => Promise<unknown>) | null,
  persisted: [] as Array<{ command: string; args: Record<string, unknown> }>,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (command: string) => { throw new Error(`unexpected invoke: ${command}`) },
}))
vi.mock('@/modules/persistence/userData', () => ({
  preloadedUserData: () => mocks.preloaded,
  persistUserData: (command: string, args: Record<string, unknown>) => {
    mocks.persisted.push({ command, args })
    return mocks.persist!(command, args)
  },
}))

const STORAGE_KEY = 'neri:bili-video-skip-rules'
const target = { bvid: 'BV1xx411c7mD', cid: 100 }

function rule(overrides: Partial<BiliVideoSkipRule> = {}): BiliVideoSkipRule {
  return { ...target, intervals: [{ startMs: 0, endMs: 5000 }], modifiedAt: 1000, isDeleted: false, ...overrides }
}

function snapshot(rules: unknown[]): UserDataSnapshot {
  return {
    playbackState: null,
    history: { entries: [], deletions: [] },
    lyricOffsets: {},
    biliVideoSkipRules: rules as BiliVideoSkipRule[],
    migrated: { playbackState: true, history: true, lyricOffsets: true },
  }
}

function storedRules(storage: MemoryStorage): BiliVideoSkipRule[] {
  return JSON.parse(storage.getItem(STORAGE_KEY) ?? 'null') as BiliVideoSkipRule[]
}

describe('biliVideoSkip store', () => {
  let storage: MemoryStorage

  beforeEach(() => {
    storage = new MemoryStorage()
    vi.stubGlobal('localStorage', storage)
    mocks.preloaded = null
    mocks.persisted = []
    mocks.persist = async () => { throw new Error('persist not expected') }
    vi.useFakeTimers()
    vi.setSystemTime(50_000)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  describe('loading', () => {
    it('normalizes rules stored by the localStorage fallback', () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([
        { bvid: ' BV1 ', cid: '7', intervals: [{ startMs: 3000, endMs: 4000 }, { startMs: 1000, endMs: 3500 }], modifiedAt: 5 },
        { bvid: '', cid: 1, intervals: [{ startMs: 0, endMs: 1 }], modifiedAt: 1 },
        { bvid: 'BV2', cid: 2, intervals: [], modifiedAt: 1 },
        'junk',
      ]))

      const store = useBiliVideoSkipStore()

      expect(store.rules).toEqual([
        { bvid: 'BV1', cid: 7, intervals: [{ startMs: 1000, endMs: 4000 }], modifiedAt: 5, isDeleted: false },
      ])
    })

    it('starts empty when the stored rules are corrupt', () => {
      storage.setItem(STORAGE_KEY, '[{')
      expect(useBiliVideoSkipStore().rules).toEqual([])
    })

    it('prefers the database snapshot over localStorage when the backend is available', () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([rule({ cid: 1 })]))
      mocks.preloaded = snapshot([rule({ cid: 2 })])

      expect(useBiliVideoSkipStore().rules.map(item => item.cid)).toEqual([2])
    })
  })

  describe('ruleFor', () => {
    it('hides tombstones and rejects targets without a valid BVID and CID', () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([rule(), rule({ cid: 200, intervals: [], isDeleted: true })]))
      const store = useBiliVideoSkipStore()

      expect(store.ruleFor({ bvid: ` ${target.bvid} `, cid: 100 })?.intervals).toEqual([{ startMs: 0, endMs: 5000 }])
      expect(store.ruleFor({ bvid: target.bvid, cid: 200 })).toBeNull()
      expect(store.ruleFor({ bvid: '', cid: 100 })).toBeNull()
      expect(store.ruleFor({ bvid: target.bvid, cid: 0 })).toBeNull()
    })
  })

  describe('setIntervals without a database', () => {
    it('merges overlapping intervals, clamps them to the duration and saves locally', async () => {
      const store = useBiliVideoSkipStore()

      const saved = await store.setIntervals(target, [
        { startMs: 9000, endMs: 20_000 },
        { startMs: -50, endMs: 2000 },
        { startMs: 1500, endMs: 3000 },
      ], 10_000)

      expect(saved).toEqual({ ...target, intervals: [{ startMs: 0, endMs: 3000 }, { startMs: 9000, endMs: 10_000 }], modifiedAt: 50_000, isDeleted: false })
      expect(store.rules).toEqual([saved])
      expect(storedRules(storage)).toEqual([saved])
      expect(mocks.persisted).toEqual([])
    })

    it('always moves modifiedAt forward even if the clock went backwards', async () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([rule({ modifiedAt: 90_000 })]))
      const store = useBiliVideoSkipStore()

      const saved = await store.setIntervals(target, [{ startMs: 0, endMs: 1000 }])

      expect(saved?.modifiedAt).toBe(90_001)
      expect(store.ruleFor(target)?.intervals).toEqual([{ startMs: 0, endMs: 1000 }])
    })

    it('returns the existing rule untouched when nothing changed', async () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([rule()]))
      const store = useBiliVideoSkipStore()
      const writes = vi.spyOn(storage, 'setItem')

      const result = await store.setIntervals(target, [{ startMs: 0, endMs: 2000 }, { startMs: 1000, endMs: 5000 }])

      expect(result).toEqual(rule())
      expect(writes).not.toHaveBeenCalled()
    })

    it('stores a tombstone when the last interval is removed, and skips work for unknown targets', async () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([rule()]))
      const store = useBiliVideoSkipStore()

      expect(await store.setIntervals({ bvid: 'BVother', cid: 1 }, [])).toBeNull()
      const removed = await store.setIntervals(target, [{ startMs: 500, endMs: 500 }])

      expect(removed).toMatchObject({ ...target, intervals: [], isDeleted: true, modifiedAt: 50_000 })
      expect(store.ruleFor(target)).toBeNull()
      expect(storedRules(storage)).toEqual([removed])
    })

    it('rejects an invalid target', async () => {
      const store = useBiliVideoSkipStore()
      await expect(store.setIntervals({ bvid: 'BV1', cid: Number.NaN }, [{ startMs: 0, endMs: 1 }])).rejects.toThrow(/BVID and CID/)
    })
  })

  describe('setIntervals with the database', () => {
    beforeEach(() => {
      mocks.preloaded = snapshot([])
    })

    it('saves through the user data queue and keeps the backend result', async () => {
      const backendRule = rule({ modifiedAt: 77 })
      mocks.persist = async () => backendRule
      const store = useBiliVideoSkipStore()

      const saved = await store.setIntervals(target, [{ startMs: 0, endMs: 5000 }], 12_345.6)

      expect(mocks.persisted).toEqual([{
        command: 'set_bili_video_skip_rule',
        args: { ...target, intervals: [{ startMs: 0, endMs: 5000 }], durationMs: 12_346 },
      }])
      expect(saved).toEqual(backendRule)
      expect(store.rules).toEqual([backendRule])
      expect(storage.getItem(STORAGE_KEY)).toBeNull()
    })

    it('sends a zero duration when the duration is not a finite number', async () => {
      mocks.persist = async () => rule()
      const store = useBiliVideoSkipStore()

      await store.setIntervals(target, [{ startMs: 0, endMs: 5000 }], Number.POSITIVE_INFINITY)

      expect(mocks.persisted[0].args.durationMs).toBe(0)
    })

    it('leaves the rules unchanged when saving fails so the editor can retry', async () => {
      mocks.preloaded = snapshot([rule()])
      mocks.persist = async () => { throw new Error('disk full') }
      const store = useBiliVideoSkipStore()

      await expect(store.setIntervals(target, [{ startMs: 0, endMs: 9000 }])).rejects.toThrow('disk full')

      expect(store.rules).toEqual([rule()])
    })

    it('returns null without touching memory when the backend reports nothing saved', async () => {
      mocks.persist = async () => null
      const store = useBiliVideoSkipStore()

      expect(await store.setIntervals(target, [{ startMs: 0, endMs: 1000 }])).toBeNull()
      expect(store.rules).toEqual([])
    })
  })

  describe('replaceFromSync', () => {
    it('keeps a newer local edit over an older synced rule and accepts newer remote rules', () => {
      storage.setItem(STORAGE_KEY, JSON.stringify([rule({ modifiedAt: 2000, intervals: [{ startMs: 0, endMs: 1000 }] })]))
      const store = useBiliVideoSkipStore()

      store.replaceFromSync([
        rule({ modifiedAt: 1000, intervals: [{ startMs: 0, endMs: 9999 }] }),
        rule({ cid: 300, modifiedAt: 10 }),
      ])

      expect(store.ruleFor(target)?.intervals).toEqual([{ startMs: 0, endMs: 1000 }])
      expect(store.ruleFor({ bvid: target.bvid, cid: 300 })).not.toBeNull()
      expect(storedRules(storage).map(item => item.cid)).toEqual([100, 300])

      store.replaceFromSync([rule({ modifiedAt: 3000, intervals: [], isDeleted: true })])
      expect(store.ruleFor(target)).toBeNull()
    })

    it('does not write localStorage when the database owns the rules', () => {
      mocks.preloaded = snapshot([])
      const store = useBiliVideoSkipStore()

      store.replaceFromSync([rule()])

      expect(store.rules).toEqual([rule()])
      expect(storage.getItem(STORAGE_KEY)).toBeNull()
    })
  })
})
