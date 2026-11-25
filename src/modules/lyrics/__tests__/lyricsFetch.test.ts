import { beforeEach, describe, expect, it, vi } from 'vitest'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

const { fetchAutomaticLyrics } = await import('@/modules/lyrics/lyricsFetch')

const lineLevel = { source: 'lrclib', lines: [{ start_ms: 0, duration_ms: 3000, text: '歌词' }] }
const wordTimed = {
  source: 'amll_ttml',
  lines: [{ start_ms: 0, duration_ms: 3000, text: '歌词', words: [{ start_ms: 0, duration_ms: 1500, text: '歌' }] }],
}
const empty = { source: null, lines: [] }

const biliTrack = { id: 'bilibili:BV1', title: '【洛天依】《歌名》原创PV', artist: 'UP', durationMs: 200_000 }

const LYRIC_COMMANDS = new Set(['fetch_lyrics', 'fetch_word_timed_lyrics'])

// 其它模块（如日志转发）也会调用 invoke，这里只模拟并断言两条取词命令
function respond(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    if (!LYRIC_COMMANDS.has(command)) return undefined
    const handler = handlers[command]
    if (!handler) throw new Error(`unexpected command ${command}`)
    return handler(args)
  })
}

function lyricCalls(): Array<[string, Record<string, unknown>]> {
  return invoke.mock.calls.filter(([command]) => LYRIC_COMMANDS.has(command)) as Array<[string, Record<string, unknown>]>
}

describe('fetchAutomaticLyrics', () => {
  beforeEach(() => invoke.mockReset())

  it('uses word-timed lyrics first for Bilibili playback when preferred', async () => {
    respond({ fetch_word_timed_lyrics: () => wordTimed, fetch_lyrics: () => lineLevel })

    const result = await fetchAutomaticLyrics(biliTrack, 'bilibili', true)

    expect(result.source).toBe('amll_ttml')
    expect(lyricCalls()).toEqual([['fetch_word_timed_lyrics', { title: '歌名', artist: 'UP', durationMs: 200_000 }]])
  })

  it('falls back to the lyric waterfall when word-timed lookup has no word timing or fails', async () => {
    respond({ fetch_word_timed_lyrics: () => lineLevel, fetch_lyrics: () => lineLevel })
    expect((await fetchAutomaticLyrics(biliTrack, 'bilibili', true)).source).toBe('lrclib')

    invoke.mockReset()
    respond({ fetch_word_timed_lyrics: () => { throw new Error('offline') }, fetch_lyrics: () => lineLevel })
    expect((await fetchAutomaticLyrics(biliTrack, 'bilibili', true)).source).toBe('lrclib')
  })

  it('skips the word-timed lookup when not preferred or not Bilibili', async () => {
    respond({ fetch_lyrics: () => lineLevel })

    await fetchAutomaticLyrics(biliTrack, 'bilibili', false)
    await fetchAutomaticLyrics(biliTrack, 'youtube', true)

    expect(lyricCalls().map(([command]) => command)).toEqual(['fetch_lyrics', 'fetch_lyrics'])
  })

  it('tries the quoted song title before the full video title', async () => {
    const titles: unknown[] = []
    respond({ fetch_lyrics: (args) => { titles.push(args.title); return args.title === biliTrack.title ? lineLevel : empty } })

    const result = await fetchAutomaticLyrics(biliTrack, 'bilibili', false)

    expect(titles).toEqual(['歌名', biliTrack.title])
    expect(result.source).toBe('lrclib')
  })

  it('keeps the platform title for tracks fetched by NetEase id', async () => {
    const titles: unknown[] = []
    respond({ fetch_lyrics: (args) => { titles.push(args.title); return empty } })

    const result = await fetchAutomaticLyrics({ id: 'netease:1', title: '《歌名》', artist: 'A' }, 'netease', false)

    expect(titles).toEqual(['《歌名》'])
    expect(result).toEqual({ source: null, lines: [] })
  })
})
