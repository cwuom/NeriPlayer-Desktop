// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { installZoomGuard } from '@/integrations/zoomGuard'

let release: (() => void) | null = null

afterEach(() => {
  release?.()
  release = null
})

function wheel(init: WheelEventInit): WheelEvent {
  const event = new WheelEvent('wheel', { cancelable: true, bubbles: true, ...init })
  // happy-dom 的 WheelEvent 不读取修饰键初始值，手动补上
  for (const key of ['ctrlKey', 'metaKey'] as const) {
    Object.defineProperty(event, key, { value: Boolean(init[key]) })
  }
  document.body.dispatchEvent(event)
  return event
}

describe('installZoomGuard', () => {
  it('cancels pinch / ctrl+wheel page zoom', () => {
    release = installZoomGuard()
    expect(wheel({ ctrlKey: true, deltaY: -40 }).defaultPrevented).toBe(true)
    expect(wheel({ metaKey: true, deltaY: 40 }).defaultPrevented).toBe(true)
  })

  it('keeps ordinary scrolling untouched', () => {
    release = installZoomGuard()
    expect(wheel({ deltaY: 120 }).defaultPrevented).toBe(false)
  })

  it('cancels WebKit gesture zoom events', () => {
    release = installZoomGuard()
    const event = new Event('gesturestart', { cancelable: true })
    window.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
  })

  it('stops guarding after release', () => {
    installZoomGuard()()
    expect(wheel({ ctrlKey: true, deltaY: -40 }).defaultPrevented).toBe(false)
  })
})
