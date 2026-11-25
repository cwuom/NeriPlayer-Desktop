// 回归：页面吸顶栏（返回、搜索）落在窗口顶栏这一行，顶栏太矮时控件几乎贴着窗口上沿。
// 控件高 36px，macOS 上下至少各留 16px（与居中的红绿灯对齐且不拥挤），其他平台至少 10px
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), 'utf8')
const globalScss = await read('src/styles/global.scss')
const detailScss = await read('src/styles/detail-view.scss')

function block(source, selector) {
  const start = source.indexOf(`${selector} {`)
  assert.notEqual(start, -1, `${selector} missing`)
  return source.slice(start, source.indexOf('}', start))
}

function pxOf(body, property) {
  const match = new RegExp(`${property}:\\s*(?:max\\()?(\\d+)px`).exec(body)
  assert.ok(match, `${property} px value missing`)
  return Number(match[1])
}

const defaultHeight = pxOf(block(globalScss, ':root'), '--titlebar-height')
const macHeight = pxOf(block(globalScss, 'html.platform-mac'), '--titlebar-height')
const controlHeight = Math.max(pxOf(block(detailScss, '.back-btn'), 'height'), pxOf(block(detailScss, '.header-search'), 'height'))

assert.ok((defaultHeight - controlHeight) / 2 >= 10, `Windows/Linux titlebar ${defaultHeight}px leaves too little room around ${controlHeight}px controls`)
assert.ok((macHeight - controlHeight) / 2 >= 16, `macOS titlebar ${macHeight}px leaves too little room around ${controlHeight}px controls`)
// 缩放补偿也要跟着同一个基准，否则缩小界面时又会挤回去
assert.match(block(globalScss, 'html.platform-mac'), new RegExp(`max\\(${macHeight}px, calc\\(${macHeight}px / var\\(--app-ui-scale`))

console.log('titlebar spacing: ok')
