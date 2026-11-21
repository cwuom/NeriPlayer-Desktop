import { describe, expect, it, vi } from 'vitest'
import { audioOutputLabel, audioOutputOptionsFor } from '@/components/settings/usePlaybackSettings'
import type { AudioOutputDeviceOption } from '@/components/settings/settingsShared'

// 纯函数不碰 store；真实 store 模块在导入时就会读 localStorage
vi.mock('@/stores/settings', () => ({ YOUTUBE_PLAYBACK_SOURCES: [], useSettingsStore: vi.fn() }))
vi.mock('@/stores/player', () => ({ usePlayerStore: vi.fn() }))

const t = (key: string, named?: Record<string, unknown>) => {
  if (key === 'settings.audio_output_role_analog_stereo') return 'Analog Stereo'
  if (key === 'settings.audio_output_role_surround') return ` ${String(named?.detail)} Surround `
  if (key === 'settings.audio_output_system_default') return '(default)'
  if (key === 'settings.audio_output_unavailable') return `${String(named?.name)} (unavailable)`
  if (key === 'settings.audio_output_default') return 'System default'
  return key
}

function device(overrides: Partial<AudioOutputDeviceOption>): AudioOutputDeviceOption {
  return { name: 'hw:0', label: '', isDefault: false, ...overrides }
}

describe('audioOutputLabel', () => {
  it('composes ALSA devices as <card> <role> with the trimmed role text', () => {
    expect(audioOutputLabel(device({ card: 'USB Audio', role: 'analog_stereo' }), t)).toBe('USB Audio Analog Stereo')
    expect(audioOutputLabel(device({ role: 'surround', detail: '5.1' }), t)).toBe('5.1 Surround')
  })

  it('falls back to the backend label, then the device name', () => {
    expect(audioOutputLabel(device({ label: 'Speakers' }), t)).toBe('Speakers')
    expect(audioOutputLabel(device({ name: 'pulse' }), t)).toBe('pulse')
  })

  it('marks the sound server sink that currently routes the audio', () => {
    expect(audioOutputLabel(device({ label: 'Headphones', kind: 'sink', isDefault: true }), t)).toBe('Headphones (default)')
    expect(audioOutputLabel(device({ label: 'Headphones', kind: 'device', isDefault: true }), t)).toBe('Headphones')
  })
})

describe('audioOutputOptionsFor', () => {
  it('prepends the system default and disambiguates duplicate labels with the PCM name', () => {
    const options = audioOutputOptionsFor([
      device({ name: 'front:CARD=0', card: 'USB', role: 'analog_stereo' }),
      device({ name: 'surround40:CARD=0', card: 'USB', role: 'analog_stereo' }),
      device({ name: 'hdmi', label: 'HDMI' }),
    ], 'hdmi', t)
    expect(options).toEqual([
      { value: '', label: 'System default' },
      { value: 'front:CARD=0', label: 'USB Analog Stereo (front:CARD=0)' },
      { value: 'surround40:CARD=0', label: 'USB Analog Stereo (surround40:CARD=0)' },
      { value: 'hdmi', label: 'HDMI' },
    ])
  })

  it('keeps a saved device that disappeared visible as unavailable', () => {
    expect(audioOutputOptionsFor([device({ name: 'hdmi', label: 'HDMI' })], 'usb', t)).toEqual([
      { value: '', label: 'System default' },
      { value: 'usb', label: 'usb (unavailable)' },
      { value: 'hdmi', label: 'HDMI' },
    ])
  })
})
