import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { renderToString } from 'vue/server-renderer'
import { compileScript, parse } from 'vue/compiler-sfc'
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
console.log('now playing title tests passed')
