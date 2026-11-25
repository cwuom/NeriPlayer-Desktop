import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async (..._args: unknown[]): Promise<unknown> => undefined),
  open: vi.fn(async (..._args: unknown[]): Promise<unknown> => null),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open }))

import {
  cacheLocationErrorKey,
  getMediaCacheLocation,
  openMediaCacheDir,
  pickMediaCacheDir,
  resetMediaCacheDir,
  selectedDirectory,
  setMediaCacheDir,
} from '@/modules/settings/cacheLocation'

describe('cache location IPC', () => {
  beforeEach(() => {
    mocks.invoke.mockClear()
    mocks.open.mockReset()
  })

  it('calls the registered backend commands with the existing-cache action', async () => {
    await getMediaCacheLocation()
    await setMediaCacheDir('D:\\NeriCache', 'move')
    await resetMediaCacheDir('discard')
    await openMediaCacheDir()
    expect(mocks.invoke.mock.calls).toEqual([
      ['get_media_cache_dir'],
      ['set_media_cache_dir', { path: 'D:\\NeriCache', existing: 'move' }],
      ['reset_media_cache_dir', { existing: 'discard' }],
      ['open_media_cache_dir'],
    ])
  })

  it('picks a single directory starting from the current location', async () => {
    mocks.open.mockResolvedValueOnce('/Volumes/Data/Cache')
    await expect(pickMediaCacheDir('/old')).resolves.toBe('/Volumes/Data/Cache')
    expect(mocks.open).toHaveBeenCalledWith({ directory: true, multiple: false, defaultPath: '/old' })
    mocks.open.mockResolvedValueOnce(null)
    await expect(pickMediaCacheDir()).resolves.toBeNull()
  })
})

describe('selectedDirectory', () => {
  it('accepts strings, arrays and legacy { path } results', () => {
    expect(selectedDirectory(' /a ')).toBe('/a')
    expect(selectedDirectory(['/b', '/c'])).toBe('/b')
    expect(selectedDirectory({ path: '/d' })).toBe('/d')
  })

  it('treats cancellation and blank selections as no choice', () => {
    expect(selectedDirectory(null)).toBeNull()
    expect(selectedDirectory('   ')).toBeNull()
    expect(selectedDirectory([])).toBeNull()
    expect(selectedDirectory({ path: 3 })).toBeNull()
    expect(selectedDirectory({ path: '  ' })).toBeNull()
    expect(selectedDirectory({ other: '/e' })).toBeNull()
    expect(selectedDirectory(42)).toBeNull()
  })
})

describe('cacheLocationErrorKey', () => {
  it('maps backend error codes to specific messages', () => {
    expect(cacheLocationErrorKey('CACHE_DIR_NOT_WRITABLE')).toBe('settings.cache_location_error_not_writable')
    expect(cacheLocationErrorKey('CACHE_DIR_NOT_ABSOLUTE')).toBe('settings.cache_location_error_invalid')
    expect(cacheLocationErrorKey('CACHE_DIR_NOT_DIRECTORY')).toBe('settings.cache_location_error_invalid')
    expect(cacheLocationErrorKey('CACHE_DIR_OVERLAPS')).toBe('settings.cache_location_error_overlap')
    expect(cacheLocationErrorKey(new Error('CACHE_DIR_BUSY'))).toBe('settings.cache_location_error_busy')
  })

  it('falls back to a generic failure for anything else', () => {
    expect(cacheLocationErrorKey('IO error: denied')).toBe('settings.cache_location_failed')
    expect(cacheLocationErrorKey(undefined)).toBe('settings.cache_location_failed')
  })
})
