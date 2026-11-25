// 回归：「全部播放」与随机播放、多选图标按钮同处一个 align-items: center 的操作行，
// 按钮自带的 margin-top 会把它整体压低半个外边距，最近播放页因此上下错位
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), 'utf8')

function topLevelRule(source, selector) {
  const match = new RegExp(`^${selector.replace('.', '\\.')} \\{`, 'm').exec(source)
  assert.ok(match, `${selector} rule missing`)
  let depth = 0
  for (let i = source.indexOf('{', match.index); i < source.length; i++) {
    if (source[i] === '{') depth++
    if (source[i] === '}' && --depth === 0) return source.slice(match.index, i)
  }
  throw new Error(`${selector} rule not closed`)
}

const detail = await read('src/styles/detail-view.scss')
const ownDeclarations = topLevelRule(detail, '.play-all-btn').replace(/\n {2}[^\n]*\{[^}]*\}/g, '')
assert.doesNotMatch(ownDeclarations, /^\s*margin(-top|-bottom)?:/m, '.play-all-btn must not carry vertical margin; its action row owns the spacing')

const recent = await read('src/views/RecentView.vue')
assert.match(recent, /<div class="recent-actions">\s*<button class="play-all-btn"/, 'recent page keeps play-all inside its action row')
assert.match(topLevelRule(recent.slice(recent.indexOf('<style')), '.recent-actions'), /align-items:\s*center/, 'recent action row centres its buttons')

console.log('play-all alignment: ok')
