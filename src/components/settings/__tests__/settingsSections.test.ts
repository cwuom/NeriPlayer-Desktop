import { describe, expect, it } from 'vitest'
import { readSettingsUiState } from '@/components/settings/useSettingsSections'

function storage(value: string | null) {
  return { getItem: () => value }
}

describe('readSettingsUiState', () => {
  it('restores a saved state that points at a known section', () => {
    const saved = { activeSection: 'lyrics', expandedSections: ['lyrics'], visitedSections: ['lyrics'], scrollPositions: { lyrics: 120 } }
    expect(readSettingsUiState(storage(JSON.stringify(saved)))).toEqual(saved)
  })

  it('discards states whose active section no longer exists', () => {
    expect(readSettingsUiState(storage(JSON.stringify({ activeSection: 'personal', expandedSections: ['x'] })))).toEqual({})
    expect(readSettingsUiState(storage(JSON.stringify({ expandedSections: ['x'] })))).toEqual({})
  })

  it('falls back to defaults for missing, corrupt or unreadable storage', () => {
    expect(readSettingsUiState(storage(null))).toEqual({})
    expect(readSettingsUiState(storage('{oops'))).toEqual({})
    expect(readSettingsUiState(storage('null'))).toEqual({})
    expect(readSettingsUiState({ getItem: () => { throw new DOMException('blocked', 'SecurityError') } })).toEqual({})
  })
})
