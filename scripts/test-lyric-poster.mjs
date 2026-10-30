// 歌词海报排版与配色的回归：动态高度、CJK 换行禁则、降级丢行、底栏/品牌几何、对比度补暗判定
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

async function loadTsModule(relativePath) {
  const source = await readFile(new URL(relativePath, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)
}

const {
  planPoster, wrapText, tokenize, truncateToWidth, posterVisualLines, POSTER_LAYOUT, MAX_POSTER_VISUAL_LINES, LYRIC_ASCENT_RATIO,
} = await loadTsModule('../src/modules/lyrics/posterLayout.ts')
const {
  extraScrimFor, contrastRatio, mixRgb, relativeLuminance, toCss, noCoverStops,
  CARD_SCRIM_STOPS, EXTRA_SCRIM_RANGE, NO_COVER_SEED, WHITE, BLACK,
} = await loadTsModule('../src/modules/lyrics/posterTheme.ts')

/** 假量宽：CJK 全角、拉丁按半角估，够用来验证折行与高度 */
const measure = (text, fontSize) => [...text].reduce((width, character) => {
  if (/[\u3000-\u303F\u4E00-\u9FFF\uAC00-\uD7AF\uFF00-\uFFEF]/.test(character)) return width + 1
  if (character === ' ') return width + 0.3
  return width + 0.56
}, 0) * fontSize

let passed = 0
function check(name, run) {
  run()
  passed++
  console.log(`PASS ${name}`)
}

const CONTENT_WIDTH = POSTER_LAYOUT.width - POSTER_LAYOUT.padding * 2
const lyricTexts = plan => plan.lyrics.map(line => line.text)

check('几何常量与安卓 LyricShareSheet 一致', () => {
  assert.equal(POSTER_LAYOUT.padding, 88)
  assert.equal(POSTER_LAYOUT.lyricsTop, 246)
  assert.equal(POSTER_LAYOUT.barHeight, 214)
  assert.equal(POSTER_LAYOUT.barCover, 132)
  assert.equal(POSTER_LAYOUT.barCoverRadius, 28)
  assert.equal(POSTER_LAYOUT.titleFontSize, 46)
  assert.equal(POSTER_LAYOUT.artistFontSize, 38)
  assert.equal(POSTER_LAYOUT.brandIcon, 48)
  assert.equal(POSTER_LAYOUT.brandFontSize, 42)
  assert.equal(POSTER_LAYOUT.minHeight, 1080, '下限=安卓的方形画布，少行时不拉成长条')
  assert.equal(EXTRA_SCRIM_RANGE.max, 0.35, '补暗上限高于此就会明显比安卓暗一档')
})

check('画布高度随勾选行数变化，且始终在上下限之间', () => {
  const short = planPoster({ lines: ['一句话'], title: '夜航船', artist: '陈粒' }, measure)
  const long = planPoster({
    lines: Array.from({ length: 12 }, (_, index) => `第${index + 1}句八字歌词`),
    title: '夜航船',
    artist: '陈粒',
  }, measure)
  assert.equal(short.height, POSTER_LAYOUT.minHeight, '少行时按最小高度出图')
  assert.ok(long.height > short.height, `多行必须更高：${long.height} > ${short.height}`)
  assert.ok(long.height <= POSTER_LAYOUT.maxHeight)
  assert.equal(short.width, POSTER_LAYOUT.width)
})

check('能装下就保持最大字号，装不下才逐级降字号', () => {
  const few = planPoster({ lines: ['一'], title: 'T', artist: 'A' }, measure)
  const sixShort = planPoster({ lines: Array.from({ length: 6 }, (_, index) => `第${index}句`), title: 'T', artist: 'A' }, measure)
  assert.equal(few.lyricFontSize, 112)
  assert.equal(sixShort.lyricFontSize, 112, '6 短句仍装得下，不该降字号')
  assert.ok(sixShort.height > few.height, '6 行内容超过方形下限，高度应随之增长')
  assert.equal(few.height, POSTER_LAYOUT.minHeight, '单行按 1080 方形出图')
  const sixLong = planPoster({
    lines: Array.from({ length: 6 }, (_, index) => `第${index}句歌词写得很长需要占据好几行才能完整放进这张海报里所以还得继续写`),
    title: 'T',
    artist: 'A',
  }, measure)
  assert.ok(sixLong.lyricFontSize < sixShort.lyricFontSize, `${sixLong.lyricFontSize} 应小于 ${sixShort.lyricFontSize}`)
})

check('长到折不完的单句改用最小字号换最少行数', () => {
  const fixture = '这一段特别长的歌词用来验证折行规则是否会把逗号或者句号留在行首，如果禁则处理正确就不应该出现那种难看的情况，后面还会继续补充一些内容确保它折行数量受控'
  const plan = planPoster({ lines: [fixture], title: 'T', artist: 'A' }, measure)
  const atMinFont = wrapText(fixture, CONTENT_WIDTH, 44, 700, measure).length
  assert.equal(plan.lyricFontSize, 44, '连最小字号都超上限时不该再用大字号把句子切碎')
  assert.equal(plan.lyrics.length, atMinFont, '行数应等于最小字号下的行数')
})

check('CJK 长行折行后行首不出现禁则标点', () => {
  const wrapped = wrapText('这段歌词的结尾刚好是一个逗号，然后继续写下去看看断行位置对不对，最后一句', CONTENT_WIDTH, 72, 700, measure)
  assert.ok(wrapped.length > 1, 'fixture 应至少折一次行')
  for (const line of wrapped) {
    assert.ok(![...line][0] || !'，。、！？；：）】」』'.includes([...line][0]), `行首是禁则标点：${line}`)
    assert.ok(!/\s$/.test(line), `行尾留了空格：${line}`)
  }
})

check('拉丁词整体换行，不在词中间断开', () => {
  const wrapped = wrapText('一二三四五六七八九十一二三四五六七八 beautiful 九十', CONTENT_WIDTH, 72, 700, text => text.length * 40)
  assert.ok(wrapped.length > 1, 'fixture 至少要折一行')
  const joined = wrapped.join('|')
  for (const fragment of ['beau', 'auti', 'iful']) {
    assert.ok(!joined.includes(fragment) || joined.includes('beautiful'), `单词被拆断：${joined}`)
  }
  assert.ok(wrapped.some(line => line.includes('beautiful')))
})

check('tokenize 把 CJK 拆成单字、拉丁连成词', () => {
  assert.deepEqual(tokenize('AB 中x'), ['AB', ' ', '中', 'x'])
})

check('行数超出容量时从末尾丢行并回报数量', () => {
  const plan = planPoster({
    lines: Array.from({ length: 40 }, (_, index) => `这是第${index}句歌词，长度适中刚好需要一点空间来摆放它`),
    title: 'T',
    artist: 'A',
  }, measure)
  assert.ok(plan.dropped > 0, '40 行必然装不下，应报告丢行')
  assert.ok(lyricTexts(plan).length > 0, '仍要出图，不能整张空')
  assert.ok(plan.height <= POSTER_LAYOUT.maxHeight)
})

check('版面预算按视觉行算，长句与短句一视同仁', () => {
  const shortLines = Array.from({ length: 24 }, (_, i) => `第${i + 1}句十字`)
  const longLines = Array.from({ length: 12 }, (_, i) => `第${i + 1}句${'字'.repeat(24)}`)
  assert.equal(posterVisualLines(shortLines, measure), MAX_POSTER_VISUAL_LINES)
  assert.equal(posterVisualLines(longLines, measure), posterVisualLines(shortLines, measure),
    '12 句长句折行后的占用应与 24 句短句相同')
})

check('预算内的选择不会丢行', () => {
  const lines = Array.from({ length: MAX_POSTER_VISUAL_LINES }, (_, index) => `第${index + 1}句正常长度的歌词`)
  const plan = planPoster({ lines, title: 'T', artist: 'A' }, measure)
  assert.equal(plan.dropped, 0, `预算 ${MAX_POSTER_VISUAL_LINES} 视觉行内却丢了行`)
  assert.equal(plan.lyrics.length, MAX_POSTER_VISUAL_LINES)
})

check('空行与纯空白行不占位也不上图', () => {
  const plan = planPoster({ lines: ['', '   ', '真实的一句', '\t'], title: 'T', artist: 'A' }, measure)
  assert.deepEqual(lyricTexts(plan), ['真实的一句'])
})

check('底栏歌名/歌手单行截断并回报', () => {
  const plan = planPoster({
    lines: ['一句'],
    title: '这是一首名字长得必须截断的歌曲标题它超出了底栏可用宽度',
    artist: '这位歌手名字同样长得必须截断显示所以再加几个字才够超出可用宽度',
  }, measure)
  assert.ok(plan.bar.title.text.endsWith('…'), plan.bar.title.text)
  assert.ok(plan.bar.artist.text.endsWith('…'), plan.bar.artist.text)
  assert.equal(plan.titleTruncated, true)
  assert.equal(plan.artistTruncated, true)
})

check('truncateToWidth 在限内原样返回', () => {
  assert.deepEqual(truncateToWidth('短标题', 500, 46, 700, measure), { text: '短标题', truncated: false })
})

check('底栏通栏、文字基线按安卓偏移，歌词不压进底栏', () => {
  const plan = planPoster({ lines: ['第一句', '第二句是很长的一段需要折行才能放得下所以继续写下去直到断开为止'], title: 'T', artist: 'A' }, measure)
  assert.equal(plan.bar.rect.x, 0, '底栏左右不留内缩，与安卓一致')
  assert.equal(plan.bar.rect.size, POSTER_LAYOUT.barHeight)
  assert.equal(plan.bar.rect.y, plan.height - POSTER_LAYOUT.barHeight)
  assert.equal(plan.bar.title.y, plan.bar.rect.y + POSTER_LAYOUT.titleBaselineOffset)
  assert.equal(plan.bar.artist.y, plan.bar.rect.y + POSTER_LAYOUT.artistBaselineOffset)
  assert.equal(plan.bar.title.x, 88 + 132 + 28)
  assert.ok(plan.lyrics.every(line => line.x === POSTER_LAYOUT.padding), '歌词左基线必须一致')
  const last = plan.lyrics[plan.lyrics.length - 1]
  assert.ok(last.y + last.fontSize * 0.3 <= plan.bar.rect.y + 1, '歌词不能压进底栏')
})

check('歌词基线含上行偏移，行距按 1.35 倍字号推进', () => {
  const plan = planPoster({ lines: ['第一句', '第二句'], title: 'T', artist: 'A' }, measure)
  assert.equal(plan.lyrics.length, 2)
  assert.ok(plan.lyrics[0].y > POSTER_LAYOUT.lyricsTop, '首行基线应在 lyricsTop 之下')
  const pitch = plan.lyrics[1].y - plan.lyrics[0].y
  assert.ok(Math.abs(pitch - plan.lyricFontSize * 1.35) < 1, `行距 ${pitch} 应约等于字号 × 1.35`)
  assert.ok(LYRIC_ASCENT_RATIO > 0.7 && LYRIC_ASCENT_RATIO < 0.9)
})

check('品牌位在左上角：图标 48@y84，文字基线 124 排在图标右侧', () => {
  const plan = planPoster({ lines: ['一句'], title: 'T', artist: 'A' }, measure)
  assert.equal(plan.brand.icon.x, POSTER_LAYOUT.padding)
  assert.equal(plan.brand.icon.y, 84)
  assert.equal(plan.brand.icon.size, 48)
  assert.equal(plan.brand.text.y, 124)
  assert.equal(plan.brand.text.x, 88 + 48 + 18)
  assert.ok(plan.brand.icon.y + plan.brand.icon.size < POSTER_LAYOUT.lyricsTop, '品牌位不能压到歌词区')
})

check('无封面时封面位仍在，改画首字母占位', () => {
  const withCover = planPoster({ lines: ['一句'], title: '夜航船', artist: 'A' }, measure)
  const without = planPoster({ lines: ['一句'], title: '夜航船', artist: 'A', hasCover: false }, measure)
  assert.equal(withCover.bar.cover.size, 132)
  assert.deepEqual(without.bar.cover, withCover.bar.cover, '占位块与真封面同一位置同一尺寸')
  assert.equal(withCover.bar.placeholderLetter, '')
  assert.equal(without.bar.placeholderLetter, '夜')
})

check('extraScrimFor：暗底不补、亮底才补，补完必须达标', () => {
  assert.equal(extraScrimFor([8, 8, 12], 7), 0, '本来就很暗不该再补')
  assert.equal(extraScrimFor([255, 255, 255], 7), EXTRA_SCRIM_RANGE.max, '纯白封面补到上限为止，不再压更深')
  for (const background of [[120, 140, 160], [240, 244, 250], [255, 255, 255], [200, 220, 180]]) {
    const alpha = extraScrimFor(background, 7)
    assert.ok(alpha > 0 && alpha <= EXTRA_SCRIM_RANGE.max, `亮底应补暗：${background} → ${alpha}`)
    const composited = mixRgb(background, BLACK, alpha)
    assert.ok(contrastRatio(WHITE, composited) >= 7
      || alpha === EXTRA_SCRIM_RANGE.max, `补完仍不达标：${JSON.stringify(background)}`)
  }
})

check('对角蒙层与无封面渐变沿用安卓的停靠点', () => {
  assert.deepEqual(CARD_SCRIM_STOPS.map(stop => stop.at), [0, 0.48, 1])
  assert.equal(CARD_SCRIM_STOPS[0].color, WHITE)
  assert.equal(Math.round(CARD_SCRIM_STOPS[0].alpha * 255), 86)
  assert.equal(Math.round(CARD_SCRIM_STOPS[1].alpha * 255), 64)
  assert.equal(Math.round(CARD_SCRIM_STOPS[2].alpha * 255), 118)
  assert.deepEqual(noCoverStops(NO_COVER_SEED).map(stop => stop.at), [0, 0.62, 1])
  assert.deepEqual([...noCoverStops(NO_COVER_SEED)[1].color], [...NO_COVER_SEED])
})

check('toCss 输出取整后的 rgb，且带透明度时走 rgba', () => {
  assert.equal(toCss([10.4, 20.6, 30.2]), 'rgb(10, 21, 30)')
  assert.equal(toCss([0, 0, 0], 0.5), 'rgba(0, 0, 0, 0.5)')
  assert.equal(toCss([255, 255, 255]), 'rgb(255, 255, 255)')
})

check('relativeLuminance 黑白端点正确', () => {
  assert.ok(Math.abs(relativeLuminance(WHITE) - 1) < 1e-6)
  assert.equal(relativeLuminance(BLACK), 0)
  assert.ok(Math.abs(contrastRatio(WHITE, BLACK) - 21) < 0.01)
})

console.log(`lyrics poster: ${passed} passed, 0 failed`)
