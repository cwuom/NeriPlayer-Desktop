// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { installRangeFill, sliderFillPercent } from '@/integrations/rangeFill'

let release: (() => void) | null = null
afterEach(() => { release?.(); release = null; document.body.innerHTML = '' })

const nextFrame = () => new Promise(resolve => requestAnimationFrame(() => resolve(null)))

describe('sliderFillPercent', () => {
  it('maps the value into 0–100 inside the range', () => {
    expect(sliderFillPercent(50, 0, 200)).toBe(25)
    expect(sliderFillPercent(-1, -1, 1)).toBe(0)
    expect(sliderFillPercent(1, -1, 1)).toBe(100)
  })

  it('clamps and guards invalid ranges', () => {
    expect(sliderFillPercent(500, 0, 100)).toBe(100)
    expect(sliderFillPercent(5, 10, 10)).toBe(0)
    expect(sliderFillPercent(Number.NaN, 0, 1)).toBe(0)
  })
})

describe('installRangeFill', () => {
  it('fills mounted sliders and follows dragging', async () => {
    document.body.innerHTML = '<input type="range" class="m3-slider" min="0" max="10" value="5" />'
    release = installRangeFill()
    await nextFrame()
    const input = document.querySelector('input')!
    expect(input.style.getPropertyValue('--slider-fill')).toBe('50%')
    input.value = '8'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    expect(input.style.getPropertyValue('--slider-fill')).toBe('80%')
  })

  it('ignores ranges that are not themed sliders', async () => {
    document.body.innerHTML = '<input type="range" min="0" max="10" value="5" />'
    release = installRangeFill()
    await nextFrame()
    expect(document.querySelector('input')!.style.getPropertyValue('--slider-fill')).toBe('')
  })
})
