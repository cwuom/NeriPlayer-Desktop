import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import * as vue from 'vue'
import { parse } from 'vue/compiler-sfc'
import ts from 'typescript'

// 桌面歌词外观与时间轴：规整（歌词窗口不信任收到的字段）、配色、CSS 变量、逐字进度、锚点插值
async function load(path) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
}

const style = await load('../src/modules/desktopLyrics/style.ts')
const timeline = await load('../src/modules/desktopLyrics/timeline.ts')

// ---- 规整 ----
const defaults = style.normalizeDesktopLyricsStyle(undefined)
assert.deepEqual(defaults, { ...style.DEFAULT_DESKTOP_LYRICS_STYLE, playedColors: [...defaults.playedColors], unplayedColors: [...defaults.unplayedColors] })
assert.equal(defaults.layout, 'single')
assert.equal(defaults.locked, false)
assert.equal(defaults.bounds, null, '第一次打开由后端摆到屏幕底部居中')
assert.equal(defaults.maxWidth, 860, '旧设置缺少最大宽度时保留默认窗口的显示规模')
assert.equal(style.normalizeDesktopLyricsStyle({ maxWidth: Number.NaN }).maxWidth, 860)
assert.equal(style.normalizeDesktopLyricsStyle({ maxWidth: 'invalid' }).maxWidth, 860)
assert.equal(style.normalizeDesktopLyricsStyle({ maxWidth: 1209 }).maxWidth, 1200, '最大宽度使用逻辑像素并按滑块步进保存')
assert.equal(style.normalizeDesktopLyricsStyle({ maxWidth: 0 }).maxWidth, 360)
assert.equal(style.normalizeDesktopLyricsStyle({ maxWidth: 99_999 }).maxWidth, 4080)
assert.notEqual(defaults.playedColors, style.DEFAULT_DESKTOP_LYRICS_STYLE.playedColors, '不共享默认值的数组，改了也不污染默认')
const restored = style.normalizeDesktopLyricsStyle({ strokeWidth: Number.NaN, shadowBlur: 'invalid' })
assert.deepEqual([restored.strokeWidth, restored.shadowBlur], [0.5, 1], '缺失或无效的描边、阴影回到轻量默认值')
const customEffects = style.normalizeDesktopLyricsStyle({ strokeWidth: 2.5, shadowBlur: 12 })
assert.deepEqual([customEffects.strokeWidth, customEffects.shadowBlur], [2.5, 12], '已保存的自定义描边、阴影保持不变')

const messy = style.normalizeDesktopLyricsStyle({
  theme: 'rainbow', layout: 'quad', align: 'justify', secondary: 'both', karaoke: 'yes', background: 'blur',
  fontSize: 999, fontWeight: 650, letterSpacing: 0.26, secondaryScale: 0.02, strokeWidth: 3.3, shadowBlur: -4,
  opacity: 0.12, backgroundOpacity: 7, playedColors: ['#FFF', 'red'], unplayedColors: '#ffffff',
  secondaryColor: '#ABCDEF', strokeEnabled: 'yes', locked: 'true', showTrackInfo: 0,
})
assert.deepEqual(
  [messy.theme, messy.layout, messy.align, messy.secondary, messy.karaoke, messy.background],
  ['aurora', 'single', 'center', 'translation', 'word', 'hover'],
  '未知取值回到默认',
)
assert.equal(messy.fontSize, 96, '字号夹到上限')
assert.equal(messy.fontWeight, 700, '只收列出的字重')
assert.equal(messy.letterSpacing, 0.5, '按 0.5 px 步进')
assert.equal(messy.secondaryScale, 0.4)
assert.equal(messy.strokeWidth, 3.5)
assert.equal(messy.shadowBlur, 0)
assert.equal(messy.opacity, 0.3, '歌词不透明度最低 30%，免得整个看不见')
assert.equal(messy.backgroundOpacity, 1)
assert.deepEqual(messy.playedColors, defaults.playedColors, '只收 #rrggbb')
assert.deepEqual(messy.unplayedColors, defaults.unplayedColors)
assert.equal(messy.secondaryColor, '#abcdef')
assert.equal(messy.strokeEnabled, true, '非布尔值用默认')
assert.equal(messy.locked, false, '只有 true 才锁定')
assert.equal(messy.showTrackInfo, true)

assert.equal(style.sanitizeFontFamily('Noto Sans SC'), 'Noto Sans SC')
assert.equal(style.sanitizeFontFamily('微软雅黑'), '微软雅黑')
assert.equal(style.sanitizeFontFamily('x"; background: url(evil); "'), 'x background urlevil', '拼进 CSS 前去掉引号、分号、括号')
assert.equal(style.sanitizeFontFamily('a'.repeat(200)).length, 64)

assert.deepEqual(style.normalizeDesktopLyricsStyle({ bounds: { x: 10.6, y: -20, width: 100, height: 99_999 } }).bounds, { x: 11, y: -20, width: 360, height: 1200 })
assert.equal(style.normalizeDesktopLyricsStyle({ bounds: { x: 1, y: 2, width: 'w', height: 3 } }).bounds, null)
assert.equal(style.normalizeDesktopLyricsStyle({ bounds: { x: 1e7, y: 0, width: 900, height: 200 } }).bounds, null, '离谱的坐标不收')

// ---- 配色 ----
const sakura = style.applyDesktopLyricsTheme(defaults, 'sakura')
assert.equal(sakura.theme, 'sakura')
assert.deepEqual(sakura.playedColors, style.DESKTOP_LYRICS_THEMES.find(theme => theme.id === 'sakura').played)
assert.equal(style.applyDesktopLyricsTheme(defaults, 'missing'), defaults, '未知预设不改')
assert.equal(style.applyDesktopLyricsTheme(defaults, 'accent').theme, 'accent')
assert.equal(style.parseCssColor('rgb(208, 188, 255)'), '#d0bcff')
assert.equal(style.parseCssColor(' #D0BCFF '), '#d0bcff')
assert.equal(style.parseCssColor('var(--x)'), null)
const accentColors = style.resolveDesktopLyricsColors({ ...defaults, theme: 'accent' }, '#6750a4')
assert.equal(accentColors.played[1], '#6750a4', '跟随主题色：已唱渐变落到主题色')
assert.equal(accentColors.unplayed[0], '#ffffff')
assert.deepEqual(style.resolveDesktopLyricsColors({ ...defaults, theme: 'accent' }, null).played, defaults.playedColors, '读不到主题色时用自己的颜色')

const vars = style.desktopLyricsCssVars({ ...defaults, fontFamily: 'Noto Sans SC', fontSize: 40, secondaryScale: 0.5, strokeEnabled: false, shadowEnabled: false }, null)
assert.match(vars['--dl-font-family'], /^"Noto Sans SC", /)
assert.equal(vars['--dl-font-size'], '40px')
assert.equal(vars['--dl-max-width'], '860px')
assert.equal(vars['--dl-secondary-size'], '20px')
assert.equal(vars['--dl-stroke'], '0 transparent')
assert.equal(vars['--dl-shadow'], 'none')
assert.match(vars['--dl-played'], /^linear-gradient\(180deg, #/)
assert.match(style.desktopLyricsCssVars(defaults, null)['--dl-stroke'], /^1px #/, '默认描边只露出外半边，实际可见宽度为 0.5px')
assert.equal(style.desktopLyricsCssVars(defaults, null)['--dl-shadow'], 'drop-shadow(0 1px 1px #000000d9)', '默认阴影偏移和模糊范围均为 1px')
assert.equal(style.desktopLyricsCssVars({ ...defaults, fontFamily: '' }, null)['--dl-font-family'].startsWith('var('), true)

const effectCases = [
  [{ shadowEnabled: false, glowEnabled: false, strokeEnabled: false }, '0px'],
  [{ shadowEnabled: false, glowEnabled: false, strokeWidth: 4 }, '4px'],
  [{ shadowBlur: 24, strokeWidth: 4, glowEnabled: true, fontSize: 96 }, '82px'],
  [{ shadowEnabled: false, strokeEnabled: false, glowEnabled: true, fontSize: 96 }, '72px'],
]
async function karaokeRuntime(line, boxWidth, padding, align = 'center', options = {}) {
  const source = await readFile(new URL('../src/components/desktopLyrics/KaraokeLine.vue', import.meta.url), 'utf8')
  const script = parse(source).descriptor.scriptSetup.content
  const parsed = ts.createSourceFile('KaraokeLine.ts', script, ts.ScriptTarget.ES2022, true)
  const statements = parsed.statements.filter(statement => !ts.isImportDeclaration(statement)).map(statement => statement.getText(parsed)).join('\n')
  const compiled = ts.transpileModule(statements, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  const props = vue.reactive({ line, endMs: 2_000, karaoke: 'line', align, ...options })
  const dependencies = {
    computed: vue.computed, nextTick: vue.nextTick, ref: vue.ref, watch: vue.watch,
    defineProps: () => props,
    inject: () => null,
    onMounted() {},
    onBeforeUnmount() {},
    KARAOKE_REGISTRY: Symbol(), KARAOKE_WAKE: Symbol(),
    hasWordTiming: timeline.hasWordTiming, highlightWidth: timeline.highlightWidth,
    longLineScrollOffset: timeline.longLineScrollOffset,
    getComputedStyle: () => ({ paddingLeft: `${padding}px`, paddingRight: `${padding}px` }),
  }
  const scope = vue.effectScope()
  const actual = scope.run(() => new Function(...Object.keys(dependencies), `${compiled}\nreturn { container, fit, base, outline, fill, measure, update }`)(...Object.values(dependencies)))
  const classes = new Set()
  actual.container.value = { clientWidth: boxWidth, classList: { toggle(name, enabled) { enabled ? classes.add(name) : classes.delete(name) } } }
  actual.fit.value = { style: {} }
  actual.base.value = { offsetWidth: 600, children: [], style: {} }
  actual.outline.value = { style: {} }
  actual.fill.value = { style: {} }
  return { ...actual, props, classes, dispose: () => scope.stop() }
}

const karaoke = await karaokeRuntime({ startMs: 1_000, durationMs: 1_000, text: 'long line', words: [] }, 500, 82)
try {
  karaoke.measure()
  karaoke.update(1_000)
  assert.equal(karaoke.fit.value.style.transform, '', '长行保留设置字号，不随文字长度缩放')
  assert.equal(karaoke.base.value.style.clipPath, 'inset(-100% 264px -100% 0px)', '长句从开头显示，并为外层效果保留空间')
  karaoke.update(1_500)
  assert.equal(karaoke.fill.value.style.clipPath, 'inset(0 300px 0 0)', '效果留白不改变歌词原始坐标中的高亮进度')
  karaoke.base.value.offsetWidth = 1_000
  karaoke.measure()
  karaoke.update(1_000)
  karaoke.update(6_020)
  assert.equal(karaoke.fit.value.style.transform, 'translateX(-332px)', '长句在起步停顿后平滑移动到中点')
  assert.equal(karaoke.fill.value.style.clipPath, 'inset(0 332px 0 332px)', '唱完后滚动仍继续，已唱层可见区跟随视窗')
  assert.equal(karaoke.base.value.style.clipPath, 'inset(-100% 332px -100% 332px)', '未唱字形不能越过滚动视窗')
  assert.equal(karaoke.outline.value.style.clipPath, 'inset(-100% 332px -100% 332px)', '描边与字形使用相同视窗，阴影留在外层')
  karaoke.measure()
  assert.equal(karaoke.fit.value.style.transform, 'translateX(-332px)', '尺寸未变化的重测不能让歌词闪回开头')
  assert.equal(karaoke.fill.value.style.clipPath, 'inset(0 332px 0 332px)', '重测保留已唱画面，避免一帧闪烁')
  karaoke.update(6_020)
  assert.equal(karaoke.fit.value.style.transform, 'translateX(-332px)', '暂停锚点不变时滚动也停住')
  karaoke.update(10_520)
  assert.equal(karaoke.fit.value.style.transform, 'translateX(-664px)', '滚动到末尾时完整显示最后一段')
  karaoke.update(12_770)
  assert.equal(karaoke.fit.value.style.transform, 'translateX(-560.5px)', '到末尾后缓动返回，不循环跳回起点')
  karaoke.update(1_000)
  assert.equal(karaoke.fit.value.style.transform, '', '向前拖回歌词开头时重置滚动')
  karaoke.base.value.offsetWidth = 100
  karaoke.measure()
  karaoke.update(2_000)
  assert.equal(karaoke.fit.value.style.transform, '', '短句恢复原对齐并始终保留字号')
  assert.equal(karaoke.classes.has('overflowing'), false)
} finally {
  karaoke.dispose()
}

for (const align of ['left', 'center', 'right']) {
  const row = await karaokeRuntime({ startMs: 1_000, durationMs: 1_000, text: 'long line', words: [] }, 500, 82, align)
  try {
    row.base.value.offsetWidth = 1_000
    row.measure()
    row.update(1_000)
    assert.equal(row.fit.value.style.transform, '', '所有对齐方式的长句均从开头开始，保持字号')
    assert.equal(row.base.value.style.clipPath, 'inset(-100% 664px -100% 0px)')
    row.update(6_020)
    assert.equal(row.fill.value.style.clipPath, 'inset(0 332px 0 332px)', '左右对齐的长句也能滚动显示全文')
    row.update(6_019)
    assert.equal(row.fit.value.style.transform, 'translateX(-332px)', '播放时钟小幅重锚不能让滚动跳回起点')
    row.props.line = { ...row.props.line }
    await vue.nextTick()
    row.update(6_020)
    assert.equal(row.fit.value.style.transform, 'translateX(-332px)', '同句的播放帧刷新不能重启滚动')
    row.props.upcoming = true
    await vue.nextTick()
    row.update(8_000)
    assert.equal(row.fit.value.style.transform, '', '下一句预告固定显示开头，不提前滚动')
    row.props.upcoming = false
    await vue.nextTick()
    row.update(8_000)
    assert.equal(row.fit.value.style.transform, '', '双行槽位唱到下一句时从头开始滚动')
  } finally {
    row.dispose()
  }
}

const timedRow = await karaokeRuntime({ startMs: 1_000, durationMs: 1_000, text: 'AB', words: [
  { startMs: 1_000, durationMs: 500, text: 'A' }, { startMs: 1_500, durationMs: 500, text: 'B' },
] }, 500, 82, 'center', { karaoke: 'word' })
try {
  timedRow.base.value.children = [{ offsetLeft: 0, offsetWidth: 400 }, { offsetLeft: 400, offsetWidth: 200 }]
  timedRow.measure()
  timedRow.update(1_000)
  timedRow.update(1_250)
  assert.equal(timedRow.fill.value.style.clipPath, 'inset(0 400px 0 0)', '固定字号后仍按真实字宽推进逐字高亮')
} finally {
  timedRow.dispose()
}

const stageSource = await readFile(new URL('../src/components/desktopLyrics/DesktopLyricsStage.vue', import.meta.url), 'utf8')
const stageScript = parse(stageSource).descriptor.scriptSetup.content
const stageParsed = ts.createSourceFile('DesktopLyricsStage.ts', stageScript, ts.ScriptTarget.ES2022, true)
const stageStatements = stageParsed.statements.filter(statement => !ts.isImportDeclaration(statement)).map(statement => statement.getText(stageParsed)).join('\n')
const stageCompiled = ts.transpileModule(stageStatements, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText
const stageDependencies = {
  computed: vue.computed, ref: vue.ref, watch: vue.watch, nextTick: vue.nextTick,
  defineProps: () => ({ lines: [], firstIndex: 0, style: defaults, clock: () => 0 }),
  provide() {}, onMounted() {}, onBeforeUnmount() {},
  KARAOKE_REGISTRY: Symbol(), KARAOKE_WAKE: Symbol(),
  desktopLyricsCssVars: style.desktopLyricsCssVars, activeLineIndex: timeline.activeLineIndex, lineEndMs: timeline.lineEndMs,
}
const stageScope = vue.effectScope()
try {
  const stage = stageScope.run(() => new Function(...Object.keys(stageDependencies), `${stageCompiled}\nreturn { container, content, fitHeight }`)(...Object.values(stageDependencies)))
  stage.container.value = { clientHeight: 60 }
  stage.content.value = { offsetHeight: 300, style: {} }
  stage.fitHeight()
  assert.equal(stage.content.value.style.transform, 'scale(0.2)', '矮窗口将歌词和效果留白一起缩小到可用高度')
  stage.container.value.clientHeight = 320
  stage.fitHeight()
  assert.equal(stage.content.value.style.transform, '', '窗口有足够高度时恢复原字号')
} finally {
  stageScope.stop()
}
for (const [effects, inset] of effectCases) {
  assert.equal(style.desktopLyricsCssVars({ ...defaults, ...effects }, null)['--dl-effect-inset'], inset, '行边缘为描边、阴影和发光预留可见范围')
}

assert.equal(style.nextDesktopLyricsLayout('single'), 'double')
assert.equal(style.nextDesktopLyricsLayout('triple'), 'single')
assert.equal(style.stepDesktopLyricsFontSize(defaults, 1).fontSize, 38)
assert.equal(style.stepDesktopLyricsFontSize({ ...defaults, fontSize: 96 }, 1).fontSize, 96)
assert.equal(style.stepDesktopLyricsFontSize({ ...defaults, fontSize: 16 }, -1).fontSize, 16)

// ---- 时间轴 ----
const anchor = { positionMs: 10_000, anchorAt: 1_000, rate: 1.5, isPlaying: true }
assert.equal(timeline.anchoredPositionMs(anchor, 3_000), 13_000, '按倍速推进')
assert.equal(timeline.anchoredPositionMs({ ...anchor, isPlaying: false }, 3_000), 10_000, '暂停时停在锚点')
assert.equal(timeline.anchoredPositionMs({ ...anchor, clockRunning: false }, 3_000), 10_000, '播放中等待 seek 确认时也停在锚点')
assert.equal(timeline.anchoredPositionMs({ ...anchor, clockRunning: true }, 3_000), 13_000, 'seek 确认后按倍速恢复时钟')
assert.equal(timeline.anchoredPositionMs(anchor, 500), 10_000, '时钟回拨时不倒退')
assert.equal(timeline.anchoredPositionMs({ ...anchor, rate: Number.NaN }, 2_000), 11_000, '倍速无效按 1')

const lines = [
  { startMs: 1_000, durationMs: 0, text: 'AB', words: [{ startMs: 1_000, durationMs: 500, text: 'A' }, { startMs: 1_500, durationMs: 500, text: 'B' }] },
  { startMs: 3_000, durationMs: 1_000, text: 'plain', words: [] },
  { startMs: 6_000, durationMs: 0, text: 'tail', words: [] },
]
assert.equal(timeline.activeLineIndex(lines, 999), -1)
assert.equal(timeline.activeLineIndex(lines, 1_000), 0)
assert.equal(timeline.activeLineIndex(lines, 5_999), 1)
assert.equal(timeline.lineEndMs(lines, 0), 2_000, '逐字行按最后一个字结尾')
assert.equal(timeline.lineEndMs(lines, 1), 4_000, '按行时长')
assert.equal(timeline.lineEndMs(lines, 2), 10_000, '最后一行没有时长时按 4 秒')
assert.equal(timeline.lineEndMs([{ startMs: 0, durationMs: 0, text: 'x', words: [] }, { startMs: 2_500, durationMs: 0, text: 'y', words: [] }], 0), 2_500, '按下一行开头')

const offsets = [[0, 40], [40, 60]]
assert.equal(timeline.highlightWidth(lines[0], 2_000, 900, 100, offsets, true), 0, '还没开始')
assert.equal(timeline.highlightWidth(lines[0], 2_000, 1_250, 100, offsets, true), 20, '第一个字唱到一半')
assert.equal(timeline.highlightWidth(lines[0], 2_000, 1_750, 100, offsets, true), 70, '第二个字唱到一半')
assert.equal(timeline.highlightWidth(lines[0], 2_000, 2_500, 100, offsets, true), 100)
assert.equal(timeline.highlightWidth(lines[0], 2_000, 1_250, 100, offsets, false), 25, '整行扫过按时间比例')
assert.equal(timeline.highlightWidth(lines[1], 4_000, 3_500, 200, null, true), 100, '没有逐字时间轴时整行扫过')
assert.equal(timeline.highlightWidth(lines[0], 2_000, 1_250, 100, [[0, 40]], true), 25, '字的位置没量齐时退回整行扫过')

for (const [overflow, elapsed, expected] of [
  [0, 5_000, 0], [2, 5_000, 0], [200, -10, 0], [200, 520, 0],
  [342, 2_770, 53.4375], [342, 5_020, 171], [342, 9_520, 342],
  [342, 11_770, 288.5625], [342, 18_520, 0],
  [38, 1_620, 19], [3800, 5_020, 1900], [Number.NaN, 6_000, 0], [100, Number.NaN, 0],
]) {
  assert.equal(timeline.longLineScrollOffset(overflow, elapsed), expected, '长句平移使用缓动往返，短溢出、暂停前和无效值不滚动')
}

console.log('desktop lyrics style and timeline tests passed')
