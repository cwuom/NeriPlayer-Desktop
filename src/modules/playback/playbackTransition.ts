/** 设置页的淡入淡出与切歌交叉淡化参数 */
export interface PlaybackTransitionSettings {
  fadeIn: boolean
  fadeInDuration: number
  fadeOutDuration: number
  crossfadeNext: boolean
  crossfadeInDuration: number
  crossfadeOutDuration: number
}

export interface PlaybackTransitionPlan {
  useOverlapCrossfade: boolean
  wantsCrossfade: boolean
  transitionFadeInMs: number
  transitionFadeOutMs: number
}

export function resolvePlaybackTransition(
  settings: PlaybackTransitionSettings,
  wasPlayingBeforeSwitch: boolean,
  isSwitchingTrack: boolean,
  requestedStartMs: number,
): PlaybackTransitionPlan {
  const fadeInDurationMs = Math.max(0, Math.round(settings.fadeInDuration))
  const fadeOutDurationMs = Math.max(0, Math.round(settings.fadeOutDuration))
  const overlapFadeInDurationMs = settings.crossfadeNext
    ? Math.max(0, Math.round(settings.crossfadeInDuration))
    : fadeInDurationMs
  const overlapFadeOutDurationMs = settings.crossfadeNext
    ? Math.max(0, Math.round(settings.crossfadeOutDuration))
    : fadeOutDurationMs
  const useOverlapCrossfade = wasPlayingBeforeSwitch && isSwitchingTrack
    && settings.crossfadeNext
    && overlapFadeOutDurationMs > 0
    && overlapFadeInDurationMs > 0
  const useTrackSwitchFadeIn = settings.fadeIn
    && wasPlayingBeforeSwitch
    && isSwitchingTrack
    && !useOverlapCrossfade
    && fadeInDurationMs > 0
  const useInitialFadeIn = settings.fadeIn
    && !wasPlayingBeforeSwitch
    && fadeInDurationMs > 0
    && requestedStartMs === 0
  const transitionFadeInMs = useTrackSwitchFadeIn || useInitialFadeIn
    ? fadeInDurationMs
    : overlapFadeInDurationMs
  const wantsCrossfade = useOverlapCrossfade || useTrackSwitchFadeIn || useInitialFadeIn
  const transitionFadeOutMs = useOverlapCrossfade ? overlapFadeOutDurationMs : 0
  return { useOverlapCrossfade, wantsCrossfade, transitionFadeInMs, transitionFadeOutMs }
}
