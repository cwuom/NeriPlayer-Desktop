import { useSettingsStore } from '@/stores/settings'

export function useTrackActivation() {
  const settings = useSettingsStore()
  let selectionClick = false

  function onTrackClick(event: MouseEvent | undefined, activate: () => void, selectionMode = false) {
    const detail = event?.detail ?? 0
    if (detail > 1) return
    selectionClick = selectionMode
    if (selectionMode || detail === 0 || !settings.doubleClickPlay) activate()
  }

  function onTrackDoubleClick(event: MouseEvent | undefined, activate: () => void, selectionMode = false) {
    if (selectionMode || selectionClick || !settings.doubleClickPlay) return
    const control = (event?.target as Element | null)?.closest?.('button, input, a')
    if (control && control !== event?.currentTarget) return
    activate()
  }

  return { onTrackClick, onTrackDoubleClick }
}
