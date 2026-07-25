export type PlaybackUiSource = 'netease' | 'qq' | 'bilibili' | 'youtube' | 'local'
export type OnlinePlaybackUiSource = Exclude<PlaybackUiSource, 'local'>

export interface PlaybackSourceTrack {
  id: string
}

export interface QualityReplayState {
  track?: PlaybackSourceTrack | null
  audioInfoSource?: string | null
  isLoadingAudio: boolean
  isPlayingFromDownload: boolean
}

const PLAYBACK_UI_SOURCES: PlaybackUiSource[] = [
  'netease',
  'qq',
  'bilibili',
  'youtube',
  'local',
]

function normalizePlaybackSource(source?: string | null): PlaybackUiSource | null {
  const normalized = source?.trim().toLowerCase()
  return PLAYBACK_UI_SOURCES.includes(normalized as PlaybackUiSource)
    ? normalized as PlaybackUiSource
    : null
}

export function logicalPlaybackSource(
  track?: PlaybackSourceTrack | null,
): PlaybackUiSource {
  const idPrefix = track?.id.split(':', 1)[0]?.toLowerCase()
  return normalizePlaybackSource(idPrefix) ?? 'local'
}

export function resolvedPlaybackSourceForUi(
  track?: PlaybackSourceTrack | null,
  audioInfoSource?: string | null,
): PlaybackUiSource {
  if (!track) return 'local'
  return normalizePlaybackSource(audioInfoSource) ?? logicalPlaybackSource(track)
}

export function shouldReplayForQualityChange(
  source: OnlinePlaybackUiSource,
  state: QualityReplayState,
): boolean {
  return Boolean(
    state.track
    && !state.isLoadingAudio
    && !state.isPlayingFromDownload
    && resolvedPlaybackSourceForUi(state.track, state.audioInfoSource) === source,
  )
}
