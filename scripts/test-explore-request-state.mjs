import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`

const vueUrl = dataModule(`
  export const ref = value => ({ value })
  export const computed = getter => ({ get value() { return getter() } })
  export const watch = () => {}
  export const onMounted = () => {}
`)
const routerUrl = dataModule('export const useRouter = () => ({ push() {} })')
const i18nUrl = dataModule('export const useI18n = () => ({ t: key => key })')
const searchUrl = dataModule(`
  export const useSearchStore = () => globalThis.__exploreSearchStore
`)
const playerUrl = dataModule('export const usePlayerStore = () => ({ play() {} })')
const recommendUrl = dataModule('export const useRecommendStore = () => globalThis.__exploreRecommendStore')
const coverUrl = dataModule('export default {}')

const sourceUrl = new URL('../src/views/ExploreView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const setupMatch = source.match(/<script setup[^>]*>\r?\n([\s\S]*?)\r?\n<\/script>/)
assert.ok(setupMatch, 'ExploreView must contain a script setup block')

let setupSource = setupMatch[1]
  .replace("from 'vue'", `from '${vueUrl}'`)
  .replace("from 'vue-router'", `from '${routerUrl}'`)
  .replace("from 'vue-i18n'", `from '${i18nUrl}'`)
  .replace("from '@/stores/search'", `from '${searchUrl}'`)
  .replace("from '@/stores/player'", `from '${playerUrl}'`)
  .replace("from '@/stores/recommend'", `from '${recommendUrl}'`)
  .replace("from '@/components/BilibiliCoverImage.vue'", `from '${coverUrl}'`)
  .replace(/defineOptions\(\{ name: 'ExploreView' \}\)/, 'void 0')

setupSource += `\n\nglobalThis.__exploreTest = { loadQualityByTag, qualityPlaylists, isLoadingPlaylists, selectedTag }\n`

const compiled = ts.transpileModule(setupSource, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText

globalThis.__exploreSearchStore = {
  results: [],
  isSearching: false,
  error: null,
  clear() {},
  search() {},
}
const qualityRequests = []
globalThis.__exploreRecommendStore = {
  fetchHighQualityPlaylists(cat, limit) {
    const request = { cat, limit }
    qualityRequests.push(request)
    return new Promise((resolve, reject) => {
      request.resolve = resolve
      request.reject = reject
    })
  },
}

await import(dataModule(compiled))
const state = globalThis.__exploreTest

const stale = state.loadQualityByTag('tag_pop')
await Promise.resolve()
const fresh = state.loadQualityByTag('tag_rock')
await Promise.resolve()
assert.equal(qualityRequests.length, 2)

qualityRequests[0].resolve([{ id: 'old-playlist' }])
await stale
assert.equal(
  state.isLoadingPlaylists.value,
  true,
  'a stale tag response must not end the latest request loading state',
)

qualityRequests[1].resolve([{ id: 'fresh-playlist' }])
await fresh
assert.equal(state.isLoadingPlaylists.value, false)
assert.equal(state.qualityPlaylists.value[0].id, 'fresh-playlist')

const staleAgain = state.loadQualityByTag('tag_pop')
await Promise.resolve()
const freshAgain = state.loadQualityByTag('tag_rock')
await Promise.resolve()
qualityRequests[3].resolve([{ id: 'fresh-again' }])
await freshAgain
qualityRequests[2].resolve([{ id: 'old-again' }])
await staleAgain
assert.equal(
  state.qualityPlaylists.value[0].id,
  'fresh-again',
  'a stale tag response must not overwrite the latest playlists',
)

assert.match(source, /qualityRequestGeneration/)
assert.match(source, /qualityError/)
assert.match(source, /searchStore\.error/)
assert.match(source, /player\.retry/)

console.log('explore request state tests passed')
