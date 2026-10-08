import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import * as pinia from 'pinia'
import ts from 'typescript'

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
  const key = `__youtubeHomeFeedTest${++sequence}`
  globalThis[key] = dependencies
  try {
    return await import(`data:text/javascript;base64,${Buffer.from(`const deps = globalThis[${JSON.stringify(key)}];\n${compiled}`).toString('base64')}`)
  } finally { delete globalThis[key] }
}

const CACHE_KEY = 'neri:recommend:cache'
const shelf = title => ({ title, items: [{ title: `歌曲 ${title}`, subtitle: '', coverUrl: '', videoId: title }] })
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
async function runtime(cache = null) {
  const storage = new Map(cache ? [[CACHE_KEY, JSON.stringify(cache)]] : []), requests = [], errors = []
  globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) }
  const module = await loadStore({
    vue, pinia,
    '@tauri-apps/api/core': { invoke: command => {
      if (command === 'get_user_playlists') return Promise.resolve([])
      assert.equal(command, 'get_home_feed')
      const request = deferred()
      requests.push(request)
      return request.promise
    } },
    './toast': { useToastStore: () => ({ success() {}, error() {} }) },
    '@/utils/logger': { createLogger: () => ({ error: (...args) => errors.push(args) }) },
    '@/modules/youtube/youtubePlaylistParse': {
      parseYouTubeLibraryPlaylists: () => [], parseYouTubeHomeFeed: data => data.shelves,
    },
  })
  pinia.setActivePinia(pinia.createPinia())
  return { store: module.useRecommendStore(), requests, storage, errors, cache: () => JSON.parse(storage.get(CACHE_KEY) || '{}') }
}

try {
  {
    const current = await runtime({ homeFeedShelves: [shelf('账号 A')], userPlaylists: { youtube: [{ id: 'VL-A' }], netease: [{ id: 1 }] } })
    assert.deepEqual(current.store.homeFeedShelves, [], '首页不能恢复无账号身份的旧盘缓存')
    current.store.invalidatePlatform('youtube')
    assert.deepEqual(current.store.homeFeedShelves, [], '账号退出必须清首页内存')
    assert.equal(current.cache().homeFeedShelves, undefined, '账号退出必须清首页持久缓存')
    assert.equal(current.store.userPlaylists.youtube, undefined)
    assert.equal(current.store.userPlaylists.netease[0].id, 1)
    const reloaded = await runtime(current.cache())
    assert.deepEqual(reloaded.store.homeFeedShelves, [], '重启不能恢复已清除的首页')
  }
  {
    const current = await runtime()
    const old = current.store.fetchHomeFeed()
    assert.equal(current.store.homeFeedLoading, true)
    current.store.invalidatePlatform('youtube')
    assert.equal(current.store.isLoading, false, '退出后不能遗留旧请求 loading')
    assert.equal(current.store.homeFeedLoading, false)
    current.requests[0].resolve({ shelves: [shelf('已退出账号')] })
    await old
    assert.deepEqual(current.store.homeFeedShelves, [])
    assert.equal(current.cache().homeFeedShelves, undefined)
    assert.equal(current.store.homeFeedLoading, false)
  }
  {
    const current = await runtime()
    const old = current.store.fetchHomeFeed()
    current.store.invalidatePlatform('youtube')
    const fresh = current.store.fetchHomeFeed()
    current.requests[0].resolve({ shelves: [shelf('账号 A')] })
    await old
    assert.deepEqual(current.store.homeFeedShelves, [])
    assert.equal(current.store.isLoading, true, '旧请求完成不能清掉新请求 loading')
    assert.equal(current.store.homeFeedLoading, true)
    current.requests[1].resolve({ shelves: [shelf('账号 B')] })
    await fresh
    assert.equal(current.store.homeFeedShelves[0].title, '账号 B')
    assert.equal(current.cache().homeFeedShelves, undefined, '个性化首页只保留当前会话内存')
    assert.equal(current.store.isLoading, false)
    assert.equal(current.store.homeFeedLoading, false)
  }
  {
    const current = await runtime()
    const first = current.store.fetchHomeFeed(), last = current.store.fetchHomeFeed()
    current.requests[1].resolve({ shelves: [shelf('最新刷新')] })
    await last
    current.requests[0].resolve({ shelves: [shelf('旧刷新')] })
    await first
    assert.equal(current.store.homeFeedShelves[0].title, '最新刷新')
    assert.equal(current.cache().homeFeedShelves, undefined)
  }
  {
    const current = await runtime()
    const old = current.store.fetchHomeFeed()
    current.store.invalidatePlatform('youtube')
    const fresh = current.store.fetchHomeFeed()
    current.requests[0].reject(new Error('旧账号失败'))
    await old
    assert.equal(current.errors.length, 0, '过期账号请求错误不再污染当前会话')
    assert.equal(current.store.isLoading, true)
    assert.equal(current.store.homeFeedLoading, true)
    current.requests[1].resolve({ shelves: [shelf('当前账号')] })
    await fresh
  }
  {
    const current = await runtime()
    current.store.homeFeedShelves = [shelf('缓存')]
    const request = current.store.fetchHomeFeed()
    current.requests[0].reject(new Error('当前请求失败'))
    await request
    assert.equal(current.errors.length, 1)
    assert.equal(current.store.homeFeedShelves[0].title, '缓存')
    assert.equal(current.store.isLoading, false)
    assert.equal(current.store.homeFeedLoading, false)
    current.store.invalidatePlatform('netease')
    assert.equal(current.store.homeFeedShelves[0].title, '缓存', '其他平台失效不能清掉 YouTube 首页')
  }
  {
    const current = await runtime()
    const home = current.store.fetchHomeFeed()
    assert.equal(current.store.homeFeedLoading, true)
    await current.store.fetchUserPlaylists('youtube')
    assert.equal(current.store.isLoading, false, '共享 loading 已被用户歌单请求完成清除')
    assert.equal(current.store.homeFeedLoading, true, '用户歌单完成不能清除仍在加载的首页状态')
    assert.deepEqual(current.store.homeFeedShelves, [])
    current.requests[0].resolve({ shelves: [shelf('较慢的首页')] })
    await home
    assert.equal(current.store.homeFeedLoading, false)
    assert.equal(current.store.homeFeedShelves[0].title, '较慢的首页')
  }
  console.log('youtube home feed tests passed')
} finally {
  if (originalStorage) Object.defineProperty(globalThis, 'localStorage', originalStorage)
  else delete globalThis.localStorage
}
