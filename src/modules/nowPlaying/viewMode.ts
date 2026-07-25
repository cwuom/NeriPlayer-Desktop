export type NowPlayingViewMode = 'cover' | 'lyrics'

export function canEnterLyricsMode(
  hasLyrics: boolean,
  isFetchingLyrics: boolean,
): boolean {
  return hasLyrics || isFetchingLyrics
}

export function resolveNowPlayingViewMode(
  currentMode: NowPlayingViewMode,
  requestedMode: NowPlayingViewMode,
  hasLyrics: boolean,
  isFetchingLyrics: boolean,
): NowPlayingViewMode {
  const lyricsAvailable = canEnterLyricsMode(hasLyrics, isFetchingLyrics)
  if (currentMode === 'lyrics' && !lyricsAvailable) return 'cover'
  if (requestedMode === 'lyrics' && !lyricsAvailable) return currentMode
  return requestedMode
}
