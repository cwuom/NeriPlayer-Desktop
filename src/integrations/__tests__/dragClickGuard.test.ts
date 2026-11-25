// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { installDragClickGuard } from '@/integrations/dragClickGuard'

let release: () => void
let overlay: HTMLDivElement
let input: HTMLInputElement
let onOverlayClick: ReturnType<typeof vi.fn>

function pointerDown(target: Element, x: number, y: number) {
  target.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientX: x, clientY: y }))
}

function click(target: Element, x: number, y: number, detail = 1) {
  target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, clientX: x, clientY: y, detail }))
}

beforeEach(() => {
  document.body.innerHTML = '<div id="overlay"><div class="dialog"><input id="field" /></div></div>'
  overlay = document.getElementById('overlay') as HTMLDivElement
  input = document.getElementById('field') as HTMLInputElement
  onOverlayClick = vi.fn()
  overlay.addEventListener('click', onOverlayClick)
  release = installDragClickGuard()
})

afterEach(() => release())

describe('installDragClickGuard', () => {
  it('swallows the click produced by dragging a text selection out onto the backdrop', () => {
    pointerDown(input, 100, 100)
    click(overlay, 300, 120)
    expect(onOverlayClick).not.toHaveBeenCalled()
  })

  it('keeps genuine backdrop clicks', () => {
    pointerDown(overlay, 10, 10)
    click(overlay, 10, 10)
    expect(onOverlayClick).toHaveBeenCalledTimes(1)
  })

  it('keeps clicks that start on a child and barely move (e.g. icon inside a button)', () => {
    pointerDown(input, 100, 100)
    click(overlay, 102, 101)
    expect(onOverlayClick).toHaveBeenCalledTimes(1)
  })

  it('keeps keyboard-activated clicks', () => {
    pointerDown(input, 100, 100)
    click(overlay, 0, 0, 0)
    expect(onOverlayClick).toHaveBeenCalledTimes(1)
  })
})
