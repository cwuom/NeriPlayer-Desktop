import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import * as pinyinPro from 'pinyin-pro'
import * as vue from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import { loadModule, subtitle } from './bili-subtitle-test-loader.mjs'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
const view = (await read('../src/views/SettingsView.vue')).replace(/\r\n/g, '\n')
const indexSource = await read('../src/modules/settings/searchIndex.ts')
const exports = {}
new Function('exports', ts.transpileModule(indexSource, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(exports)
const index = exports.SETTINGS_SEARCH_INDEX

const sectionIds = [...view.matchAll(/^\s+\| '(\w+)'$/gm)].map(match => match[1])
assert.ok(sectionIds.length >= 15, 'SettingsSectionId union is parsed')

const template = view.slice(view.indexOf('<template>'))
const panels = [...template.matchAll(/<div v-show="activeSettingsSection === '(\w+)'" class="settings-section-panel" data-section="(\w+)">/g)]
assert.ok(panels.length > 0, 'every settings panel carries data-section')
for (const [, shown, tagged] of panels) {
  assert.equal(shown, tagged, `panel ${shown} is tagged with its own section id`)
  assert.ok(sectionIds.includes(shown), `panel ${shown} is a declared section`)
}
for (const id of sectionIds) {
  assert.ok(panels.some(panel => panel[1] === id), `section ${id} has a panel`)
}

const indexed = new Set(index.map(entry => `${entry.section}:${entry.title}`))
const missing = []
panels.forEach((panel, i) => {
  const body = template.slice(panel.index, panels[i + 1]?.index ?? template.length)
  for (const match of body.matchAll(/class="setting-title"[^>]*>\{\{ t\('([\w.]+)'\) \}\}/g)) {
    if (!indexed.has(`${panel[1]}:${match[1]}`)) missing.push(`${panel[1]}:${match[1]}`)
  }
})
assert.deepEqual(missing, [], 'every static setting title is searchable')
for (const entry of index) assert.ok(sectionIds.includes(entry.section), `index entry ${entry.title} points to a real section`)

const matcher = {}
new Function('require', 'exports', ts.transpileModule(await read('../src/modules/search/textMatcher.ts'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText)(name => { assert.equal(name, 'pinyin-pro'); return pinyinPro }, matcher)
const parsed = ts.createSourceFile('SettingsView.ts', view.slice(view.indexOf('<script setup lang="ts">') + '<script setup lang="ts">'.length, view.indexOf('</script>')), ts.ScriptTarget.ES2022, true)
const searchStatement = parsed.statements.find(statement => ts.isVariableStatement(statement)
  && statement.declarationList.declarations.some(declaration => declaration.name.getText(parsed) === 'settingsSearchResults'))
const searchCompiled = ts.transpileModule(searchStatement.getText(parsed), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
for (const mac of [false, true]) {
  const results = new Function('computed', 'settingsQuery', 'settingsNavGroups', 'SETTINGS_SEARCH_INDEX', 'SETTINGS_SECTION_IDS', 't', 'filterAndRank', 'searchValue', 'isMacPlatform', `${searchCompiled}\nreturn settingsSearchResults`)(
    callback => callback(), { value: 'macOS' }, { value: [] }, index, sectionIds, key => key,
    matcher.filterAndRank, matcher.searchValue, mac,
  )
  assert.equal(results.some(result => result.key === 'desktop_lyrics:desktop_lyrics.menu_bar'), mac,
    'macOS-only controls must only appear in search on macOS')
}

for (const [query, expected] of [
  ['长音', 'lyrics:settings.lyric_emphasis_threshold'],
  ['long note', 'lyrics:settings.lyric_emphasis_threshold'],
  ['滚动', 'desktop_lyrics:desktop_lyrics.max_width'],
  ['max width', 'desktop_lyrics:desktop_lyrics.max_width'],
]) {
  const results = new Function('computed', 'settingsQuery', 'settingsNavGroups', 'SETTINGS_SEARCH_INDEX', 'SETTINGS_SECTION_IDS', 't', 'filterAndRank', 'searchValue', 'isMacPlatform', `${searchCompiled}\nreturn settingsSearchResults`)(
    callback => callback(), { value: query }, { value: [] }, index, sectionIds, key => key,
    matcher.filterAndRank, matcher.searchValue, false,
  )
  assert.ok(results.some(result => result.key === expected),
    `lyric preferences must be discoverable with ${query}`)
}

// Render the child component and exercise the parent's real search-focus function.
const subtitleSource = await read('../src/components/BiliSubtitleSettings.vue')
const subtitleScript = compileScript(parse(subtitleSource).descriptor, {
  id: 'subtitle-search', genDefaultAs: 'component', inlineTemplate: true,
})
const subtitleCompiled = ts.transpileModule(subtitleScript.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const settings = vue.reactive({ biliCcSubtitleEnabled: true, biliCcSubtitleLanguage: 'zh', biliCcSubtitleAiFallback: true, biliCcSubtitleOffset: 0 })
const dependencies = {
  vue, 'vue-i18n': { useI18n: () => ({ t: key => key, locale: vue.ref('en') }) },
  '@/stores/settings': { useSettingsStore: () => settings },
  '@/modules/lyrics/biliSubtitle': subtitle,
  '@/modules/lyrics/biliSubtitleLoader': { biliSubtitleRevision: vue.ref(0), invalidateBiliSubtitleCache() {}, loadBiliSubtitleTracks: async () => ({ tracks: [] }) },
  '@/modules/lyrics/lyricOffset': loadModule('../src/modules/lyrics/lyricOffset.ts'),
  '@/components/ui/CustomSelect.vue': { default: { render: () => null } },
}
const subtitleComponent = new Function('require', 'exports', `${subtitleCompiled}\nreturn component`)(name => {
  assert.ok(name in dependencies, name); return dependencies[name]
}, {})
function node(type, text = '') {
  const element = {
    type, text, props: {}, children: [], parent: null, scrolls: [], offsetWidth: 1,
    addEventListener() {},
    get textContent() { return this.text + this.children.map(child => child.textContent).join('') },
    matches(selector) { return (this.props.class ?? '').split(/\s+/).includes(selector.slice(1)) },
    querySelectorAll(selector) {
      return this.children.flatMap(child => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)])
    },
    closest(selector) { return this.matches(selector) ? this : this.parent?.closest(selector) },
    scrollIntoView(options) { this.scrolls.push(options) },
  }
  element.classList = {
    add(value) { element.props.class = `${element.props.class ?? ''} ${value}`.trim() },
    remove(value) { element.props.class = (element.props.class ?? '').split(/\s+/).filter(name => name !== value).join(' ') },
  }
  return element
}
const renderer = vue.createRenderer({
  createElement: type => node(type), createText: text => node('text', text), createComment: () => node('comment'),
  setText(element, text) { element.text = text },
  setElementText(element, text) { element.text = text; element.children = [] },
  patchProp(element, key, _, value) { element.props[key] = value },
  parentNode: element => element.parent,
  nextSibling: element => element.parent?.children[element.parent.children.indexOf(element) + 1] ?? null,
  insert(element, parent) { element.parent = parent; parent.children.push(element) },
  remove(element) { element.parent.children = element.parent.children.filter(child => child !== element) },
})
const focusStatement = parsed.statements.find(statement => ts.isFunctionDeclaration(statement) && statement.name?.text === 'focusSettingsItem')
const focusCompiled = ts.transpileModule(focusStatement.getText(parsed), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const root = node('panel'), timers = []
const focus = new Function('settingsContentRef', 'audioEffectsSettingsRef', 't', 'setTimeout', `${focusCompiled}\nreturn focusSettingsItem`)(
  { value: { querySelector(selector) { assert.equal(selector, '.settings-section-panel[data-section="lyrics"]'); return root } } },
  { value: null }, key => key, (callback, delay) => timers.push({ callback, delay }),
)
const app = renderer.createApp(subtitleComponent, { variant: 'settings', showOffset: false })
app.mount(root)
try {
  for (const title of ['bili_subtitle.title', 'bili_subtitle.language', 'bili_subtitle.ai_fallback']) {
    assert.ok(index.some(entry => entry.section === 'lyrics' && entry.title === title))
    await focus('lyrics', title)
    const highlighted = root.querySelectorAll('.search-highlight')
    assert.equal(highlighted.length, 1, `${title} must identify a rendered child card`)
    const card = highlighted[0]
    assert.ok(card.querySelectorAll('.setting-title').some(element => element.textContent === title))
    assert.deepEqual(card.scrolls, [{ block: 'center', behavior: 'smooth' }])
    assert.deepEqual(timers.map(timer => timer.delay), [1800], 'a rendered result needs no retries')
    timers.shift().callback()
    assert.deepEqual(root.querySelectorAll('.search-highlight'), [])
  }
} finally { app.unmount() }

console.log(`Settings search index covers ${index.length} entries across ${panels.length} panels, including rendered subtitle cards`)
