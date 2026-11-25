import { describe, expect, it } from 'vitest'
import { installScrolledState, SCROLLED_ATTRIBUTE, SCROLLED_THRESHOLD_PX } from '@/integrations/scrolledState'

function container(scrollTop = 0) {
  const attributes = new Set<string>()
  return {
    scrollTop,
    attributes,
    toggleAttribute(name: string, force: boolean) {
      if (force) attributes.add(name)
      else attributes.delete(name)
      return force
    },
  }
}

function root(scrollTop = 0) {
  const base = container(scrollTop)
  const listeners: Array<{ listener: (event: { target: unknown }) => void, options: AddEventListenerOptions }> = []
  return Object.assign(base, {
    listeners,
    addEventListener(_type: 'scroll', listener: (event: { target: unknown }) => void, options: AddEventListenerOptions) {
      listeners.push({ listener, options })
    },
    removeEventListener(_type: 'scroll', listener: (event: { target: unknown }) => void) {
      const index = listeners.findIndex(entry => entry.listener === listener)
      if (index >= 0) listeners.splice(index, 1)
    },
    dispatch(target: unknown) {
      for (const { listener } of listeners) listener({ target })
    },
  })
}

describe('scrolled state for frosted top bars', () => {
  it('listens in the capture phase so nested page scrollers are tracked too', () => {
    const content = root()
    installScrolledState(content)
    expect(content.listeners[0].options).toMatchObject({ capture: true, passive: true })

    const detailView = container()
    detailView.scrollTop = 120
    content.dispatch(detailView)
    expect(detailView.attributes.has(SCROLLED_ATTRIBUTE)).toBe(true)
    expect(content.attributes.has(SCROLLED_ATTRIBUTE)).toBe(false)

    detailView.scrollTop = 0
    content.dispatch(detailView)
    expect(detailView.attributes.has(SCROLLED_ATTRIBUTE)).toBe(false)
  })

  it('reflects a restored scroll position immediately and ignores sub-threshold bounce', () => {
    const content = root(400)
    installScrolledState(content)
    expect(content.attributes.has(SCROLLED_ATTRIBUTE)).toBe(true)

    content.scrollTop = SCROLLED_THRESHOLD_PX
    content.dispatch(content)
    expect(content.attributes.has(SCROLLED_ATTRIBUTE)).toBe(false)
  })

  it('ignores non-element targets and stops listening after uninstall', () => {
    const content = root()
    const uninstall = installScrolledState(content)
    expect(() => content.dispatch({})).not.toThrow()
    uninstall()
    expect(content.listeners).toHaveLength(0)
  })
})
