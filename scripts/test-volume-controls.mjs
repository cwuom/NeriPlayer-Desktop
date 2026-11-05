import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { compileStyleAsync, compileTemplate, parse } from 'vue/compiler-sfc'
import ts from 'typescript'

async function volumeSlider(component, className) {
  const source = await readFile(new URL(`../src/components/${component}.vue`, import.meta.url), 'utf8')
  const descriptor = parse(source).descriptor
  function find(node) {
    if (node.tag === 'input' && node.props?.some(prop => prop.name === 'class' && prop.value?.content === className)) return node
    for (const child of node.children ?? []) {
      const result = find(child)
      if (result) return result
    }
  }
  const input = find(descriptor.template.ast)
  assert.ok(input, `${component} must render its volume slider`)
  const template = compileTemplate({
    source: input.loc.source, filename: `${component}.vue`, id: 'volume-controls-test',
    compilerOptions: { expressionPlugins: ['typescript'] },
  })
  assert.deepEqual(template.errors, [])
  const exports = {}
  new Function('require', 'exports', ts.transpileModule(template.code, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText)(name => { assert.equal(name, 'vue'); return vue }, exports)
  const player = vue.reactive({ volume: 0.5, setVolume(value) { this.volume = value } })
  const bindings = { player }
  if (component === 'MiniPlayer') {
    const script = ts.createSourceFile(`${component}.ts`, descriptor.scriptSetup.content, ts.ScriptTarget.ES2022, true)
    const percent = script.statements.filter(ts.isVariableStatement)
      .flatMap(statement => [...statement.declarationList.declarations])
      .find(declaration => ts.isIdentifier(declaration.name) && declaration.name.text === 'volumePercent')
    assert.ok(percent?.initializer, 'MiniPlayer must derive its displayed volume from player state')
    const computed = ts.transpileModule(`const actual = ${percent.initializer.getText(script)}`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText
    bindings.volumePercent = new Function('computed', 'player', `${computed}\nreturn actual`)(vue.computed, player)
  }
  const context = vue.proxyRefs(bindings)
  return { player, descriptor, render: () => exports.render(context, []) }
}

let failures = 0
async function test(name, run) {
  try { await run(); console.log(`passed: ${name}`) }
  catch (error) { failures++; console.error(`failed: ${name}: ${error.message}`) }
}

for (const [component, className] of [['NowPlaying', 'volume-slider'], ['MiniPlayer', 'mp-volume-slider']]) {
  await test(`${component} keeps native range increases aligned with louder playback`, async () => {
    const slider = await volumeSlider(component, className)
    for (const [volume, nativeStep, expected] of [[0.5, 1, 0.51], [0.5, -1, 0.49], [0, 1, 0.01], [1, -1, 0.99]]) {
      slider.player.volume = volume
      const props = slider.render().props
      const nativeValue = Math.max(Number(props.min), Math.min(Number(props.max), props.value + nativeStep * Number(props.step)))
      props.onInput({ target: { value: String(nativeValue) } })
      assert.equal(slider.player.volume, expected)
    }
  })

  await test(`${component} places maximum volume above mute after styling`, async () => {
    const slider = await volumeSlider(component, className)
    const style = slider.descriptor.styles[0]
    const compiled = await compileStyleAsync({
      source: style.content, filename: `${component}.vue`, id: 'volume-controls-test',
      preprocessLang: style.lang,
    })
    assert.deepEqual(compiled.errors, [])
    let transform = 'none'
    compiled.rawResult.root.walkRules(rule => {
      if (rule.selector !== `.${className}`) return
      rule.walkDecls('transform', declaration => { transform = declaration.value })
    })
    const rotation = /^rotate\(([-+\d.]+)(deg|rad|turn)\)$/.exec(transform)
    assert.ok(rotation, 'the horizontal range must rotate into a vertical control')
    const radians = Number(rotation[1]) * ({ deg: Math.PI / 180, rad: 1, turn: Math.PI * 2 })[rotation[2]]
    const minimum = { x: -50 * Math.cos(radians), y: -50 * Math.sin(radians) }
    const maximum = { x: 50 * Math.cos(radians), y: 50 * Math.sin(radians) }
    assert.ok(Math.abs(maximum.x - minimum.x) < 0.001, 'the range endpoints must align vertically')
    assert.ok(maximum.y < minimum.y, 'increasing the native range value must move the thumb upward')
  })
}

await test('MiniPlayer fills the track from mute to the current volume', async () => {
  const slider = await volumeSlider('MiniPlayer', 'mp-volume-slider')
  for (const [volume, expectedPercent] of [[0, 0], [0.25, 25], [0.5, 50], [1, 100]]) {
    slider.player.volume = volume
    const background = slider.render().props.style?.background ?? ''
    assert.match(background, /^linear-gradient\(to right,/, 'the active fill must start at the minimum end')
    const stops = [...background.matchAll(/\)\s+([\d.]+)%/g)].map(match => Number(match[1]))
    assert.deepEqual(stops, [expectedPercent, expectedPercent], 'active and inactive tracks must meet at the current volume')
    assert.ok(background.indexOf('var(--md-primary)') < background.indexOf('var(--md-surface-container-highest)'),
      'the filled portion must use the current theme primary color')
  }
})

if (failures) process.exitCode = 1
