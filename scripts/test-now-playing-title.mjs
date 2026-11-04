import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { renderToString } from 'vue/server-renderer'
import { compileScript, compileTemplate, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

const source = await readFile(new URL('../src/components/TitleBar.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, {
  id: 'now-playing-title-test', genDefaultAs: 'component', inlineTemplate: true,
})
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const dependencies = {
  vue,
  'vue-i18n': { useI18n: () => ({ t: key => key }) },
  '@tauri-apps/api/window': { getCurrentWindow: () => ({}) },
  '@/modules/shortcuts/platform': { isMacPlatform: false },
}
const component = new Function('require', 'exports', `${compiled}\nreturn component`)(name => {
  assert.ok(name in dependencies, `unexpected dependency: ${name}`)
  return dependencies[name]
}, {})

async function render(props) {
  return renderToString(vue.createSSRApp(component, {
    nowPlaying: true, hasPlaybackSession: true, albumName: 'Original album', ...props,
  }))
}

assert.match(await render({}), /player\.now_playing/, 'the default keeps the playback label visible')
assert.match(await render({ showNowPlayingTitle: true }), /player\.now_playing/)
const hidden = await render({ showNowPlayingTitle: false })
assert.doesNotMatch(hidden, /player\.now_playing/, 'disabling the title hides the fused title bar label')
assert.match(hidden, /Original album/, 'hiding the label preserves the album name')
assert.doesNotMatch(await render({ showNowPlayingTitle: false, hasPlaybackSession: false }), /player\.not_playing/)
assert.match(await render({ showNowPlayingTitle: true, hasPlaybackSession: false }), /player\.not_playing/)
assert.match(await render({ nowPlaying: false, showNowPlayingTitle: false }), /NeriPlayer/)

const appSource = await readFile(new URL('../src/App.vue', import.meta.url), 'utf8')
function findTitleBar(node) {
  if (node.tag === 'TitleBar') return node
  for (const child of node.children ?? []) {
    const result = findTitleBar(child)
    if (result) return result
  }
}
const titleBar = findTitleBar(parse(appSource).descriptor.template.ast)
assert.ok(titleBar, 'App must render the shared title bar')
const appTemplate = compileTemplate({
  source: titleBar.loc.source, filename: 'App.vue', id: 'app-now-playing-title-test',
  compilerOptions: { bindingMetadata: { TitleBar: 'setup-const' } },
})
assert.deepEqual(appTemplate.errors, [])
const appExports = {}
new Function('require', 'exports', ts.transpileModule(appTemplate.code, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(name => { assert.equal(name, 'vue'); return vue }, appExports)
const context = vue.reactive({
  isNowPlayingOpen: true, isNowPlayingMotionActive: false, nowPlayingMotionState: null,
  settingsStore: { showNowPlayingTitle: true },
  player: { hasPlaybackSession: true, currentTrack: { title: 'Original track' } },
})
const renderAppTitle = () => renderToString(vue.createSSRApp({
  render: () => appExports.render(context, [], null, { TitleBar: component }),
}))
assert.match(await renderAppTitle(), /player\.now_playing/)
context.settingsStore.showNowPlayingTitle = false
const appHidden = await renderAppTitle()
assert.doesNotMatch(appHidden, /player\.now_playing/, 'App must forward the disabled visibility setting to TitleBar')
assert.match(appHidden, /Original track/, 'App preserves track metadata when it hides the playback label')
context.player.hasPlaybackSession = false
assert.doesNotMatch(await renderAppTitle(), /player\.not_playing/)
context.settingsStore.showNowPlayingTitle = true
assert.match(await renderAppTitle(), /player\.not_playing/, 'App must forward a reenabled visibility setting')
console.log('now playing title tests passed')
