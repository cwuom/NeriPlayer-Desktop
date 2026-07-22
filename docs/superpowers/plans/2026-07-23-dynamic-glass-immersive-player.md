# Dynamic Glass Immersive Player Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a reliable cover/lyrics view switch and a palette-aware dynamic-glass control deck to the existing desktop now-playing screen without changing playback business logic.

**Architecture:** Keep `NowPlaying.vue` as the composition owner, move mode-transition policy into a pure TypeScript module, and render the two-mode control through a focused Vue component. Reuse the existing WebGL background, cover palette, audio reactivity, and lyrics renderer; implement glass as progressive CSS enhancement with an opaque fallback.

**Tech Stack:** Vue 3 `<script setup>`, TypeScript, Pinia state consumed by the existing screen, scoped Sass, Material Symbols, Node assertion scripts, Vite, Playwright for temporary visual verification.

---

## File Map

- Create `src/modules/nowPlaying/viewMode.ts`: pure cover/lyrics availability and transition policy.
- Create `scripts/test-now-playing-view-mode.mjs`: executable Node assertions for the policy.
- Create `src/components/NowPlayingViewSwitch.vue`: accessible segmented icon control with glass styling.
- Modify `src/components/NowPlaying.vue`: connect policy, cover click, switch component, glass deck, fallback, and reduced-motion behavior.
- Modify `package.json`: expose the focused view-mode test command.
- Use existing `src/i18n/{en,ja,zh-CN,zh-TW}.json` keys `player.view_mode_cover` and `player.view_mode_lyrics`; no translation edits are needed.

### Task 1: View-Mode Policy With Tests

**Files:**
- Create: `src/modules/nowPlaying/viewMode.ts`
- Create: `scripts/test-now-playing-view-mode.mjs`
- Modify: `package.json`

- [ ] **Step 1: Write the failing policy test**

Create `scripts/test-now-playing-view-mode.mjs`:

```js
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/modules/nowPlaying/viewMode.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const {
  canEnterLyricsMode,
  resolveNowPlayingViewMode,
} = await import(moduleUrl)

assert.equal(canEnterLyricsMode(false, false), false)
assert.equal(canEnterLyricsMode(true, false), true)
assert.equal(canEnterLyricsMode(false, true), true)

assert.equal(resolveNowPlayingViewMode('cover', 'lyrics', false, false), 'cover')
assert.equal(resolveNowPlayingViewMode('cover', 'lyrics', true, false), 'lyrics')
assert.equal(resolveNowPlayingViewMode('cover', 'lyrics', false, true), 'lyrics')
assert.equal(resolveNowPlayingViewMode('lyrics', 'cover', true, false), 'cover')
assert.equal(resolveNowPlayingViewMode('lyrics', 'lyrics', false, false), 'cover')

console.log('now playing view mode tests passed')
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node scripts/test-now-playing-view-mode.mjs`

Expected: FAIL with `ENOENT` for `src/modules/nowPlaying/viewMode.ts`.

- [ ] **Step 3: Implement the pure policy**

Create `src/modules/nowPlaying/viewMode.ts`:

```ts
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
```

- [ ] **Step 4: Add the package command and verify the test passes**

Add this script beside the other focused tests in `package.json`:

```json
"test:now-playing-view-mode": "node scripts/test-now-playing-view-mode.mjs"
```

Run: `pnpm test:now-playing-view-mode`

Expected: `now playing view mode tests passed`.

- [ ] **Step 5: Commit the policy**

```powershell
git add package.json scripts/test-now-playing-view-mode.mjs src/modules/nowPlaying/viewMode.ts
git commit -m "feat(nowplaying): add cover lyrics view policy"
```

### Task 2: Accessible View Switch And Screen Integration

**Files:**
- Create: `src/components/NowPlayingViewSwitch.vue`
- Modify: `src/components/NowPlaying.vue`

- [ ] **Step 1: Create the view switch component**

Create `src/components/NowPlayingViewSwitch.vue`:

```vue
<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { NowPlayingViewMode } from '@/modules/nowPlaying/viewMode'

const props = defineProps<{
  modelValue: NowPlayingViewMode
  lyricsAvailable: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [mode: NowPlayingViewMode]
}>()

const { t } = useI18n()

function requestMode(mode: NowPlayingViewMode) {
  if (mode === 'lyrics' && !props.lyricsAvailable) return
  emit('update:modelValue', mode)
}
</script>

<template>
  <div class="np-view-switch" role="group" :aria-label="t('player.now_playing')">
    <button
      type="button"
      class="np-view-switch__button"
      :class="{ active: modelValue === 'cover' }"
      :aria-label="t('player.view_mode_cover')"
      :aria-pressed="modelValue === 'cover'"
      :title="t('player.view_mode_cover')"
      @click="requestMode('cover')"
    >
      <span class="material-symbols-rounded" aria-hidden="true">album</span>
    </button>
    <button
      type="button"
      class="np-view-switch__button"
      :class="{ active: modelValue === 'lyrics' }"
      :disabled="!lyricsAvailable"
      :aria-label="t('player.view_mode_lyrics')"
      :aria-pressed="modelValue === 'lyrics'"
      :title="t('player.view_mode_lyrics')"
      @click="requestMode('lyrics')"
    >
      <span class="material-symbols-rounded" aria-hidden="true">lyrics</span>
    </button>
  </div>
</template>

<style scoped lang="scss">
.np-view-switch {
  display: grid;
  grid-template-columns: repeat(2, 38px);
  width: 84px;
  height: 42px;
  padding: 2px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 8px;
  background: color-mix(in srgb, var(--np-primary, #16141a) 12%, rgba(18, 17, 22, 0.72));
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.24);
  backdrop-filter: blur(22px) saturate(1.12);
  -webkit-backdrop-filter: blur(22px) saturate(1.12);
}

.np-view-switch__button {
  width: 38px;
  height: 36px;
  display: grid;
  place-items: center;
  border-radius: 6px;
  color: rgba(255, 255, 255, 0.66);
  transition: color 160ms ease, background 180ms ease, transform 160ms ease;

  .material-symbols-rounded { font-size: 20px; }
  &:hover:not(:disabled) { color: rgba(255, 255, 255, 0.94); }
  &:active:not(:disabled) { transform: scale(0.92); }
  &.active {
    color: var(--np-on-primary, #141218);
    background: var(--np-primary-container, rgba(255, 255, 255, 0.88));
  }
  &:disabled { cursor: default; opacity: 0.34; }
}

@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .np-view-switch { background: rgba(24, 22, 28, 0.94); }
}

@media (prefers-reduced-motion: reduce) {
  .np-view-switch__button { transition-duration: 80ms; }
}
</style>
```

- [ ] **Step 2: Import the component and policy into `NowPlaying.vue`**

Add:

```ts
import NowPlayingViewSwitch from './NowPlayingViewSwitch.vue'
import {
  resolveNowPlayingViewMode,
  type NowPlayingViewMode,
} from '@/modules/nowPlaying/viewMode'
```

Change the current declaration to:

```ts
const playViewMode = ref<NowPlayingViewMode>('cover')
```

- [ ] **Step 3: Add availability and transition handlers after `displayLyrics`**

```ts
const lyricsModeAvailable = computed(
  () => displayLyrics.value.length > 0 || isFetchingLyrics.value,
)

function requestPlayViewMode(requestedMode: NowPlayingViewMode) {
  playViewMode.value = resolveNowPlayingViewMode(
    playViewMode.value,
    requestedMode,
    displayLyrics.value.length > 0,
    isFetchingLyrics.value,
  )
}

watch(lyricsModeAvailable, (available) => {
  if (!available && playViewMode.value === 'lyrics') {
    requestPlayViewMode('lyrics')
  }
})
```

The watcher calls `requestPlayViewMode('lyrics')` only for an active lyrics mode. The policy then normalizes that mode to cover when availability disappears, while cover mode remains unchanged.

- [ ] **Step 4: Make the cover an accessible lyrics-mode entry point**

Add these attributes to `.cover-wrap`:

```vue
:class="{
  'cover-wrap--card': settings.coverStyle === 'card',
  'cover-wrap--disc': settings.coverStyle !== 'card',
  'cover-wrap--switching': isTrackSwitchAnimating,
  'cover-wrap--lyrics-entry': lyricsModeAvailable,
}"
:role="lyricsModeAvailable ? 'button' : undefined"
:tabindex="lyricsModeAvailable ? 0 : -1"
:aria-label="lyricsModeAvailable ? t('player.view_mode_lyrics') : undefined"
:title="lyricsModeAvailable ? t('player.view_mode_lyrics') : undefined"
@click="requestPlayViewMode('lyrics')"
@keydown.enter.prevent="requestPlayViewMode('lyrics')"
@keydown.space.prevent="requestPlayViewMode('lyrics')"
```

Keep the existing context-menu handler on the same element.

- [ ] **Step 5: Render the switch without intercepting lyric seeks**

Add the switch as the first child of `.np-right`, before `LyricsView`:

```vue
<NowPlayingViewSwitch
  class="np-view-switch-anchor"
  :model-value="playViewMode"
  :lyrics-available="lyricsModeAvailable"
  @update:model-value="requestPlayViewMode"
/>
```

Add to the scoped Sass:

```scss
.np-right { position: relative; }

.np-view-switch-anchor {
  position: absolute;
  top: 8px;
  right: 48px;
  z-index: 4;
}

.np-body--lyrics-mode .np-view-switch-anchor { right: 48px; }

.cover-wrap--lyrics-entry {
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--np-primary-container, #fff);
    outline-offset: 5px;
  }
}
```

- [ ] **Step 6: Verify focused tests and the production build**

Run:

```powershell
pnpm test:now-playing-view-mode
pnpm build
```

Expected: the focused test prints its pass message; `vue-tsc` and Vite finish without errors.

- [ ] **Step 7: Commit the switch integration**

```powershell
git add src/components/NowPlayingViewSwitch.vue src/components/NowPlaying.vue
git commit -m "feat(nowplaying): switch between cover and lyrics"
```

### Task 3: Dynamic Glass Control Deck

**Files:**
- Modify: `src/components/NowPlaying.vue`

- [ ] **Step 1: Wrap transport controls and tools in one surface**

Insert the opening tag immediately before the existing `.np-controls` element and insert the closing tag immediately after the existing `.np-toolbar` closing `</div>`:

```vue
<!-- insert before the current np-controls element -->
<div class="np-control-deck">

<!-- the current np-controls element and all of its children remain unchanged -->

<!-- the current np-toolbar element and all of its children remain unchanged -->

</div>
```

The resulting nesting must be exactly `.np-control-deck > .np-controls` followed by `.np-toolbar`; no button, popover, or `@click.stop` handler moves between rows.

- [ ] **Step 2: Add stable glass geometry and progressive enhancement**

Add before the current `.np-controls` rule:

```scss
.np-control-deck {
  width: 100%;
  min-height: 108px;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
  padding: 7px 10px 6px;
  border: 1px solid color-mix(in srgb, var(--np-primary-container, #fff) 20%, rgba(255,255,255,0.08));
  border-radius: 8px;
  background: rgba(18, 17, 22, 0.66);
  background: color-mix(in srgb, var(--np-primary, #16141a) 10%, rgba(18, 17, 22, 0.66));
  box-shadow:
    0 16px 42px rgba(0, 0, 0, 0.28),
    inset 0 1px 0 rgba(255, 255, 255, 0.05);
  backdrop-filter: blur(24px) saturate(1.14);
  -webkit-backdrop-filter: blur(24px) saturate(1.14);
  isolation: isolate;
}

@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .np-control-deck {
    background: color-mix(in srgb, var(--np-primary, #16141a) 12%, rgba(18, 17, 22, 0.94));
  }
}
```

- [ ] **Step 3: Normalize child spacing inside the deck**

Update the existing rules:

```scss
.np-controls {
  margin-top: 0;
}

.np-toolbar {
  margin-top: 0;
}
```

Do not add a second background or border to either child row.

- [ ] **Step 4: Add reduced-motion behavior**

Add near the end of the scoped style:

```scss
@media (prefers-reduced-motion: reduce) {
  .np-body,
  .np-left,
  .np-right,
  .cover-wrap,
  .np-control-deck,
  .np-controls,
  .np-toolbar {
    animation: none !important;
    transition-duration: 80ms !important;
  }

  .np-body--lyrics-mode .np-left {
    transform: none;
  }
}
```

- [ ] **Step 5: Build and commit the glass deck**

Run: `pnpm build`

Expected: Vue TypeScript and Vite production build pass.

```powershell
git add src/components/NowPlaying.vue
git commit -m "style(nowplaying): add dynamic glass control deck"
```

### Task 4: Regression And Visual Verification

**Files:**
- No committed source files unless a verified defect requires a focused correction.
- Temporary screenshots and browser scripts belong under ignored `.superpowers/visual-checks/`.

- [ ] **Step 1: Run focused regression tests**

Run:

```powershell
pnpm test:now-playing-view-mode
node scripts/test-now-playing-background.mjs
pnpm test:lyrics-request
pnpm test:lyrics-format
pnpm test:lyric-offset
pnpm test:playback-request
pnpm test:playback-source
pnpm test:player-state
pnpm test:track-cover
```

Expected: every command exits 0 and prints its pass message.

- [ ] **Step 2: Run the full production build**

Run: `pnpm build`

Expected: `vue-tsc --build` and Vite finish successfully.

- [ ] **Step 3: Start Vite for browser verification**

Run: `pnpm dev -- --host 127.0.0.1`

Expected: Vite reports `http://127.0.0.1:1420/`.

- [ ] **Step 4: Inject a playback fixture with Playwright**

In a temporary Playwright script under `.superpowers/visual-checks/`, open the Vite URL and execute this fixture after the page mounts:

```js
await page.evaluate(async () => {
  const { usePlayerStore } = await import('/src/stores/player.ts')
  const player = usePlayerStore()
  player.currentTrack = {
    id: 'local:visual-check',
    title: 'Glass Horizon',
    artist: 'Neri Visual Check',
    album: 'Desktop Sessions',
    coverUrl: 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(
      '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><rect width="512" height="512" fill="#143847"/><circle cx="165" cy="170" r="150" fill="#6ab6c7"/><circle cx="390" cy="385" r="180" fill="#9c5f91"/></svg>'
    ),
    durationMs: 244000,
    audioUrl: '',
  }
  player.hasPlaybackSession = true
  player.durationMs = 244000
  player.positionMs = 74000
  player.isPlaying = true
  player.audioLevel = 0.56
  player.beatImpulse = 0.72
  player.lyrics = [
    { startMs: 0, durationMs: 8000, text: 'The skyline moves in color', words: [] },
    { startMs: 8000, durationMs: 8000, text: 'Every echo turns to light', words: [] },
    { startMs: 16000, durationMs: 8000, text: 'Stay inside the glass horizon', words: [] },
  ]
})
```

Then click `.mp-left` to open now playing.

- [ ] **Step 5: Capture and inspect cover mode**

Set viewport to 1440x900, wait for palette extraction, and save `.superpowers/visual-checks/cover-1440x900.png`. Confirm with screenshot inspection and a canvas pixel sample that:

- the WebGL canvas is not blank;
- the control deck fits below progress and metadata;
- the view switch does not overlap lyrics or the title bar; and
- long metadata stays within the left column.

Repeat at 1024x720 as `.superpowers/visual-checks/cover-1024x720.png`.

- [ ] **Step 6: Capture and inspect lyrics mode**

Click the lyrics button in `.np-view-switch`, save both viewport screenshots, and confirm:

- the left column collapses without leaving an interactive invisible layer;
- the lyrics column uses the available width;
- the switch remains reachable; and
- clicking a lyric line still updates the player seek command rather than changing view mode.

- [ ] **Step 7: Verify fallbacks and reduced motion**

Use Playwright media emulation for `reducedMotion: 'reduce'`. Inject this style to emulate missing backdrop filtering:

```js
await page.addStyleTag({
  content: '.np-control-deck,.np-view-switch{backdrop-filter:none!important;-webkit-backdrop-filter:none!important}',
})
```

Confirm controls remain readable and no transition overlaps occur during rapid cover/lyrics toggling.

- [ ] **Step 8: Review the final diff and repository state**

Run:

```powershell
git diff origin/main...HEAD --check
git diff origin/main...HEAD --stat
git status --short --branch
```

Expected: no whitespace errors, only the design/plan plus intended implementation files are committed, and the worktree is clean.
