// 回归：侧栏收起后只隐藏了品牌，128px 宽的左槽仍在 z-index 1000 的顶栏里接收点击，
// 透明地盖住详情页的返回按钮。编译 TitleBar 的 scoped 样式，确认收起态下左槽整组都放行点击
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { compileStyle, parse } from 'vue/compiler-sfc'
import { compileString } from 'sass'

const file = new URL('../src/components/TitleBar.vue', import.meta.url)
const { descriptor } = parse(await readFile(file, 'utf8'), { filename: file.pathname })
const style = descriptor.styles.find(item => item.scoped)
const source = style.lang === 'scss' ? compileString(style.content).css : style.content
const css = compileStyle({ source, filename: file.pathname, id: 'data-v-tb', scoped: true }).code.replace(/\/\*[\s\S]*?\*\//g, '')

const collapsedRules = [...css.matchAll(/([^{}]*html\.sidebar-collapsed[^{}]*)\{([^}]*)\}/g)]
  .map(([, selector, body]) => ({ selector: selector.trim(), body }))
const released = collapsedRules.filter(rule => /pointer-events:\s*none/.test(rule.body)).map(rule => rule.selector).join(',')

for (const part of ['.tb-left-slot', '.tb-left-stack', '.tb-brand']) {
  assert.ok(released.includes(part), `collapsed titlebar must release clicks on ${part}`)
}
assert.ok(collapsedRules.every(rule => rule.selector.includes(':not(.tb-np-mode)')), 'now-playing mode keeps its left controls clickable')
console.log('titlebar collapsed hit testing: ok')
