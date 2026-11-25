// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { historyDirectionFromKey, installHistoryNavigation } from '@/integrations/historyNavigation'

let release: (() => void) | null = null
afterEach(() => { release?.(); release = null; document.body.innerHTML = '' })

function key(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })
}

describe('historyDirectionFromKey', () => {
  it('maps Alt+arrows on every platform', () => {
    expect(historyDirectionFromKey(key({ key: 'ArrowLeft', altKey: true }), false)).toBe(-1)
    expect(historyDirectionFromKey(key({ key: 'ArrowRight', altKey: true }), false)).toBe(1)
  })

  it('maps Cmd+[ and Cmd+] only on macOS', () => {
    expect(historyDirectionFromKey(key({ key: '[', metaKey: true }), true)).toBe(-1)
    expect(historyDirectionFromKey(key({ key: ']', metaKey: true }), true)).toBe(1)
    expect(historyDirectionFromKey(key({ key: '[', metaKey: true }), false)).toBe(0)
  })

  it('ignores other modifier combinations and repeats', () => {
    expect(historyDirectionFromKey(key({ key: 'ArrowLeft', altKey: true, shiftKey: true }), false)).toBe(0)
    expect(historyDirectionFromKey(key({ key: 'ArrowLeft' }), false)).toBe(0)
    expect(historyDirectionFromKey(key({ key: 'ArrowLeft', altKey: true, repeat: true }), false)).toBe(0)
  })
})

describe('installHistoryNavigation', () => {
  it('navigates with mouse side buttons', () => {
    const actions = { back: vi.fn(), forward: vi.fn() }
    release = installHistoryNavigation(actions)
    window.dispatchEvent(new MouseEvent('mouseup', { button: 3 }))
    window.dispatchEvent(new MouseEvent('mouseup', { button: 4 }))
    window.dispatchEvent(new MouseEvent('mouseup', { button: 0 }))
    expect(actions.back).toHaveBeenCalledTimes(1)
    expect(actions.forward).toHaveBeenCalledTimes(1)
  })

  it('leaves Alt+arrow alone while typing', () => {
    const actions = { back: vi.fn(), forward: vi.fn() }
    release = installHistoryNavigation(actions)
    document.body.innerHTML = '<input id="field" />'
    document.getElementById('field')!.dispatchEvent(key({ key: 'ArrowLeft', altKey: true }))
    expect(actions.back).not.toHaveBeenCalled()
    document.body.dispatchEvent(key({ key: 'ArrowLeft', altKey: true }))
    expect(actions.back).toHaveBeenCalledTimes(1)
  })
})
