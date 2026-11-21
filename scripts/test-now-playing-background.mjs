import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
import { computed, ref } from 'vue'

const sourceUrl = new URL('../src/utils/nowPlayingBackground.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { shouldShowDynamicBackground } = await import(moduleUrl)

const palette = {
  shaderColors: [],
  lightOffset: 0,
  saturateOffset: 0,
  accentBg: [18, 18, 18],
  primaryColor: [18, 18, 18],
  dominant: [18, 18, 18],
  lightVibrant: [18, 18, 18],
  muted: [18, 18, 18],
  darkMuted: [18, 18, 18],
}

assert.equal(shouldShowDynamicBackground(false, true, palette), false)
assert.equal(shouldShowDynamicBackground(true, false, palette), false)
assert.equal(shouldShowDynamicBackground(true, true, null), false)
assert.equal(shouldShowDynamicBackground(true, true, palette), true)

const component = await readFile(new URL('../src/components/NowPlaying.vue', import.meta.url), 'utf8')
const script = component.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)?.[1]
assert.ok(script)
const syntax = ts.createSourceFile('NowPlaying.ts', script, ts.ScriptTarget.Latest, true)
const colorStatements = syntax.statements.filter(statement => ts.isVariableStatement(statement)
  && statement.declarationList.declarations.some(declaration => ts.isIdentifier(declaration.name)
    && ['dynamicColorVars', 'sliderActiveColor'].includes(declaration.name.text)))
assert.equal(colorStatements.length, 2)
const printer = ts.createPrinter()
const colorSource = colorStatements.map(statement => printer.printNode(ts.EmitHint.Unspecified, statement, syntax)).join('\n')
const colorCode = ts.transpileModule(colorSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText
// 配色换算拆到 now-playing/dynamicColors.ts，页面里的两个计算属性仍从 NowPlaying.vue 取
const dynamicColorsCompiled = ts.transpileModule(
  await readFile(new URL('../src/components/now-playing/dynamicColors.ts', import.meta.url), 'utf8'),
  { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } },
).outputText
const { buildDynamicColorVars } = await import(`data:text/javascript;base64,${Buffer.from(dynamicColorsCompiled).toString('base64')}`)

function colorsFor(seed, hasPlaybackSession = true) {
  return new Function('computed', 'player', 'paletteResult', 'buildDynamicColorVars', `${colorCode}\nreturn { vars: dynamicColorVars.value, active: sliderActiveColor.value }`)(
    computed, { hasPlaybackSession }, ref(seed ? { ...palette, lightVibrant: seed } : null), buildDynamicColorVars,
  )
}

function rgb(color) {
  const channels = color.match(/^rgb\((\d+),\s*(\d+),\s*(\d+)\)$/)
  assert.ok(channels, `expected an RGB color, got ${color}`)
  return channels.slice(1).map(Number)
}

function luminance(channels) {
  const linear = channels.map(value => {
    const channel = value / 255
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
  })
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}

function contrast(left, right) {
  const values = [luminance(left), luminance(right)].sort((a, b) => b - a)
  return (values[0] + 0.05) / (values[1] + 0.05)
}

for (const seed of [[32, 32, 180], [180, 32, 32], [32, 180, 32], [32, 180, 180]]) {
  const { vars, active } = colorsFor(seed)
  const primary = rgb(vars['--np-primary'])
  assert.ok(contrast(primary, [18, 18, 18]) >= 7, `accent remains visible on the dark player for ${seed}: ${primary}`)
  assert.ok(contrast(rgb(vars['--np-primary-container']), rgb(vars['--np-on-primary'])) >= 7, 'play icon contrasts with its button')
  assert.equal(active, vars['--np-primary'], 'progress and active controls share the readable accent')
}
const neutral = rgb(colorsFor([128, 128, 128]).vars['--np-primary'])
assert.equal(Math.max(...neutral), Math.min(...neutral), 'neutral palette colors do not acquire a red tint')
assert.deepEqual(colorsFor(null), { vars: {}, active: '#fff' })
assert.deepEqual(colorsFor([32, 32, 180], false), { vars: {}, active: '#fff' })

console.log('now playing background tests passed')
