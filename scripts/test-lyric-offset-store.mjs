import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

// 逐曲歌词偏移 store：默认跟着当前歌词来源走、按绝对值编辑、改默认时只 rebase 用这个来源的歌
let sequence = 0
async function loadModule(relativePath, dependencies) {
  const source = await readFile(new URL(relativePath, import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `缺少依赖 ${name}`)
    const clause = node.importClause
    const replacement = clause?.namedBindings
      ? `const { ${clause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text).join(', ')} } = deps[${JSON.stringify(name)}]`
      : ''
    compiled = compiled.slice(0, node.getStart(parsed)) + replacement + compiled.slice(node.end)
  }
  const key = `__lyricOffsetStoreTest${++sequence}`
  globalThis[key] = dependencies
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`)
  } finally { delete globalThis[key] }
}

const storage = new Map()
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
  removeItem: key => storage.delete(key),
}

const offsetModule = await loadModule('../src/modules/lyrics/lyricOffset.ts', {})
const sourceModule = await loadModule('../src/modules/lyrics/lyricSource.ts', { vue })

async function runtime({ hydrated = true, savedOffsets = null, savedMeta = null } = {}) {
  storage.clear()
  if (savedOffsets) storage.set('neri:lyric-offsets', JSON.stringify(savedOffsets))
  if (savedMeta) storage.set('neri:lyric-offset-meta', JSON.stringify(savedMeta))
  const settings = vue.reactive({
    isHydrated: hydrated,
    cloudMusicOffset: 1000, qqMusicOffset: 500, kugouOffset: 0, lrclibOffset: 0, amllTtmlOffset: 300,
  })
  const player = {
    currentTrack: null,
    patched: [],
    patchCurrentTrackSyncPayload(payload) { this.patched.push(payload); this.currentTrack = { ...this.currentTrack, syncPayload: payload } },
  }
  const module = await loadModule('../src/stores/lyricOffset.ts', {
    pinia, vue,
    '@/stores/settings': { useSettingsStore: () => settings },
    '@/modules/lyrics/lyricOffset': offsetModule,
    '@/modules/lyrics/lyricSource': sourceModule,
    '@/modules/lyrics/syncTrackPayload': { persistTrackSyncPayload: async () => {} },
    '@/stores/player': { usePlayerStore: () => player },
    '@/modules/persistence/userData': {
      LEGACY_LYRIC_OFFSETS_KEY: 'neri:lyric-offsets',
      persistUserData: async () => {},
      preloadedUserData: () => null,
    },
    '@/utils/logger': { createLogger: () => ({ error() {}, warn() {} }) },
  })
  pinia.setActivePinia(pinia.createPinia())
  return { store: module.useLyricOffsetStore(), settings, player }
}

const track = (id, extra = {}) => ({ id, title: id, artist: 'Artist', durationMs: 200000, source: id.split(':')[0], ...extra })
let cases = 0
async function regression(name, run) {
  await run()
  cases++
  console.log(`ok - ${name}`)
}

await regression('the default follows the source of the lyrics on screen', async () => {
  const { store } = await runtime()
  const song = track('netease:1')
  assert.equal(store.effectiveOffsetMs(song), 1000, '来源未知时按播放来源（网易云）')
  sourceModule.rememberLyricSource(song, 'amll_ttml')
  assert.equal(store.offsetSourceFor(song), 'amll_ttml')
  assert.equal(store.effectiveOffsetMs(song), 300, '显示的是 AMLL TTML 逐字歌词时用 TTML 的默认')
  sourceModule.rememberLyricSource(song, 'LOCAL_EDIT')
  assert.equal(store.effectiveOffsetMs(song), 1000, '手动编辑的歌词与 Android 一样按网易云默认')
  const youtube = track('youtube:abc')
  assert.equal(store.effectiveOffsetMs(youtube), 1000, 'YouTube 曲目来源未知时也回落网易云默认')
  sourceModule.rememberLyricSource(youtube, 'lrclib')
  assert.equal(store.offsetSourceFor(youtube), 'lrclib')
})

await regression('a NetEase default borrowed for unknown lyrics is reported as a guess', async () => {
  const { store } = await runtime()
  const bili = track('bilibili:1')
  assert.equal(store.offsetSourceFor(bili), 'netease')
  assert.equal(store.offsetSourceIsGuessed(bili), true, 'B 站歌词没记来源，网易云默认只是兜底')
  sourceModule.rememberLyricSource(bili, 'KUGOU')
  assert.equal(store.offsetSourceIsGuessed(bili), false)
  const netease = track('netease:9')
  assert.equal(store.offsetSourceIsGuessed(netease), false, '网易云曲目来源未知时就是网易云的歌词')
})

await regression('editing the absolute offset stores the Android delta', async () => {
  const { store, player } = await runtime()
  const song = track('netease:2')
  sourceModule.rememberLyricSource(song, 'netease')
  player.currentTrack = song
  store.setEffectiveOffsetMs(song, 1200)
  assert.equal(store.getUserOffsetMs(song), 200, '存的是相对默认的 delta')
  assert.equal(store.effectiveOffsetMs(song), 1200)
  assert.equal(player.patched.at(-1).userLyricOffsetMs, 200, '同步载荷写的也是 delta')
  store.setEffectiveOffsetMs(song, 1000)
  assert.equal(store.getUserOffsetMs(song), 0, '回到默认即不再单独调整')
  assert.equal(JSON.parse(storage.get('neri:lyric-offsets') || '{}')['netease:2'], undefined)
})

await regression('changing a default rebases only songs adjusted under that source', async () => {
  const { store, settings } = await runtime()
  const ttml = track('netease:3')
  const netease = track('netease:4')
  sourceModule.rememberLyricSource(ttml, 'amll_ttml')
  sourceModule.rememberLyricSource(netease, 'netease')
  store.setEffectiveOffsetMs(ttml, 400)
  store.setEffectiveOffsetMs(netease, 1200)
  settings.amllTtmlOffset = 500
  await vue.nextTick()
  assert.equal(store.effectiveOffsetMs(ttml), 400, '调过的 TTML 歌保持原来的绝对偏移')
  assert.equal(store.getUserOffsetMs(ttml), -100)
  assert.equal(store.getUserOffsetMs(netease), 200, '网易云歌不受 TTML 默认影响')
  const untouched = track('netease:5')
  sourceModule.rememberLyricSource(untouched, 'amll_ttml')
  assert.equal(store.effectiveOffsetMs(untouched), 500, '没调过的歌跟随新默认')
})

await regression('offsets saved before sources were recorded rebase by their track id', async () => {
  const { store, settings } = await runtime({ savedOffsets: { 'qq:9': 50, 'youtube:x': 80 } })
  settings.qqMusicOffset = 300
  await vue.nextTick()
  assert.equal(store.getUserOffsetMs(track('qq:9')), 250, 'qq: 前缀的旧偏移按 QQ 默认 rebase')
  settings.cloudMusicOffset = 800
  await vue.nextTick()
  assert.equal(store.getUserOffsetMs(track('youtube:x')), 280, 'YouTube 曲目按网易云默认 rebase（1000+80 = 800+280）')
})

await regression('songs recorded under the old zero default follow the Android default', async () => {
  const { store, settings } = await runtime({
    savedOffsets: { 'bilibili:7': 300 },
    savedMeta: { 'bilibili:7': { source: 'none' } },
  })
  const song = track('bilibili:7', { syncPayload: { userLyricOffsetMs: 300 } })
  assert.equal(store.effectiveOffsetMs(song), 1300, '同步出去的 delta 在 Android 上叠网易云默认，两端总偏移一致')
  settings.cloudMusicOffset = 800
  await vue.nextTick()
  assert.equal(store.effectiveOffsetMs(song), 1300, '改网易云默认时同样 rebase，绝对时序不变')
})

await regression('loading settings at startup only sets the baseline', async () => {
  const { store, settings } = await runtime({ hydrated: false })
  const song = track('netease:6')
  sourceModule.rememberLyricSource(song, 'netease')
  store.setUserOffsetMs(song, 200)
  settings.cloudMusicOffset = 700
  settings.isHydrated = true
  await vue.nextTick()
  assert.equal(store.getUserOffsetMs(song), 200, '读入磁盘设置不是用户改默认，不 rebase')
  settings.cloudMusicOffset = 900
  await vue.nextTick()
  assert.equal(store.getUserOffsetMs(song), 0, '之后用户改默认才 rebase（700+200 = 900+0）')
})

await regression('an adjusted song keeps its absolute offset when the lyric source changes', async () => {
  const { store, player } = await runtime()
  const song = track('netease:7')
  sourceModule.rememberLyricSource(song, 'netease')
  player.currentTrack = song
  store.setEffectiveOffsetMs(song, 1200)
  sourceModule.rememberLyricSource(song, 'amll_ttml')
  assert.equal(store.offsetSourceFor(song), 'netease', '调过的歌固定按调整时的来源')
  assert.equal(store.effectiveOffsetMs(song), 1200, '逐字歌词升级后偏移不能跳到 300+200')
  store.setEffectiveOffsetMs(song, 400)
  assert.equal(store.offsetSourceFor(song), 'amll_ttml', '再次调整改按正在显示的来源')
  assert.equal(store.getUserOffsetMs(song), 100)
  assert.equal(store.effectiveOffsetMs(song), 400)
  store.setEffectiveOffsetMs(song, 300)
  sourceModule.rememberLyricSource(song, 'netease')
  assert.equal(store.effectiveOffsetMs(song), 1000, '恢复默认后重新跟随歌词来源')
})

await regression('a rebase to zero ignores the stale delta left in the sync payload', async () => {
  const { store, settings, player } = await runtime()
  const song = track('netease:8')
  sourceModule.rememberLyricSource(song, 'netease')
  player.currentTrack = song
  store.setEffectiveOffsetMs(song, 1200)
  const withPayload = player.currentTrack
  assert.equal(withPayload.syncPayload.userLyricOffsetMs, 200)
  player.currentTrack = track('netease:other')
  settings.cloudMusicOffset = 1200
  await vue.nextTick()
  assert.equal(store.getUserOffsetMs(withPayload), 0, 'rebase 只改本地表，载荷里的旧 200 不能复活')
  assert.equal(store.effectiveOffsetMs(withPayload), 1200)
  const resynced = { ...withPayload, syncPayload: { ...withPayload.syncPayload, userLyricOffsetMs: 300 } }
  assert.equal(store.getUserOffsetMs(resynced), 300, '同步换来别的值时照常采信')
})

await regression('resetting a song that is not playing ignores its synced delta', async () => {
  const { store, player } = await runtime()
  const song = track('qq:10', { syncPayload: { userLyricOffsetMs: 150 } })
  player.currentTrack = track('qq:playing')
  assert.equal(store.getUserOffsetMs(song), 150, 'Android 同步来的 delta')
  store.setUserOffsetMs(song, 0)
  assert.equal(store.getUserOffsetMs(song), 0, '不在播放的歌载荷没改写，也要按已重置算')
  assert.equal(player.patched.length, 0, '不能改写正在播放那首的载荷')
})

console.log(`Lyric offset store regressions: ${cases} passed`)
