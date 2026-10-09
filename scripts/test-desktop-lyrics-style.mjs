import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
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
assert.notEqual(defaults.playedColors, style.DEFAULT_DESKTOP_LYRICS_STYLE.playedColors, '不共享默认值的数组，改了也不污染默认')

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
assert.equal(vars['--dl-secondary-size'], '20px')
assert.equal(vars['--dl-stroke'], '0 transparent')
assert.equal(vars['--dl-shadow'], 'none')
assert.match(vars['--dl-played'], /^linear-gradient\(180deg, #/)
assert.match(style.desktopLyricsCssVars(defaults, null)['--dl-stroke'], /^2px #/, '描边画在填充下面，只露出外半边，所以宽度翻倍')
assert.equal(style.desktopLyricsCssVars({ ...defaults, fontFamily: '' }, null)['--dl-font-family'].startsWith('var('), true)

assert.equal(style.nextDesktopLyricsLayout('single'), 'double')
assert.equal(style.nextDesktopLyricsLayout('triple'), 'single')
assert.equal(style.stepDesktopLyricsFontSize(defaults, 1).fontSize, 38)
assert.equal(style.stepDesktopLyricsFontSize({ ...defaults, fontSize: 96 }, 1).fontSize, 96)
assert.equal(style.stepDesktopLyricsFontSize({ ...defaults, fontSize: 16 }, -1).fontSize, 16)

// ---- 时间轴 ----
const anchor = { positionMs: 10_000, anchorAt: 1_000, rate: 1.5, isPlaying: true }
assert.equal(timeline.anchoredPositionMs(anchor, 3_000), 13_000, '按倍速推进')
assert.equal(timeline.anchoredPositionMs({ ...anchor, isPlaying: false }, 3_000), 10_000, '暂停时停在锚点')
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

console.log('desktop lyrics style and timeline tests passed')
