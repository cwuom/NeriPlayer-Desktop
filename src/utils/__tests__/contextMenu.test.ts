import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  clampContextMenuPosition,
  createContextMenuItem,
  createContextMenuSeparator,
  isContextMenuSeparator,
  type ContextMenuItem,
} from '@/utils/contextMenu'

const viewport = { viewportWidth: 800, viewportHeight: 600 }

describe('clampContextMenuPosition', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('keeps a position that already fits inside the viewport', () => {
    expect(clampContextMenuPosition(100, 120, 200, 150, viewport)).toEqual({ x: 100, y: 120 })
  })

  it('pushes a menu that would overflow the right/bottom edge back inside the margin', () => {
    expect(clampContextMenuPosition(700, 550, 200, 150, viewport)).toEqual({ x: 592, y: 442 })
  })

  it('enforces the margin on the top/left edge', () => {
    expect(clampContextMenuPosition(-50, 3, 200, 150, viewport)).toEqual({ x: 8, y: 8 })
  })

  it('pins a menu larger than the viewport to the margin', () => {
    expect(clampContextMenuPosition(300, 300, 1000, 900, viewport)).toEqual({ x: 8, y: 8 })
  })

  it('honours a custom margin and treats a negative margin as zero', () => {
    expect(clampContextMenuPosition(-10, 900, 100, 100, { ...viewport, margin: 20 })).toEqual({ x: 20, y: 480 })
    expect(clampContextMenuPosition(-10, 900, 100, 100, { ...viewport, margin: -5 })).toEqual({ x: 0, y: 500 })
  })

  it('treats negative or non-finite menu sizes as zero', () => {
    expect(clampContextMenuPosition(790, 590, -100, Number.NaN, viewport)).toEqual({ x: 790, y: 590 })
    expect(clampContextMenuPosition(799, 599, -100, Number.NaN, viewport)).toEqual({ x: 792, y: 592 })
  })

  it('places non-finite coordinates at the margin', () => {
    expect(clampContextMenuPosition(Number.NaN, Number.POSITIVE_INFINITY, 100, 100, viewport)).toEqual({ x: 8, y: 8 })
  })

  it('uses the window size when no viewport is given', () => {
    vi.stubGlobal('window', { innerWidth: 400, innerHeight: 300 })
    expect(clampContextMenuPosition(390, 290, 100, 50)).toEqual({ x: 292, y: 242 })
  })

  it('falls back to the window size when the viewport option is not finite', () => {
    vi.stubGlobal('window', { innerWidth: 400, innerHeight: 300 })
    expect(
      clampContextMenuPosition(390, 290, 100, 50, { viewportWidth: Number.NaN, viewportHeight: Number.POSITIVE_INFINITY }),
    ).toEqual({ x: 292, y: 242 })
  })

  it('collapses to the margin when there is no window and no viewport', () => {
    expect(typeof window).toBe('undefined')
    expect(clampContextMenuPosition(250, 250, 10, 10)).toEqual({ x: 8, y: 8 })
  })
})

describe('context menu item helpers', () => {
  it('creates an action item carrying every option', () => {
    expect(
      createContextMenuItem('Delete', { id: 'delete', icon: 'trash', danger: true, disabled: false, shortcut: 'Del' }),
    ).toEqual({ label: 'Delete', id: 'delete', icon: 'trash', danger: true, disabled: false, shortcut: 'Del' })
  })

  it('creates a bare action item when no options are given', () => {
    expect(createContextMenuItem('Play')).toStrictEqual({ label: 'Play' })
  })

  it('does not share or mutate the options object', () => {
    const options = { icon: 'play' }
    const item = createContextMenuItem('Play', options)
    item.icon = 'pause'
    expect(options).toEqual({ icon: 'play' })
  })

  it('creates a separator without an id key unless one is given', () => {
    const anonymous = createContextMenuSeparator()
    expect(anonymous).toStrictEqual({ separator: true })
    expect('id' in anonymous).toBe(false)
    expect(createContextMenuSeparator('sep-1')).toStrictEqual({ id: 'sep-1', separator: true })
  })

  it('distinguishes separators from action items', () => {
    const items: ContextMenuItem[] = [
      createContextMenuSeparator(),
      createContextMenuItem('Play'),
      { label: 'Explicit', separator: false },
    ]
    expect(items.map(isContextMenuSeparator)).toEqual([true, false, false])
  })
})
