import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive, toRefs } from 'vue'
import type { SettingsIpc } from '@/components/settings/settingsShared'

const state = reactive({
  audioOutputDevice: 'Speakers (USB DAC)',
  audioExclusiveMode: false,
  volumeBalance: 0,
  neteaseQuality: 'exhigh',
  youtubeQuality: 'high',
  biliQuality: 'high',
  youtubePlaybackSource: 'auto',
})
const toastError = vi.fn()

vi.mock('pinia', () => ({ storeToRefs: () => toRefs(state) }))
vi.mock('vue-i18n', () => ({ useI18n: () => ({ t: (key: string) => key }) }))
vi.mock('@/stores/settings', () => ({ YOUTUBE_PLAYBACK_SOURCES: ['auto'], useSettingsStore: () => state }))
vi.mock('@/stores/player', () => ({ usePlayerStore: () => ({}) }))
vi.mock('@/stores/toast', () => ({ useToastStore: () => ({ error: toastError }) }))
vi.mock('@/modules/shortcuts/platform', () => ({ isWindowsPlatform: true }))

const { usePlaybackSettings } = await import('@/components/settings/usePlaybackSettings')

function ipcWith(setAudioExclusiveMode: SettingsIpc['setAudioExclusiveMode']) {
  return { setAudioExclusiveMode, listAudioOutputDevices: async () => [] } as unknown as SettingsIpc
}

describe('WASAPI exclusive output toggle', () => {
  beforeEach(() => {
    state.audioExclusiveMode = false
    toastError.mockReset()
  })

  it('is offered on Windows and sends the selected device so the backend rebuilds that output', async () => {
    const setExclusive = vi.fn(async () => undefined)
    const settings = usePlaybackSettings(ipcWith(setExclusive))
    expect(settings.exclusiveOutputAvailable).toBe(true)

    await settings.changeAudioExclusiveMode(true)

    expect(setExclusive).toHaveBeenCalledWith(true, 'Speakers (USB DAC)')
    expect(state.audioExclusiveMode).toBe(true)
    expect(settings.audioOutputSwitching.value).toBe(false)
  })

  it('keeps the saved value and reports the failure when the backend refuses', async () => {
    const settings = usePlaybackSettings(ipcWith(vi.fn(async () => { throw new Error('device busy') })))

    await settings.changeAudioExclusiveMode(true)

    expect(state.audioExclusiveMode).toBe(false)
    expect(toastError).toHaveBeenCalledWith('settings.audio_exclusive_failed')
    expect(settings.audioOutputSwitching.value).toBe(false)
  })

  it('ignores repeated clicks while a rebuild is in flight and no-op toggles', async () => {
    let finish: () => void = () => {}
    const setExclusive = vi.fn(() => new Promise<void>((resolve) => { finish = resolve }))
    const settings = usePlaybackSettings(ipcWith(setExclusive))

    const first = settings.changeAudioExclusiveMode(true)
    await settings.changeAudioExclusiveMode(true)
    await settings.changeAudioExclusiveMode(false)
    finish()
    await first
    await settings.changeAudioExclusiveMode(true)

    expect(setExclusive).toHaveBeenCalledTimes(1)
    expect(state.audioExclusiveMode).toBe(true)
  })
})
