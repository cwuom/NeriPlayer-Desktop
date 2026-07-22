# Dynamic Glass Immersive Player Design

## Goal

Enhance the existing desktop now-playing screen with the selected combination of:

- C: a dynamic glass visual treatment over the existing audio-reactive background; and
- A: an immersive cover and synchronized-lyrics experience.

The first release must keep the existing player engine and business behavior intact. It adds a clear cover/lyrics view switch, a coherent glass control surface, and graceful visual fallbacks.

## Confirmed Product Decisions

- The initial view remains `cover`; the user did not request lyrics as the default.
- The user can switch between cover and lyrics views while playback is active.
- The switch must not interfere with clicking a lyric line to seek.
- Existing queue, prefetch, source resolution, playback effects, and synchronization behavior are out of scope.
- Existing settings for dynamic background, cover blur, audio reactivity, and advanced lyrics remain authoritative.

## Constraints And Licensing

`NeriPlayer-Desktop` is the implementation target. It already contains a desktop-native WebGL/TypeScript implementation of the five-role palette, audio-reactive background, cover blur, and synchronized lyrics.

`NeriPlayer` for Android is GPL-3.0. This work may use its public behavior and architecture as a reference, but it must not copy Android Kotlin, Java, AGSL, or other GPL source into the MIT desktop project. New code will compose and refine the existing desktop implementation.

## Architecture

The change stays in the Vue frontend. No Rust player command, audio callback, or persisted player model changes are required.

1. `NowPlaying.vue` remains the composition owner for the background, cover, lyrics, playback controls, and transient panels.
2. A small pure view-mode module owns the allowed transitions between `cover` and `lyrics`. It has no Vue or Tauri dependency and can be tested with Node.
3. A focused view switch component exposes two icon buttons with `aria-pressed` state and tooltips. It emits a requested mode and does not know player state.
4. Existing `HyperBackground.vue`, `CoverBlurBackground.vue`, `LyricsView.vue`, palette extraction, and player audio-level events remain the rendering and data sources.
5. A single glass control deck groups transport controls and the existing tool row. Shared CSS variables define tint, border, blur, and shadow so the deck and view switch read as one system.

This avoids a desktop rewrite of Android's multi-region `AdvancedGlassHost`. CSS backdrop sampling is sufficient for the first-release control surfaces, while the WebGL canvas continues to own the full-window animated background.

## Interaction Design

### Cover Mode

- Cover mode is selected when the now-playing screen mounts.
- The existing two-column layout remains: cover and controls on the left, synchronized lyrics on the right.
- Clicking the cover requests lyrics mode only when lyrics are available or currently loading.
- The segmented icon switch is always visible while a playback session exists. Its lyrics option is disabled when there is neither lyrics content nor an in-flight lyrics request.

### Lyrics Mode

- The existing lyrics-mode layout expands the lyrics column and collapses the cover column.
- The view switch remains reachable above the lyrics surface and provides the explicit route back to cover mode.
- Clicking lyric lines keeps its current seek behavior; the lyrics surface itself is not used as a mode-toggle hit target.
- If lyrics become unavailable while lyrics mode is active, the mode returns to cover automatically.

### Motion

- Mode transitions use the existing emphasized easing and remain interruptible.
- Cover-to-lyrics motion is opacity plus a small translation; it does not scale the full page.
- With `prefers-reduced-motion: reduce`, translations and beat-driven UI accents are removed and mode changes use a short crossfade.

## Glass Treatment

- The dynamic WebGL or cover-blur background remains full bleed and unframed.
- A single glass deck wraps transport controls and secondary tools. It uses a palette-aware translucent tint, a restrained one-pixel edge, and `backdrop-filter` blur/saturation.
- The view switch uses the same tokens at a smaller scale.
- Cover art and lyrics remain unframed so the screen does not become a collection of nested cards.
- Popovers retain their existing glass styling and do not become children of another decorative card.

When backdrop filtering is unavailable, the same surfaces render with a more opaque palette-aware tint and border. Controls must remain readable without relying on blur.

## Data Flow

1. The Rust audio analyzer continues to emit `player:audio-level`; the player store exposes `audioLevel` and `beatImpulse`.
2. `NowPlaying.vue` passes those values to `HyperBackground.vue` when audio reactivity is enabled.
3. Cover loading produces the existing five-role palette and dynamic CSS variables.
4. The glass deck consumes only CSS variables; it does not subscribe to audio state, preventing beat updates from causing Vue layout work.
5. The mode controller receives current mode, requested mode, lyric availability, and lyric-loading state, then returns the valid next mode.
6. `LyricsView.vue` continues to receive interpolated playback time and seek events unchanged.

## Failure And Edge Cases

- No playback session: preserve the existing empty state and do not show the mode switch.
- Missing or failed cover: retain the existing solid accent/default palette path.
- Dynamic background disabled or palette unavailable: preserve the current cover-blur or solid backdrop behavior.
- WebGL unavailable: the existing solid accent layer remains visible; glass controls must still be readable.
- Backdrop blur unavailable: use the opaque tint fallback.
- Lyrics loading: allow entry into lyrics mode and show the existing loading state.
- No lyrics: disable entry into immersive lyrics mode; if content disappears, return to cover mode.
- Rapid mode changes or track changes: transitions remain state driven, with no timers that can commit stale mode changes.

## Testing And Verification

### Automated

- Add pure tests for view-mode requests: cover to lyrics, lyrics to cover, blocked lyrics entry, loading-state entry, and forced fallback when lyrics disappear.
- Extend the existing now-playing background test coverage only if background selection behavior changes.
- Run all existing Node script tests related to playback, lyrics, player state, track cover, and now-playing background.
- Run `pnpm build` for Vue TypeScript and Vite production output.

### Visual

- Inspect cover and lyrics modes at 1440x900 and 1024x720.
- Verify the control deck does not overlap title bar, cover, progress, lyrics, or popovers.
- Verify long track metadata and translated lyrics do not resize or occlude controls.
- Verify the fallback appearance by disabling backdrop filtering in the browser.
- Verify reduced-motion behavior and rapid switching.

## Success Criteria

- A playback session opens in cover mode.
- The user can enter lyrics mode by clicking the cover or the lyrics icon, and return using the cover icon.
- Lyric-line click-to-seek still works.
- The control deck and mode switch present a coherent dynamic-glass treatment over both WebGL and cover-blur backgrounds.
- Unsupported blur, missing cover, missing lyrics, and reduced-motion states remain usable.
- No Rust playback interfaces or Android source files are copied or changed.
- The production frontend build and focused tests pass.
