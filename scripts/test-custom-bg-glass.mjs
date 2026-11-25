// 回归：自定义背景图模式下的玻璃效果
// 1. 切页不能做整页透明度过渡：祖先 opacity < 1 会让玻璃在过渡期间采不到背景图，结束时才闪现
// 2. 吸顶栏停在顶部时透明，正文滚到下面才淡入玻璃（data-scrolled 由 scrolledState 维护）
// 3. 音量浮层等半透明浮层始终带玻璃；首页、统计页、侧栏高亮纳入进阶模糊
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { compile } from 'sass'

const postcss = createRequire(import.meta.resolve('vite'))('postcss')
const css = postcss.parse(compile(fileURLToPath(new URL('../src/styles/global.scss', import.meta.url))).css)

function rulesMatching(predicate) {
  const rules = []
  css.walkRules(rule => { if (rule.selectors.some(predicate)) rules.push(rule) })
  return rules
}
function declarations(predicate) {
  const style = {}
  for (const rule of rulesMatching(predicate)) rule.walkDecls(decl => { style[decl.prop] = decl.value })
  return style
}
const exact = selector => candidate => candidate.replace(/\s+/g, ' ').trim() === selector

for (const state of ['fade-enter-from', 'fade-leave-to']) {
  assert.equal(declarations(exact(`html.has-custom-bg .${state}`)).opacity, '1', `${state} keeps glass pages opaque`)
}
assert.equal(declarations(exact('html.has-custom-bg .fade-leave-active')).transition, 'none')

// iOS 式渐隐顶栏：停在顶部透明，滚动后淡入；玻璃向下伸出渐隐区并用遮罩淡出，没有硬边
const bars = ':is(.home-topbar, .detail-header, .content-drag-strip)'
const resting = declarations(exact(`#app ${bars}`))
assert.equal(resting.background, 'transparent', 'top bars are transparent at rest')
assert.equal(resting['backdrop-filter'], 'none', 'top bars do not blur at rest')
const glass = declarations(exact('#app :is(.home-topbar, .detail-header, .content-drag-strip, .content-gutter-glass)::before'))
assert.equal(glass.opacity, '0')
assert.match(glass['backdrop-filter'], /blur\(var\(--topbar-blur\)\)/)
assert.match(glass['mask-image'], /linear-gradient\(to bottom, #000 0%, #000 55%, transparent 100%\)/, 'glass fades out downwards')
assert.equal(glass.height, 'calc(100% + var(--topbar-fade))', 'glass extends below the bar to fade out')
for (const scrolled of [
  '#app .content[data-scrolled] > .content-drag-strip::before',
  '#app .content[data-scrolled] .home-topbar::before',
  '#app .detail-view[data-scrolled] > .detail-header::before',
  '#app:has(.content[data-scrolled], .detail-view[data-scrolled]) .content-gutter-glass::before',
]) assert.equal(declarations(exact(scrolled)).opacity, '1', `${scrolled} fades the glass in`)
// 背景图模式用玻璃模糊半径与浅底色；滚动条列也由窗口层补上玻璃，滚动条不伸进顶栏
assert.equal(declarations(exact('html.has-custom-bg #app'))['--topbar-blur'], 'var(--glass-blur, 36px)')
assert.match(declarations(exact('#app :is(.content, .detail-view)::-webkit-scrollbar-track'))['margin-top'], /--titlebar-height/)

assert.match(declarations(exact('html.has-custom-bg #app .mp-volume-popover'))['backdrop-filter'] ?? '', /blur\(/, 'volume popover keeps glass without enhanced blur')

const enhanced = rulesMatching(selector => selector.startsWith('html.has-custom-bg.enhanced-blur #app :is('))
assert.equal(enhanced.length, 1)
for (const target of [
  '.quick-tile', '.nav-item.active', '.nav-item:hover', '.stats-card', '.stats-hero', '.spotlight', '.tracks-panel',
  // 探索页、音乐库、详情页顶栏里的半透明控件也要有玻璃
  '.platform-tabs', '.kind-chip', '.search-bar', '.tab-bar', '.tab-search', '.local-category-chip', '.playlist-item:hover',
  '.back-btn', '.header-search', '.segmented', '.custom-select-trigger',
]) {
  assert.ok(enhanced[0].selector.includes(target), `${target} gets glass with enhanced blur`)
}

console.log('custom background glass: ok')
