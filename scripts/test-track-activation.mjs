import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const settings = { doubleClickPlay: false }
const source = await readFile(new URL('../src/composables/useTrackActivation.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
const exports = {}
new Function('require', 'exports', compiled)(name => {
  if (name === '@/stores/settings') return { useSettingsStore: () => settings }
  throw new Error(`Unexpected activation dependency: ${name}`)
}, exports)
let onTrackClick
let onTrackDoubleClick

let cases = 0
function regression(name, run) {
  settings.doubleClickPlay = false
  ;({ onTrackClick, onTrackDoubleClick } = exports.useTrackActivation())
  run()
  cases++
  console.log(`ok - ${name}`)
}
const row = {}
const mouse = (detail, interactive = null, currentTarget = row) => ({
  detail,
  currentTarget,
  target: { closest: selector => {
    assert.equal(selector, 'button, input, a')
    return interactive
  } },
})

regression('single-click mode activates only once for a browser double-click sequence', () => {
  let count = 0
  const play = () => { count++ }
  onTrackClick(mouse(1), play)
  onTrackClick(mouse(2), play)
  onTrackDoubleClick(mouse(2), play)
  assert.equal(count, 1)
})

regression('double-click mode ignores mouse clicks and activates once on double-click', () => {
  settings.doubleClickPlay = true
  let count = 0
  const play = () => { count++ }
  onTrackClick(mouse(1), play)
  onTrackClick(mouse(2), play)
  assert.equal(count, 0)
  onTrackDoubleClick(mouse(2), play)
  assert.equal(count, 1)
})

regression('keyboard and programmatic button clicks activate in either mode', () => {
  let count = 0
  const play = () => { count++ }
  onTrackClick(mouse(0), play)
  settings.doubleClickPlay = true
  onTrackClick(mouse(0), play)
  onTrackClick(undefined, play)
  assert.equal(count, 3)
})

regression('selection toggles only on the first click regardless of playback preference', () => {
  for (const mode of [false, true]) {
    settings.doubleClickPlay = mode
    let selected = false
    const toggle = () => { selected = !selected }
    onTrackClick(mouse(1), toggle, true)
    onTrackClick(mouse(2), toggle, true)
    onTrackDoubleClick(mouse(2), toggle, true)
    assert.equal(selected, true)
  }
})

regression('double-clicks from nested controls do not activate the row', () => {
  settings.doubleClickPlay = true
  let count = 0
  onTrackDoubleClick(mouse(2, {}), () => { count++ })
  assert.equal(count, 0)
})

regression('clearing the last selection never turns the same double-click sequence into playback', () => {
  settings.doubleClickPlay = true
  let selected = true
  let count = 0
  const activate = () => {
    if (selected) selected = false
    else count++
  }
  onTrackClick(mouse(1), activate, selected)
  onTrackClick(mouse(2), activate, selected)
  onTrackDoubleClick(mouse(2), activate, selected)
  assert.equal(selected, false)
  assert.equal(count, 0)
  onTrackClick(mouse(1), activate, selected)
  onTrackDoubleClick(mouse(2), activate, selected)
  assert.equal(count, 1)
})

regression('a track button itself remains a valid double-click target', () => {
  settings.doubleClickPlay = true
  let count = 0
  onTrackDoubleClick(mouse(2, row), () => { count++ })
  assert.equal(count, 1)
})

console.log(`track activation regression cases passed: ${cases}`)
