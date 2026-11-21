// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { isEditableTarget } from '@/modules/shortcuts/platform'

interface FakeNavigator {
  platform: string
  userAgent: string
  userAgentData?: { platform?: string }
}

const WINDOWS: FakeNavigator = { platform: 'Win32', userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' }
const MAC: FakeNavigator = { platform: 'MacIntel', userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)' }

/** isMacPlatform 在模块加载时求值，切换平台必须重新加载模块 */
async function loadPlatformModule(nav: FakeNavigator) {
  vi.stubGlobal('navigator', nav)
  vi.resetModules()
  return import('@/modules/shortcuts/platform')
}

function key(init: KeyboardEventInit = {}): KeyboardEvent {
  return new KeyboardEvent('keydown', { key: 'a', ...init })
}

afterEach(() => {
  vi.unstubAllGlobals()
  document.body.replaceChildren()
})

describe('primary modifier on Windows and Linux', () => {
  it('uses Ctrl as the primary modifier', async () => {
    const platform = await loadPlatformModule(WINDOWS)

    expect(platform.isMacPlatform).toBe(false)
    expect(platform.primaryModifierLabel).toBe('Ctrl')
    expect(platform.hasPrimaryModifier(key({ ctrlKey: true }))).toBe(true)
    expect(platform.hasPrimaryModifier(key({ metaKey: true }))).toBe(false)
  })

  it('rejects Ctrl combined with Alt or the Windows key', async () => {
    const platform = await loadPlatformModule(WINDOWS)

    expect(platform.hasOnlyPrimaryModifier(key({ ctrlKey: true }))).toBe(true)
    expect(platform.hasOnlyPrimaryModifier(key({ ctrlKey: true, shiftKey: true }))).toBe(true)
    expect(platform.hasOnlyPrimaryModifier(key({ ctrlKey: true, altKey: true }))).toBe(false)
    expect(platform.hasOnlyPrimaryModifier(key({ ctrlKey: true, metaKey: true }))).toBe(false)
    expect(platform.hasOnlyPrimaryModifier(key({ metaKey: true }))).toBe(false)
  })
})

describe('primary modifier on macOS', () => {
  it('uses Command as the primary modifier', async () => {
    const platform = await loadPlatformModule(MAC)

    expect(platform.isMacPlatform).toBe(true)
    expect(platform.primaryModifierLabel).toBe('⌘')
    expect(platform.hasPrimaryModifier(key({ metaKey: true }))).toBe(true)
    expect(platform.hasPrimaryModifier(key({ ctrlKey: true }))).toBe(false)
  })

  it('rejects Command combined with Control or Option', async () => {
    const platform = await loadPlatformModule(MAC)

    expect(platform.hasOnlyPrimaryModifier(key({ metaKey: true }))).toBe(true)
    expect(platform.hasOnlyPrimaryModifier(key({ metaKey: true, ctrlKey: true }))).toBe(false)
    expect(platform.hasOnlyPrimaryModifier(key({ metaKey: true, altKey: true }))).toBe(false)
    expect(platform.hasOnlyPrimaryModifier(key({ ctrlKey: true }))).toBe(false)
  })
})

describe('hasNoModifier', () => {
  it('ignores Shift but reports any of Meta, Ctrl or Alt', async () => {
    const platform = await loadPlatformModule(WINDOWS)

    expect(platform.hasNoModifier(key())).toBe(true)
    expect(platform.hasNoModifier(key({ shiftKey: true }))).toBe(true)
    expect(platform.hasNoModifier(key({ metaKey: true }))).toBe(false)
    expect(platform.hasNoModifier(key({ ctrlKey: true }))).toBe(false)
    expect(platform.hasNoModifier(key({ altKey: true }))).toBe(false)
  })
})

describe('isEditableTarget', () => {
  function input(type: string): HTMLInputElement {
    const element = document.createElement('input')
    element.type = type
    return element
  }

  it('rejects targets that are not HTML elements', () => {
    expect(isEditableTarget(null)).toBe(false)
    expect(isEditableTarget(window)).toBe(false)
    expect(isEditableTarget(document)).toBe(false)
    expect(isEditableTarget(document.createTextNode('text'))).toBe(false)
  })

  it('treats text-like inputs, including range sliders, as editable', () => {
    for (const type of ['text', 'search', 'number', 'email', 'password', 'range']) {
      expect(isEditableTarget(input(type)), type).toBe(true)
    }
  })

  it('does not treat button-like inputs as editable', () => {
    for (const type of ['checkbox', 'radio', 'button', 'submit', 'reset', 'color']) {
      expect(isEditableTarget(input(type)), type).toBe(false)
    }
  })

  it('treats textareas and selects as editable', () => {
    expect(isEditableTarget(document.createElement('textarea'))).toBe(true)
    expect(isEditableTarget(document.createElement('select'))).toBe(true)
  })

  it('treats contenteditable elements and their descendants as editable', () => {
    const editor = document.createElement('div')
    editor.setAttribute('contenteditable', 'true')
    const child = document.createElement('span')
    editor.appendChild(child)
    document.body.appendChild(editor)

    expect(isEditableTarget(editor)).toBe(true)
    expect(isEditableTarget(child)).toBe(true)
  })

  it('does not treat ordinary elements as editable', () => {
    expect(isEditableTarget(document.createElement('div'))).toBe(false)
    expect(isEditableTarget(document.createElement('button'))).toBe(false)
  })
})
