import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import ts from 'typescript'
import { loadModule, subtitle, deferred, flush } from './bili-subtitle-test-loader.mjs'

const source = readFileSync(new URL('../src/components/BiliSubtitleSettings.vue', import.meta.url), 'utf8')
const script = compileScript(parse(source).descriptor, { id: 'bili-subtitle-settings-test', genDefaultAs: 'component' })
const compiled = ts.transpileModule(script.content, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
const renderer = vue.createRenderer({
  createElement: type => ({ type, children: [] }), createText: text => ({ text }), createComment: text => ({ text }),
  setText() {}, setElementText() {}, patchProp() {}, parentNode: node => node.parent, nextSibling: () => null,
  insert(node, parent) { node.parent = parent; parent.children.push(node) },
  remove(node) { node.parent.children = node.parent.children.filter(child => child !== node) },
})
const video = cid => ({ id: `bilibili:170001:${cid}`, album: `Bilibili|${cid}|BV1xx411c7mD` })
const result = language => ({ tracks: [{ language, label: `label-${language}`, isAi: language.startsWith('ai-') }], loginRequired: false })

function mount(track = null, handler = async () => result('zh-CN'), overrides = {}) {
  const settings = vue.reactive({ biliCcSubtitleEnabled: true, biliCcSubtitleLanguage: '', biliCcSubtitleAiFallback: true, biliCcSubtitleOffset: 0, ...overrides })
  const revision = vue.ref(0)
  const calls = []
  const dependencies = {
    vue, 'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '@/stores/settings': { useSettingsStore: () => settings }, '@/modules/lyrics/biliSubtitle': subtitle,
    '@/modules/lyrics/biliSubtitleLoader': {
      biliSubtitleRevision: revision, invalidateBiliSubtitleCache: () => { revision.value++ },
      loadBiliSubtitleTracks: async track => { calls.push(track); return handler(track) },
    },
    '@/modules/lyrics/lyricOffset': loadModule('../src/modules/lyrics/lyricOffset.ts'),
    '@/components/ui/CustomSelect.vue': {},
  }
  const component = new Function('require', 'exports', `${compiled}\nreturn component`)(name => {
    assert.ok(name in dependencies, name); return dependencies[name]
  }, {})
  let bindings
  const setup = component.setup
  component.setup = (props, context) => { bindings = setup(props, context); return () => null }
  const input = vue.reactive({ track })
  const app = renderer.createApp({ setup: () => () => vue.h(component, input) })
  app.mount({ children: [] })
  return { bindings, input, settings, calls, revision, stop: () => app.unmount() }
}
let cases = 0
async function test(name, run) { await run(); cases++; console.log(`PASS ${name}`) }

await test('settings-only controls preserve a selected language and need no video request', async () => {
  const r = mount(null, undefined, { biliCcSubtitleLanguage: 'ko' })
  try {
    await flush()
    assert.equal(r.calls.length, 0)
    assert.ok(r.bindings.languageOptions.value.some(option => option.value === 'ko'))
    assert.equal(r.bindings.languageOptions.value[0].value, '')
    r.bindings.updateOffset({ target: { value: '-75' } })
    assert.equal(r.settings.biliCcSubtitleOffset, -50)
    r.bindings.updateOffset({ target: { value: '9000' } })
    assert.equal(r.settings.biliCcSubtitleOffset, 5000)
  } finally { r.stop() }
})
await test('disabled controls avoid loading and enabling loads the current video', async () => {
  const r = mount(video(22), undefined, { biliCcSubtitleEnabled: false })
  try {
    await flush(); assert.equal(r.calls.length, 0)
    r.settings.biliCcSubtitleEnabled = true; await flush()
    assert.equal(r.calls.length, 1)
    assert.equal(r.bindings.languageOptions.value[1].label, 'label-zh-CN')
    assert.equal(r.bindings.status.value, 'bili_subtitle.tracks_available')
  } finally { r.stop() }
})
await test('login-required, empty and failed results display distinct retryable states', async () => {
  for (const [handler, expected] of [
    [async () => ({ tracks: [], loginRequired: true }), 'login_required'],
    [async () => ({ tracks: [], loginRequired: false }), 'empty'],
    [async () => { throw new Error('offline') }, 'failed'],
  ]) {
    const r = mount(video(22), handler)
    try { await flush(); assert.equal(r.bindings.status.value, `bili_subtitle.${expected}`); r.revision.value++; await flush(); assert.equal(r.calls.length, 2) }
    finally { r.stop() }
  }
})
await test('late lists cannot replace a new video or repopulate disabled controls', async () => {
  const late = deferred()
  const r = mount(video(22), track => track.id.endsWith(':22') ? late.promise : result('ai-en'))
  try {
    await flush(); r.input.track = video(33); await flush()
    late.resolve(result('ja')); await flush()
    assert.equal(r.bindings.languageOptions.value[1].value, 'ai-en')
    assert.match(r.bindings.languageOptions.value[1].label, /AI/)
    r.settings.biliCcSubtitleEnabled = false; await flush()
    assert.deepEqual(r.bindings.tracks.value, [])
    assert.equal(r.bindings.loading.value, false)
  } finally { r.stop() }
})
await test('unmount rejects pending UI updates', async () => {
  const late = deferred()
  const r = mount(video(22), () => late.promise)
  r.stop(); late.resolve(result('ja')); await flush()
  assert.deepEqual(r.bindings.tracks.value, [])
})
console.log(`Bilibili subtitle controls: ${cases} passed`)
