import { describe, expect, it } from 'vitest'
import { readSidebarCollapsed, writeSidebarCollapsed } from '@/modules/layout/sidebar'

function memory() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value) },
  }
}

describe('sidebar collapse preference', () => {
  it('defaults to expanded', () => {
    expect(readSidebarCollapsed(memory())).toBe(false)
  })

  it('round-trips the collapsed flag', () => {
    const storage = memory()
    writeSidebarCollapsed(true, storage)
    expect(readSidebarCollapsed(storage)).toBe(true)
    writeSidebarCollapsed(false, storage)
    expect(readSidebarCollapsed(storage)).toBe(false)
  })

  it('treats unavailable storage as expanded and ignores write failures', () => {
    const broken = {
      getItem: () => { throw new Error('denied') },
      setItem: () => { throw new Error('denied') },
    }
    expect(readSidebarCollapsed(broken)).toBe(false)
    expect(() => writeSidebarCollapsed(true, broken)).not.toThrow()
  })
})
