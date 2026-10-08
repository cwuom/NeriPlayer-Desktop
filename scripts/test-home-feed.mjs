import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

const parserPath = new URL('../src/modules/library/neteaseHome.ts', import.meta.url)
const storePath = new URL('../src/stores/homeFeed.ts', import.meta.url)
assert.ok(existsSync(parserPath), '网易云首页解析尚未实现')
assert.ok(existsSync(storePath), '独立首页分区状态尚未实现')
let sequence = 0
async function load(path, dependencies = {}) {
  const source = await readFile(path, 'utf8')
  const key = `__homeFeedTest${++sequence}`
  globalThis[key] = dependencies
  let compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `missing dependency ${name}`)
    const names = node.importClause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text)
    compiled = compiled.slice(0, node.getStart(parsed)) + `const { ${names.join(', ')} } = deps[${JSON.stringify(name)}]` + compiled.slice(node.end)
  }
  try { return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`) }
  finally { delete globalThis[key] }
}
const parser = await load(parserPath)
const { NETEASE_HOME_SECTIONS, normalizeHomeSection } = parser
const sources = ['personal_radar', 'radar_playlists', 'daily_recommend', 'private_fm', 'top_soaring', 'personalized_new_songs', 'top_hot', 'top_new', 'personalized', 'daily_resource', 'high_quality', 'hot_playlists', 'acg_playlists']
const privateSources = ['daily_recommend', 'private_fm', 'daily_resource']
const song = (id = 1, overrides = {}) => ({ id, name: `歌曲 ${id}`, ar: [{ id: 3, name: '甲' }, { id: 4, name: '乙' }], al: { id: 5, name: '专辑', picUrl: 'http://cover.test/song.jpg' }, dt: 123000, ...overrides })
const playlist = (id = 1, overrides = {}) => ({ id, name: `歌单 ${id}`, picUrl: 'http://cover.test/playlist.jpg', trackCount: 12, ...overrides })
const rawFor = source => ({ code: 200, result: source === 'radar_playlists' || ['personalized', 'daily_resource', 'high_quality', 'hot_playlists', 'acg_playlists'].includes(source) ? [playlist(1, { name: source })] : [song(1, { name: source })] })
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
async function flush() { for (let i = 0; i < 16; i++) await Promise.resolve() }
const oldStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
async function runtime(options = {}) {
  const storage = options.storage || new Map(), calls = []
  globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) }
  let active = 0, maxActive = 0
  const module = await load(storePath, {
    pinia, vue, '@/modules/library/neteaseHome': parser,
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      calls.push({ command, args }); active++; maxActive = Math.max(maxActive, active)
      try { return options.invoke ? await options.invoke(command, args) : rawFor(args.source) }
      finally { active-- }
    } },
  })
  pinia.setActivePinia(pinia.createPinia())
  return { store: module.useHomeFeedStore(), storage, calls, get maxActive() { return maxActive } }
}

try {
  assert.deepEqual(NETEASE_HOME_SECTIONS.map(s => s.key), sources)
  assert.deepEqual(NETEASE_HOME_SECTIONS.filter(s => s.requiresLogin).map(s => s.key), privateSources)
  for (const shape of [
    { data: { dailySongs: [song()] } }, { data: { songs: [song()] } }, { data: [song()] },
    { result: [{ song: song() }] }, { songs: [song()] }, { playlist: { tracks: [song()] } },
  ]) {
    const parsed = normalizeHomeSection({ code: 200, ...shape }, 'songs')
    assert.deepEqual(parsed.songs[0], { id: 'netease:1', title: '歌曲 1', artist: '甲 / 乙', album: '专辑', durationMs: 123000, coverUrl: 'https://cover.test/song.jpg', audioUrl: '', source: 'netease' })
    assert.deepEqual(parsed.playlists, [])
  }
  for (const shape of [
    { result: [playlist()] }, { recommend: [playlist()] }, { playlists: [playlist()] },
    { data: { playlists: [playlist()] } }, { data: { list: [playlist()] } },
  ]) assert.equal(normalizeHomeSection({ code: 200, ...shape }, 'playlists').playlists[0].coverUrl, 'https://cover.test/playlist.jpg')
  assert.equal(normalizeHomeSection(JSON.stringify({ code: 200, songs: [song()] }), 'songs').songs.length, 1)
  assert.equal(normalizeHomeSection({ code: 200, songs: [song(2, { ar: [], artists: [{ name: '旧艺术家' }], al: undefined, album: { name: '旧专辑', picUrl_str: 'http://cover.test/legacy.jpg' }, dt: undefined, duration: 5000 })] }, 'songs').songs[0].artist, '旧艺术家')
  const legacy = normalizeHomeSection({ code: 200, result: [playlist(2, { picUrl: '', coverImgUrl: 'http://cover.test/fallback.jpg', trackCount: undefined, songCount: 8 })] }, 'playlists').playlists[0]
  assert.deepEqual(legacy, { id: '2', name: '歌单 2', coverUrl: 'https://cover.test/fallback.jpg', trackCount: 8, playCount: 0 })
  assert.equal(normalizeHomeSection({ code: 200, result: [playlist(3, { playCount: 321 })] }, 'playlists').playlists[0].playCount, 321)
  assert.equal(normalizeHomeSection({ code: 200, result: [playlist(3, { playcount: 123 })] }, 'playlists').playlists[0].playCount, 123)
  assert.equal(normalizeHomeSection({ code: 200, result: [playlist()] }, 'radar').playlists.length, 1)
  assert.throws(() => normalizeHomeSection({ code: 301, message: '请登录' }, 'songs'), /301/)
  assert.throws(() => normalizeHomeSection({ songs: [song()] }, 'songs'), /code/)
  assert.throws(() => normalizeHomeSection('invalid JSON', 'songs'))
  assert.deepEqual(normalizeHomeSection({ code: 200 }, 'songs'), { songs: [], playlists: [] })
  assert.equal(normalizeHomeSection({ code: 200, data: { dailySongs: [] }, songs: [song()] }, 'songs').songs.length, 0)
  const invalid = [null, song(0), song(-1), song('bad'), song(2, { name: ' ' }), song(3), song(3), ...Array.from({ length: 45 }, (_, i) => song(i + 4))]
  const normalized = normalizeHomeSection({ code: 200, songs: invalid }, 'songs').songs
  assert.equal(normalized.length, 30); assert.equal(new Set(normalized.map(s => s.id)).size, 30); assert.equal(normalized[0].id, 'netease:3')
  const normalizedPlaylists = normalizeHomeSection({ code: 200, result: [null, playlist(0), playlist(2, { name: '' }), playlist(3), playlist(3), ...Array.from({ length: 45 }, (_, i) => playlist(i + 4))] }, 'playlists').playlists
  assert.equal(normalizedPlaylists.length, 30); assert.equal(new Set(normalizedPlaylists.map(p => p.id)).size, 30); assert.equal(normalizedPlaylists[0].id, '3')
  const big = normalizeHomeSection({ code: 200, result: [playlist('9007199254740993')] }, 'playlists').playlists[0]
  assert.equal(big.id, '9007199254740993')
  console.log('home feed parser shapes, validation, ordering and deduplication passed')

  {
    const r = await runtime(); await r.store.refresh(false, '')
    assert.deepEqual(r.calls.map(c => c.args.source), sources.filter(s => !privateSources.includes(s)))
    assert.ok(r.calls.every(c => c.command === 'get_netease_home_section'))
    assert.ok(r.maxActive <= 3)
    assert.ok(privateSources.every(s => r.store.sections[s].songs.length === 0 && r.store.sections[s].playlists.length === 0 && !r.store.sections[s].loading))
  }
  {
    const r = await runtime(); await r.store.refresh(true, 'account-a')
    assert.deepEqual(r.calls.map(c => c.args.source), sources)
    const persisted = JSON.parse(r.storage.get('neri:home-feed:v1'))
    for (const key of privateSources) assert.equal(key in persisted.sections, false, '私人分区不得持久化')
    const r2 = await runtime({ storage: r.storage }); await r2.store.refresh(true, 'account-a')
    assert.deepEqual(r2.calls.map(c => c.args.source), privateSources)
    assert.equal(r2.store.sections.top_hot.songs[0].title, 'top_hot')
    const r3 = await runtime({ storage: r.storage }); await r3.store.refresh(false, '')
    assert.equal(r3.calls.length, 10); assert.equal(r3.store.sections.daily_recommend.songs.length, 0)
  }
  {
    const slow = deferred(), r = await runtime({ invoke: async (_command, { source }) => source === 'personal_radar' ? slow.promise : source === 'top_hot' ? { code: 500 } : rawFor(source) })
    const running = r.store.refresh(false, ''); await flush()
    assert.equal(r.store.sections.personal_radar.loading, true)
    assert.equal(r.store.sections.radar_playlists.playlists.length, 1)
    assert.equal(r.store.sections.acg_playlists.playlists.length, 1)
    assert.match(r.store.sections.top_hot.error, /500/)
    assert.ok(r.maxActive <= 3); slow.resolve(rawFor('personal_radar')); await running
  }
  {
    let fail = false
    const r = await runtime({ invoke: async (_command, { source }) => fail && source === 'top_hot' ? Promise.reject(new Error('fixture partial failure')) : rawFor(source) })
    await r.store.refresh(false, ''); fail = true; await r.store.refresh(false, '', true)
    assert.equal(r.store.sections.top_hot.songs[0].title, 'top_hot'); assert.match(r.store.sections.top_hot.error, /partial failure/)
    const before = r.calls.length; fail = false; await r.store.retry('top_hot')
    assert.equal(r.calls.length, before + 1); assert.equal(r.calls.at(-1).args.source, 'top_hot'); assert.equal(r.store.sections.top_hot.error, null)
    await r.store.retry('daily_recommend'); assert.equal(r.calls.length, before + 1)
  }
  {
    let fail = false
    const r = await runtime({ invoke: async (_command, { source }) => fail && source === 'daily_recommend' ? Promise.reject(new Error('fixture private refresh failure')) : rawFor(source) })
    await r.store.refresh(true, 'nickname|avatar|0'); fail = true
    await r.store.refresh(true, 'nickname|avatar|0', true)
    assert.equal(r.store.sections.daily_recommend.songs[0]?.title, 'daily_recommend', '同账号手动刷新失败必须保留私人内容')
    assert.match(r.store.sections.daily_recommend.error, /private refresh failure/)
    const next = r.store.refresh(true, 'nickname|avatar|1')
    assert.equal(r.store.sections.daily_recommend.songs.length, 0, '会话 revision 变化必须立即清除私人内容')
    await next
  }
  {
    let blockRetry = false
    const waiting = deferred(), r = await runtime({ invoke: async (_command, { source }) => blockRetry && source === 'top_hot' ? waiting.promise : rawFor(source) })
    await r.store.refresh(false, ''); blockRetry = true
    const retry = r.store.retry('top_hot'); await flush()
    const refresh = r.store.refresh(false, ''); await flush()
    assert.equal(r.store.sections.top_hot.loading, true, '常规重入刷新不应取消当前分区重试')
    assert.equal(r.calls.length, 11)
    waiting.resolve({ code: 200, result: [song(7, { name: '有效重试结果' })] }); await Promise.all([retry, refresh])
    assert.equal(r.store.sections.top_hot.songs[0].title, '有效重试结果')
  }
  {
    const r = await runtime(); await r.store.refresh(false, '')
    const prior = r.store.sections.top_hot.songs[0], cache = r.storage.get('neri:home-feed:v1')
    r.store.deactivate(); await r.store.retry('top_hot')
    assert.equal(r.calls.length, 10); assert.equal(r.store.sections.top_hot.songs[0], prior); assert.equal(r.storage.get('neri:home-feed:v1'), cache)
    await r.store.refresh(false, ''); assert.equal(r.calls.length, 10)
  }
  {
    let block = false, account = '旧账号'
    const waiting = deferred(), r = await runtime({ invoke: async (_command, { source }) => block && source === 'daily_recommend' ? waiting.promise : {
      ...rawFor(source), result: rawFor(source).result.map(item => ({ ...item, name: account })),
    } })
    await r.store.refresh(true, 'nickname|avatar|0'); block = true
    const retry = r.store.retry('daily_recommend'); await flush()
    const before = r.calls.length
    r.store.deactivate(true, 'nickname|avatar|1')
    assert.equal(r.calls.length, before, '停页同步账号上下文不能发出 IPC')
    assert.ok(Object.values(r.store.sections).every(s => !s.loading && s.songs.length + s.playlists.length === 0), '停页换号时必须立即清除所有旧分区')
    waiting.resolve({ code: 200, result: [song(77, { name: '旧账号迟到内容' })] }); await retry; await flush()
    assert.equal(JSON.stringify(r.store.sections).includes('旧账号'), false)
    block = false; account = '新账号'
    await r.store.refresh(true, 'nickname|avatar|1')
    assert.equal(r.calls.length, before + 13); assert.equal(r.store.sections.daily_recommend.songs[0].title, '新账号')
  }
  {
    const r = await runtime(); await r.store.refresh(false, '')
    const r2 = await runtime({ storage: r.storage }); r2.store.deactivate(false, '')
    await r2.store.refresh(false, '')
    assert.equal(r2.calls.length, 0, '首次停页同步context后，首次refresh仍应恢复相符的公共缓存')
    assert.equal(r2.store.sections.top_hot.songs[0].title, 'top_hot')
  }
  {
    const blockers = [deferred(), deferred(), deferred()], r = await runtime({ invoke: async (_command, { source }) => {
      const index = sources.indexOf(source)
      return index < 3 ? blockers[index].promise : rawFor(source)
    } })
    const running = r.store.refresh(true, 'account-a'); await flush(); assert.equal(r.calls.length, 3)
    r.store.deactivate(); assert.ok(Object.values(r.store.sections).every(s => !s.loading))
    blockers.forEach((b, i) => b.resolve(rawFor(sources[i]))); await running; await flush()
    assert.equal(r.calls.length, 3); assert.ok(Object.values(r.store.sections).every(s => s.songs.length + s.playlists.length === 0))
  }
  {
    const blockers = [deferred(), deferred(), deferred()], r = await runtime({ invoke: async (_command, { source }) => {
      const index = sources.indexOf(source)
      if (r.calls.length <= 3) return blockers[index].promise
      return { code: 200, result: source === 'radar_playlists' ? [playlist(2, { name: '新账号' })] : [song(2, { name: '新账号' })] }
    } })
    const oldRun = r.store.refresh(true, 'account-a'); await flush()
    const newRun = r.store.refresh(false, ''); await flush(); assert.equal(r.calls.length, 3, '旧 IPC 尚未结束时也必须遵守三并发')
    blockers.forEach((b, i) => b.resolve(rawFor(sources[i]))); await Promise.all([oldRun, newRun])
    assert.ok(r.maxActive <= 3); assert.equal(r.store.sections.daily_recommend.songs.length, 0); assert.equal(r.store.sections.personal_radar.songs[0].title, '新账号')
    assert.equal(r.calls.slice(3).some(c => privateSources.includes(c.args.source)), false)
  }
  {
    const blocker = deferred(), r = await runtime({ invoke: async (_command, { source }) => source === 'daily_recommend' && r.calls.length <= 3 ? blocker.promise : rawFor(source) })
    const oldRun = r.store.refresh(true, 'account-a'); await flush()
    const newRun = r.store.refresh(true, 'account-b'); await flush()
    blocker.resolve({ code: 200, result: [song(88, { name: '旧账号私人内容' })] }); await Promise.all([oldRun, newRun])
    assert.equal(r.store.sections.daily_recommend.songs[0].title, 'daily_recommend')
    assert.equal(JSON.stringify(r.store.sections).includes('旧账号私人内容'), false)
  }
  {
    const r = await runtime(); await r.store.refresh(true, 'shared-nickname-avatar')
    const prior = r.store.sections.daily_recommend.songs[0]
    const originalCalls = r.calls.length
    await r.store.refresh(false, ''); assert.equal(r.store.sections.daily_recommend.songs.length, 0)
    await r.store.refresh(true, 'shared-nickname-avatar'); assert.notEqual(r.store.sections.daily_recommend.songs[0], prior)
    assert.ok(r.calls.length > originalCalls)
  }
  {
    const r = await runtime(); await r.store.refresh(false, '')
    const cache = JSON.parse(r.storage.get('neri:home-feed:v1'))
    for (const state of Object.values(cache.sections)) state.fetchedAt = Date.now() - 31 * 60 * 1000
    r.storage.set('neri:home-feed:v1', JSON.stringify(cache))
    const r2 = await runtime({ storage: r.storage }); await r2.store.refresh(false, ''); assert.equal(r2.calls.length, 10)
    cache.version = 99; r.storage.set('neri:home-feed:v1', JSON.stringify(cache))
    const r3 = await runtime({ storage: r.storage }); await r3.store.refresh(false, ''); assert.equal(r3.calls.length, 10)
    const storage = new Map([['neri:recommend:cache', JSON.stringify({ homeHotSongs: { items: [song(999)] } })]])
    const r4 = await runtime({ storage }); await r4.store.refresh(false, ''); assert.equal(r4.store.sections.top_hot.songs[0].title, 'top_hot')
  }
  console.log('home feed independent sections, three-request scheduling, cache privacy and generation races passed')
} finally {
  if (oldStorage) Object.defineProperty(globalThis, 'localStorage', oldStorage)
  else delete globalThis.localStorage
}
