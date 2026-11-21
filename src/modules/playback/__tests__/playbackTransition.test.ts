import { describe, expect, it } from 'vitest'
import { resolvePlaybackTransition, type PlaybackTransitionSettings } from '@/modules/playback/playbackTransition'

function settings(overrides: Partial<PlaybackTransitionSettings> = {}): PlaybackTransitionSettings {
  return {
    fadeIn: false,
    fadeInDuration: 500,
    fadeOutDuration: 400,
    crossfadeNext: false,
    crossfadeInDuration: 800,
    crossfadeOutDuration: 700,
    ...overrides,
  }
}

describe('resolvePlaybackTransition', () => {
  it('starts without any fade when fades and crossfade are off', () => {
    expect(resolvePlaybackTransition(settings(), true, true, 0)).toEqual({
      useOverlapCrossfade: false,
      wantsCrossfade: false,
      transitionFadeInMs: 500,
      transitionFadeOutMs: 0,
    })
  })

  it('overlaps the outgoing track only when switching tracks while playing', () => {
    const crossfade = settings({ crossfadeNext: true })
    expect(resolvePlaybackTransition(crossfade, true, true, 0)).toEqual({
      useOverlapCrossfade: true,
      wantsCrossfade: true,
      transitionFadeInMs: 800,
      transitionFadeOutMs: 700,
    })
    expect(resolvePlaybackTransition(crossfade, false, true, 0).useOverlapCrossfade).toBe(false)
    expect(resolvePlaybackTransition(crossfade, true, false, 0).useOverlapCrossfade).toBe(false)
  })

  it('needs both crossfade durations to overlap', () => {
    const plan = resolvePlaybackTransition(settings({ crossfadeNext: true, crossfadeOutDuration: 0 }), true, true, 0)
    expect(plan.useOverlapCrossfade).toBe(false)
    expect(plan.transitionFadeOutMs).toBe(0)
  })

  it('fades in a switched track when crossfade is off', () => {
    expect(resolvePlaybackTransition(settings({ fadeIn: true }), true, true, 30_000)).toEqual({
      useOverlapCrossfade: false,
      wantsCrossfade: true,
      transitionFadeInMs: 500,
      transitionFadeOutMs: 0,
    })
  })

  it('fades in a cold start only from the beginning of the track', () => {
    const fadeIn = settings({ fadeIn: true, crossfadeNext: true })
    expect(resolvePlaybackTransition(fadeIn, false, false, 0)).toMatchObject({ wantsCrossfade: true, transitionFadeInMs: 500 })
    expect(resolvePlaybackTransition(fadeIn, false, false, 42_000)).toMatchObject({ wantsCrossfade: false, transitionFadeInMs: 800 })
  })

  it('rounds durations and treats negative values as zero', () => {
    const plan = resolvePlaybackTransition(
      settings({ fadeIn: true, fadeInDuration: -20, crossfadeNext: true, crossfadeInDuration: 249.6, crossfadeOutDuration: 100.4 }),
      true,
      true,
      0,
    )
    expect(plan).toEqual({ useOverlapCrossfade: true, wantsCrossfade: true, transitionFadeInMs: 250, transitionFadeOutMs: 100 })
  })
})
