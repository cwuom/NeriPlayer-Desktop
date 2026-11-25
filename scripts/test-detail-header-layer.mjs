// 回归：内容区顶部的窗口拖动条带毛玻璃底色，详情页（歌单/专辑/歌手）自成层叠上下文，
// 若整页层级不高于拖动条，吸顶栏里的返回和搜索会被毛玻璃盖住看不见
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), 'utf8')

function ruleBody(source, selector) {
  const start = source.indexOf(`${selector} {`)
  assert.notEqual(start, -1, `${selector} rule missing`)
  let depth = 0
  for (let i = source.indexOf('{', start); i < source.length; i++) {
    if (source[i] === '{') depth++
    if (source[i] === '}' && --depth === 0) return source.slice(start, i)
  }
  throw new Error(`${selector} rule not closed`)
}

function zIndexOf(body, selector) {
  const match = /^\s*z-index:\s*(\d+);/m.exec(body)
  assert.ok(match, `${selector} needs an explicit z-index`)
  return Number(match[1])
}

const app = await read('src/App.vue')
const detail = await read('src/styles/detail-view.scss')

const strip = ruleBody(app, '.content-drag-strip')
const stripLayer = zIndexOf(strip, '.content-drag-strip')
// 拖动条的渐隐玻璃画在 global.scss 的 ::before 上，同样会盖住层级不高于它的页面吸顶栏
const globalStyles = await read('src/styles/global.scss')
assert.match(globalStyles, /#app :is\(\.home-topbar, \.detail-header, \.content-drag-strip, \.content-gutter-glass\)::before \{[^}]*backdrop-filter/, 'drag strip is the frosted layer this test guards against')

const detailView = ruleBody(detail, '.detail-view')
const createsOwnStackingContext = /transform:\s*translateZ\(0\)/.test(detailView)
if (createsOwnStackingContext) {
  assert.match(detailView, /^\s*position:\s*(relative|sticky|absolute)/m, '.detail-view must be positioned for z-index to apply')
  assert.ok(
    zIndexOf(detailView, '.detail-view') > stripLayer,
    '.detail-view must sit above .content-drag-strip, otherwise its sticky header is covered',
  )
}

// 首页吸顶栏同理：切页淡入时页面 opacity < 1 自成层叠上下文，层级不高于拖动条就会整段被盖住，结束时闪现
const home = await read('src/views/HomeView.vue')
const homeView = ruleBody(home, '.home-view')
assert.match(homeView, /^\s*position:\s*relative;/m, '.home-view must be positioned for z-index to apply')
assert.ok(zIndexOf(homeView, '.home-view') > stripLayer, '.home-view must sit above .content-drag-strip while fading in')

// 页面切换时吸顶栏不能随整页位移/缩放（会和红绿灯错开再弹回），位移只给吸顶栏之后的正文
const globalScss = globalStyles
const enterFrom = /\.fade-enter-from \{([^}]*)\}/.exec(globalScss)?.[1] ?? ''
assert.doesNotMatch(enterFrom, /transform|scale/, 'page enter must not transform the page root that holds the top bar')
assert.match(globalScss, /\.fade-enter-from :is\(\.home-topbar, \.detail-header\) ~ \*/, 'content after the top bar still slides in')
assert.doesNotMatch(/\.fade-leave-to \{([^}]*)\}/.exec(globalScss)?.[1] ?? '', /transform|scale/, 'page leave must not scale the top bar')

console.log('detail header layer: ok')
