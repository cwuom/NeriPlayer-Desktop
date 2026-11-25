import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { compile } from 'sass'

const postcss = createRequire(import.meta.resolve('vite'))('postcss')
const css = postcss.parse(compile(fileURLToPath(new URL('../src/styles/global.scss', import.meta.url))).css)

function transitionStyle(state, rootClasses) {
  const selectors = new Set([
    `.${state}`,
    ...rootClasses.map(name => `html.${name} .${state}`),
  ])
  const style = {}
  css.each(rule => {
    if (rule.type !== 'rule' || !rule.selectors.some(selector => selectors.has(selector))) return
    rule.walkDecls(declaration => { style[declaration.prop] = declaration.value })
  })
  return style
}

const BAR = ':is(.home-topbar, .detail-header)'
for (const platform of ['windows', 'mac', 'linux']) {
  const root = [`platform-${platform}`]
  const entering = transitionStyle('fade-enter-from', root)
  const leaving = transitionStyle('fade-leave-to', root)
  // 整页只淡入淡出：吸顶栏占着窗口顶栏这一行，整页位移/缩放会让它和红绿灯错开再弹回
  assert.equal(entering.opacity, '0')
  assert.equal(entering.transform, undefined, `${platform} does not move the page root that holds the top bar`)
  assert.equal(leaving.opacity, '0')
  assert.equal(leaving.transform, undefined, `${platform} does not scale the leaving page`)
  // 正文仍有上滑；没有吸顶栏的页面整页上滑
  assert.equal(transitionStyle(`fade-enter-from ${BAR} ~ *`, root).transform, 'translateY(8px)', `${platform} slides the content below the top bar`)
  assert.equal(transitionStyle(`fade-enter-from:not(:has(.home-topbar, .detail-header))`, root).transform, 'translateY(8px)', `${platform} slides pages without a top bar`)
  for (const state of ['fade-enter-active', `fade-enter-active ${BAR} ~ *`]) {
    const { transition } = transitionStyle(state, root)
    assert.match(transition, /(?:^|,\s*)opacity /)
    assert.match(transition, /(?:^|,\s*)transform /)
  }
  assert.match(transitionStyle('fade-leave-active', root).transition, /(?:^|,\s*)opacity /)
}

const nvidia = ['platform-linux', 'nvidia-compat']
for (const state of ['fade-enter-from', 'fade-leave-to']) {
  const style = transitionStyle(state, nvidia)
  assert.equal(style.opacity, '1', 'NVIDIA compatibility keeps pages visible during an instant switch')
  assert.equal(style.transform, 'none', 'NVIDIA compatibility avoids the full-page compositor transform')
}
assert.equal(transitionStyle(`fade-enter-from ${BAR} ~ *`, nvidia).transform, 'none', 'NVIDIA compatibility does not slide the content either')
for (const state of ['fade-enter-active', 'fade-leave-active', 'slide-up-enter-active', 'slide-up-leave-active', 'np-background-enter-active']) {
  assert.equal(transitionStyle(state, nvidia).transition, 'none', 'NVIDIA compatibility keeps the no-animation fallback')
}
assert.equal(transitionStyle('np-background-enter-from', nvidia).opacity, '0.8', 'NVIDIA compatibility shows the background immediately at its normal opacity')

console.log('page transition styles passed: top bar stays put, content slides, NVIDIA compatibility fallback')
