import { describe, expect, it } from 'vitest'
import { IDENTITY, clampPan, clampScale, wheelZoomFactor, zoomAround } from '@/modules/viewer/zoom'

const size = { width: 400, height: 400 }

describe('viewer zoom math', () => {
  it('clamps scale to the supported range', () => {
    expect(clampScale(0.2)).toBe(1)
    expect(clampScale(99)).toBe(6)
    expect(clampScale(Number.NaN)).toBe(1)
  })

  it('keeps the pixel under the cursor fixed while zooming', () => {
    const anchor = { x: 100, y: -50 }
    const next = zoomAround(IDENTITY, 2, anchor, size)
    // 缩放前锚点下的图片坐标 = (anchor - pan) / scale
    const before = { x: (anchor.x - IDENTITY.x) / IDENTITY.scale, y: (anchor.y - IDENTITY.y) / IDENTITY.scale }
    const after = { x: (anchor.x - next.x) / next.scale, y: (anchor.y - next.y) / next.scale }
    expect(after.x).toBeCloseTo(before.x)
    expect(after.y).toBeCloseTo(before.y)
  })

  it('snaps back to identity when zoomed fully out', () => {
    const zoomed = zoomAround(IDENTITY, 3, { x: 150, y: 150 }, size)
    expect(zoomAround(zoomed, 0.5, { x: 0, y: 0 }, size)).toEqual(IDENTITY)
  })

  it('never lets the image edge move inside the viewport', () => {
    expect(clampPan({ scale: 2, x: 900, y: -900 }, size)).toEqual({ scale: 2, x: 200, y: -200 })
    expect(clampPan({ scale: 1, x: 30, y: 30 }, size)).toEqual({ scale: 1, x: 0, y: 0 })
  })

  it('maps wheel deltas to smooth multiplicative factors', () => {
    expect(wheelZoomFactor(-100)).toBeGreaterThan(1)
    expect(wheelZoomFactor(100)).toBeLessThan(1)
    expect(wheelZoomFactor(-100) * wheelZoomFactor(100)).toBeCloseTo(1)
    expect(wheelZoomFactor(-10000)).toBe(wheelZoomFactor(-200))
  })
})
