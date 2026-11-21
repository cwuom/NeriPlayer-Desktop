// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import {
  dialogSelectionPath,
  onExpandAfterEnter,
  onExpandAfterLeave,
  onExpandEnter,
  onExpandLeave,
} from '@/components/settings/settingsShared'

function element(scrollHeight: number): HTMLElement {
  const el = document.createElement('div')
  Object.defineProperty(el, 'scrollHeight', { value: scrollHeight })
  return el
}

describe('dialogSelectionPath', () => {
  it('returns string selections unchanged', () => {
    expect(dialogSelectionPath('/music/downloads')).toBe('/music/downloads')
  })

  it('prefers the path field of legacy object selections', () => {
    expect(dialogSelectionPath({ path: '/images/bg.png' })).toBe('/images/bg.png')
  })

  it('falls back to string conversion when no usable path is present', () => {
    expect(dialogSelectionPath(['/a', '/b'])).toBe('/a,/b')
    expect(dialogSelectionPath({ path: '' })).toBe('[object Object]')
    expect(dialogSelectionPath({ path: 42 })).toBe('[object Object]')
  })
})

describe('expand transition hooks', () => {
  it('animates height from 0 to the content height on enter and clears it afterwards', () => {
    const el = element(120)
    onExpandEnter(el)
    expect(el.style.overflow).toBe('hidden')
    expect(el.style.height).toBe('120px')
    expect(el.style.opacity).toBe('1')
    expect(el.style.transition).toContain('height 300ms')

    onExpandAfterEnter(el)
    expect(el.style.height).toBe('')
    expect(el.style.overflow).toBe('')
    expect(el.style.transition).toBe('')
    expect(el.style.opacity).toBe('1')
  })

  it('collapses to 0 and fades out on leave, then clears every inline style', () => {
    const el = element(80)
    onExpandLeave(el)
    expect(el.style.overflow).toBe('hidden')
    expect(el.style.height).toBe('0px')
    expect(el.style.opacity).toBe('0')
    expect(el.style.transition).toContain('height 250ms')

    onExpandAfterLeave(el)
    expect(el.style.height).toBe('')
    expect(el.style.overflow).toBe('')
    expect(el.style.transition).toBe('')
    expect(el.style.opacity).toBe('')
  })

  it('declares a single parameter so Vue treats the transition as CSS-driven', () => {
    // Vue 依据 hook 形参个数判断是否需要手动 done()，多一个参数会让过渡永远不结束
    for (const hook of [onExpandEnter, onExpandAfterEnter, onExpandLeave, onExpandAfterLeave]) {
      expect(hook.length).toBe(1)
    }
  })
})
