import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

// 云端歌单：先显示缓存、后台刷新时有状态可显示、并发合并、失败不清缓存、换号丢弃旧结果
let sequence = 0
async function loadStore(dependencies) {
  const source = await readFile(new URL('../src/stores/recommend.ts', import.meta.url), 'utf8')
  let compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const parsed = ts.createSourceFile('test.mjs', compiled, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS)
  for (const node of [...parsed.statements].reverse()) {
    if (!ts.isImportDeclaration(node)) continue
    const name = node.moduleSpecifier.text
    assert.ok(name in dependencies, `缺少依赖 ${name}`)
    const bindings = node.importClause.namedBindings.elements.map(item => item.propertyName ? `${item.propertyName.text}: ${item.name.text}` : item.name.text)
    compiled = compiled.slice(0, node.getStart(parsed)) + `const { ${bindings.join(', ')} } = deps[${JSON.stringify(name)}]` + compiled.slice(node.end)
  }
  const key = `__cloudPlaylistsTest${++sequence}`
  globalThis[key] = dependencies
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`)
  } finally { delete globalThis[key] }
}

const CACHE_KEY = 'neri:recommend:cache'
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const playlist = (id, name) => ({ id, name, coverUrl: '', trackCount: 1 })
const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')

async function runtime(cache = null) {
  const storage = new Map(cache ? [[CACHE_KEY, JSON.stringify(cache)]] : [])
  const requests = [], toasts = [], errors = []
  globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) }
  const module = await loadStore({
    vue, pinia,
    '@tauri-apps/api/core': { invoke: (command, args) => {
      const request = deferred()
      requests.push({ command, args, ...request })
      return request.promise
    } },
    './toast': { useToastStore: () => ({ success() {}, error: message => toasts.push(message) }) },
    '@/utils/logger': { createLogger: () => ({ error: (...args) => errors.push(args), warn() {} }) },
    '@/modules/youtube/youtubePlaylistParse': {
      parseYouTubeLibraryPlaylists: data => data.playlists, parseYouTubeHomeFeed: () => [],
    },
  })
  pinia.setActivePinia(pinia.createPinia())
  return {
    store: module.useRecommendStore(), requests, toasts, errors,
    cache: () => JSON.parse(storage.get(CACHE_KEY) || '{}'),
  }
}

let cases = 0
async function regression(name, run) {
  await run()
  cases++
  console.log(`ok - ${name}`)
}

try {
  await regression('cached playlists show at once and refresh once per launch with a visible state', async () => {
    const current = await runtime({ userPlaylists: { youtube: [playlist('VL-cached', '缓存')] } })
    assert.equal(current.store.userPlaylists.youtube[0].name, '缓存', '启动即显示缓存')
    const refresh = current.store.ensureUserPlaylists('youtube')
    assert.equal(current.requests.length, 1)
    assert.deepEqual(current.requests[0].args, { platform: 'youtube' })
    assert.deepEqual(current.store.userPlaylistsStatus.youtube, { loading: true, error: null }, '刷新期间要能显示加载中')
    assert.equal(current.store.userPlaylists.youtube[0].name, '缓存', '刷新期间不清掉缓存')
    current.requests[0].resolve({ playlists: [playlist('VL-fresh', '最新')] })
    await refresh
    assert.equal(current.store.userPlaylists.youtube[0].name, '最新')
    assert.deepEqual(current.store.userPlaylistsStatus.youtube, { loading: false, error: null })
    assert.equal(current.cache().userPlaylists.youtube[0].name, '最新', '新结果写回缓存，下次启动直接显示')
    await current.store.ensureUserPlaylists('youtube')
    assert.equal(current.requests.length, 1, '同一次启动不重复拉')
  })

  await regression('concurrent requests for one platform share a single fetch', async () => {
    const current = await runtime()
    const requests = [
      current.store.fetchUserPlaylists('netease'),
      current.store.fetchUserPlaylists('netease'),
      current.store.ensureUserPlaylists('netease'),
    ]
    assert.equal(current.requests.length, 1, '并发请求合并成一个')
    current.requests[0].resolve({ playlist: [{ id: 7, name: '网易云', coverImgUrl: '', trackCount: 3 }] })
    await Promise.all(requests)
    assert.equal(current.store.userPlaylists.netease[0].name, '网易云')
  })

  await regression('a failed refresh keeps the cached list and reports the failure in place', async () => {
    const current = await runtime({ userPlaylists: { bilibili: [playlist(1, '缓存收藏夹')] } })
    const refresh = current.store.ensureUserPlaylists('bilibili')
    current.requests[0].reject(new Error('网络超时'))
    await refresh
    assert.equal(current.store.userPlaylists.bilibili[0].name, '缓存收藏夹', '失败不清掉缓存')
    assert.equal(current.store.userPlaylistsStatus.bilibili.loading, false)
    assert.match(current.store.userPlaylistsStatus.bilibili.error, /网络超时/)
    assert.equal(current.toasts.length, 0, '有缓存可看时只在页面上标出刷新失败，不弹窗')
    assert.equal(current.errors.length, 1, '失败要记日志')
    await current.store.ensureUserPlaylists('bilibili')
    assert.equal(current.requests.length, 1, '失败后不自动反复重试，由重试按钮触发')
    const retry = current.store.fetchUserPlaylists('bilibili')
    assert.deepEqual(current.store.userPlaylistsStatus.bilibili, { loading: true, error: null }, '重试时清掉上次的错误')
    current.requests[1].resolve({ data: { list: [{ id: 2, title: '新收藏夹', cover: 'cover.jpg', media_count: 4 }] } })
    await retry
    assert.equal(current.store.userPlaylists.bilibili[0].name, '新收藏夹')
  })

  await regression('a failure with nothing to show is announced instead of looking like an empty list', async () => {
    const current = await runtime()
    const load = current.store.fetchUserPlaylists('youtube')
    current.requests[0].reject(new Error('登录已失效'))
    await load
    assert.equal(current.toasts.length, 1)
    assert.equal(current.store.userPlaylists.youtube, undefined, '失败不能写入空列表冒充「暂无」')
    assert.match(current.store.userPlaylistsStatus.youtube.error, /登录已失效/)
  })

  await regression('a background prefetch fails quietly unless a page joins it', async () => {
    const current = await runtime()
    const quiet = current.store.ensureUserPlaylists('netease', { quiet: true })
    current.requests[0].reject(new Error('离线'))
    await quiet
    assert.equal(current.toasts.length, 0, '后台预取失败不弹提示')
    assert.match(current.store.userPlaylistsStatus.netease.error, /离线/, '失败仍记在状态里，进页面能看到')
    assert.equal(current.errors.length, 1, '失败要记日志')

    const prefetch = current.store.fetchUserPlaylists('bilibili', { quiet: true })
    const page = current.store.ensureUserPlaylists('bilibili')
    assert.equal(current.requests.length, 2, '页面加入同一个请求，不重复拉')
    current.requests[1].reject(new Error('超时'))
    await Promise.all([prefetch, page])
    assert.equal(current.toasts.length, 1, '页面在等结果时失败要提示')

    const next = current.store.fetchUserPlaylists('bilibili', { quiet: true })
    current.requests[2].reject(new Error('超时'))
    await next
    assert.equal(current.toasts.length, 1, '上一次页面等待不能让下一次后台请求也弹提示')
  })

  await regression('switching accounts mid-request drops the old account result', async () => {
    const current = await runtime({ userPlaylists: { youtube: [playlist('VL-A', '账号 A')] } })
    const old = current.store.ensureUserPlaylists('youtube')
    current.store.invalidatePlatform('youtube')
    assert.equal(current.store.userPlaylists.youtube, undefined)
    assert.equal(current.store.userPlaylistsStatus.youtube, undefined)
    const fresh = current.store.ensureUserPlaylists('youtube')
    assert.equal(current.requests.length, 2, '换号后重新拉，不复用旧账号的请求')
    current.requests[0].resolve({ playlists: [playlist('VL-A2', '旧账号的结果')] })
    await old
    assert.equal(current.store.userPlaylists.youtube, undefined, '旧账号的结果不能写入')
    assert.equal(current.store.userPlaylistsStatus.youtube.loading, true, '旧请求结束不能清掉新请求的加载状态')
    current.requests[1].resolve({ playlists: [playlist('VL-B', '账号 B')] })
    await fresh
    assert.equal(current.store.userPlaylists.youtube[0].name, '账号 B')
    assert.equal(current.cache().userPlaylists.youtube[0].name, '账号 B')
  })

  await regression('saved albums follow the same cache-then-refresh rules', async () => {
    const current = await runtime({ userAlbums: [{ id: 1, name: '缓存专辑' }] })
    assert.equal(current.store.userAlbums[0].name, '缓存专辑')
    const refresh = current.store.ensureUserAlbums()
    assert.equal(current.requests[0].command, 'get_user_stared_albums')
    assert.deepEqual(current.store.userAlbumsStatus, { loading: true, error: null })
    assert.equal(current.store.userAlbums[0].name, '缓存专辑', '刷新期间不清掉缓存')
    current.requests[0].resolve({ data: [{ id: 2, name: '新专辑', picUrl: '', artists: [{ name: '鹿乃' }], size: 10 }] })
    await refresh
    assert.equal(current.store.userAlbums[0].artist, '鹿乃')
    assert.deepEqual(current.store.userAlbumsStatus, { loading: false, error: null })
    await current.store.ensureUserAlbums()
    assert.equal(current.requests.length, 1, '同一次启动不重复拉')
    current.store.invalidatePlatform('netease')
    assert.deepEqual(current.store.userAlbums, [])
    assert.deepEqual(current.store.userAlbumsStatus, { loading: false, error: null })
    void current.store.ensureUserAlbums()
    assert.equal(current.requests.length, 2, '换号后专辑重新拉')
  })

  console.log(`Cloud playlist regressions: ${cases} passed`)
} finally {
  if (originalStorage) Object.defineProperty(globalThis, 'localStorage', originalStorage)
  else delete globalThis.localStorage
}
