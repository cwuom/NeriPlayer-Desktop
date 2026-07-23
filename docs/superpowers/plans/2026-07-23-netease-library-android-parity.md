# NetEase Library Android Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the two peer-level NetEase library tabs with one Android-aligned NetEase surface containing Playlists and Albums categories, search, refresh, isolated failures, and legacy URL migration.

**Architecture:** Keep `LibraryView.vue` as the composition owner, but move route normalization, category filtering, and request-generation/coalescing rules into three small pure TypeScript modules. The existing recommend store remains the shared owner of NetEase playlist and album collections; the view adds category-specific presentation state without adding a backend endpoint or runtime dependency.

**Tech Stack:** Vue 3 `<script setup>`, TypeScript, Pinia, Vue Router, Vue I18n, Tauri invoke APIs, Node 24 source/module tests, Vite/Vue type checking, SCSS.

---

### Task 1: Lock The Four-Tab Route Contract

**Files:**
- Create: `src/modules/library/libraryRoute.ts`
- Create: `scripts/test-library-route.mjs`
- Modify: `package.json`

- [ ] **Step 1: Write the failing route test**

Create `scripts/test-library-route.mjs` with a TypeScript transpile helper and assertions for canonical, legacy, array-valued, and invalid queries:

```js
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/libraryRoute.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { resolveLibraryLocation, buildLibraryQuery, isCanonicalLibraryLocation } = await import(moduleUrl)

assert.deepEqual(resolveLibraryLocation('local', undefined), { tab: 'local', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation('netease', 'albums'), { tab: 'netease', category: 'albums' })
assert.deepEqual(resolveLibraryLocation(['netease'], ['playlists']), { tab: 'netease', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation('netease_playlists', undefined), { tab: 'netease', category: 'playlists' })
assert.deepEqual(resolveLibraryLocation('netease_albums', undefined), { tab: 'netease', category: 'albums' })
assert.deepEqual(resolveLibraryLocation('unknown', 'albums'), { tab: 'local', category: 'playlists' })
assert.deepEqual(buildLibraryQuery({ tab: 'netease', category: 'albums' }), { tab: 'netease', category: 'albums' })
assert.deepEqual(buildLibraryQuery({ tab: 'downloads', category: 'albums' }), { tab: 'downloads' })
assert.equal(isCanonicalLibraryLocation('netease', 'albums'), true)
assert.equal(isCanonicalLibraryLocation('netease_albums', undefined), false)
assert.equal(isCanonicalLibraryLocation('favorites', 'albums'), false)

console.log('library route tests passed')
```

- [ ] **Step 2: Run the test and verify RED**

Run: `node scripts/test-library-route.mjs`

Expected: FAIL with `ENOENT` for `src/modules/library/libraryRoute.ts`.

- [ ] **Step 3: Implement the pure route contract**

Create `src/modules/library/libraryRoute.ts`:

```ts
export type LibraryTabKey = 'local' | 'favorites' | 'downloads' | 'netease'
export type NeteaseLibraryCategory = 'playlists' | 'albums'

export interface LibraryLocation {
  tab: LibraryTabKey
  category: NeteaseLibraryCategory
}

const LIBRARY_TABS = new Set<LibraryTabKey>(['local', 'favorites', 'downloads', 'netease'])
const NETEASE_CATEGORIES = new Set<NeteaseLibraryCategory>(['playlists', 'albums'])

function firstQueryValue(value: unknown): string | undefined {
  if (Array.isArray(value)) return firstQueryValue(value[0])
  return typeof value === 'string' ? value : undefined
}

export function resolveLibraryLocation(rawTab: unknown, rawCategory: unknown): LibraryLocation {
  const tabValue = firstQueryValue(rawTab)
  const categoryValue = firstQueryValue(rawCategory)

  if (tabValue === 'netease_playlists') return { tab: 'netease', category: 'playlists' }
  if (tabValue === 'netease_albums') return { tab: 'netease', category: 'albums' }

  const tab = LIBRARY_TABS.has(tabValue as LibraryTabKey)
    ? tabValue as LibraryTabKey
    : 'local'
  const category = tab === 'netease' && NETEASE_CATEGORIES.has(categoryValue as NeteaseLibraryCategory)
    ? categoryValue as NeteaseLibraryCategory
    : 'playlists'
  return { tab, category }
}

export function buildLibraryQuery(location: LibraryLocation): Record<string, string> {
  return location.tab === 'netease'
    ? { tab: 'netease', category: location.category }
    : { tab: location.tab }
}

export function isCanonicalLibraryLocation(rawTab: unknown, rawCategory: unknown): boolean {
  const tab = firstQueryValue(rawTab)
  const category = firstQueryValue(rawCategory)
  const resolved = resolveLibraryLocation(rawTab, rawCategory)
  const canonical = buildLibraryQuery(resolved)
  return tab === canonical.tab && category === canonical.category
}
```

- [ ] **Step 4: Register and run the route test**

Add to `package.json`:

```json
"test:library-route": "node scripts/test-library-route.mjs"
```

Append `pnpm test:library-route` to `test:netease`.

Run: `pnpm test:library-route`

Expected: `library route tests passed`.

- [ ] **Step 5: Commit the route contract**

```powershell
git add package.json scripts/test-library-route.mjs src/modules/library/libraryRoute.ts
git commit -m "test(library): define Android-aligned route contract"
```

### Task 2: Add Category-Specific NetEase Filtering

**Files:**
- Create: `src/modules/library/neteaseLibraryFilter.ts`
- Create: `scripts/test-library-netease-filter.mjs`
- Modify: `src/stores/recommend.ts`
- Modify: `package.json`

- [ ] **Step 1: Write the failing filter test**

Create `scripts/test-library-netease-filter.mjs`:

```js
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/neteaseLibraryFilter.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { filterNeteasePlaylists, filterNeteaseAlbums } = await import(moduleUrl)

const playlists = [
  { id: 123, name: 'Morning Mix', playCount: 8700, trackCount: 42 },
  { id: 456, name: 'Night Drive', playCount: 120, trackCount: 18 },
]
assert.equal(filterNeteasePlaylists(playlists, ' morning ').length, 1)
assert.equal(filterNeteasePlaylists(playlists, '456')[0].name, 'Night Drive')
assert.equal(filterNeteasePlaylists(playlists, '8700')[0].id, 123)
assert.equal(filterNeteasePlaylists(playlists, '42')[0].id, 123)
assert.equal(filterNeteasePlaylists(playlists, 'missing').length, 0)
assert.equal(filterNeteasePlaylists(playlists, '').length, 2)

const albums = [
  { id: 99, name: 'Blue Hour', artist: 'Alice', trackCount: 11 },
  { id: 100, name: 'Red Moon', artist: 'Bob', trackCount: 8 },
]
assert.equal(filterNeteaseAlbums(albums, 'alice')[0].id, 99)
assert.equal(filterNeteaseAlbums(albums, '100')[0].artist, 'Bob')
assert.equal(filterNeteaseAlbums(albums, '11')[0].name, 'Blue Hour')
assert.equal(filterNeteaseAlbums(albums, 'unknown').length, 0)

console.log('library NetEase filter tests passed')
```

- [ ] **Step 2: Run the test and verify RED**

Run: `node scripts/test-library-netease-filter.mjs`

Expected: FAIL with `ENOENT` for `neteaseLibraryFilter.ts`.

- [ ] **Step 3: Implement the filter module**

Create `src/modules/library/neteaseLibraryFilter.ts`:

```ts
export interface SearchableNeteasePlaylist {
  id: string | number
  name: string
  playCount?: number
  trackCount: number
}

export interface SearchableNeteaseAlbum {
  id: string | number
  name: string
  artist?: string
  trackCount: number
}

function normalizeSearch(value: unknown): string {
  return String(value ?? '').trim().toLocaleLowerCase()
}

function matches(query: string, values: unknown[]): boolean {
  return values.some(value => normalizeSearch(value).includes(query))
}

export function filterNeteasePlaylists<T extends SearchableNeteasePlaylist>(items: T[], rawQuery: string): T[] {
  const query = normalizeSearch(rawQuery)
  if (!query) return items
  return items.filter(item => matches(query, [item.name, item.id, item.playCount ?? 0, item.trackCount]))
}

export function filterNeteaseAlbums<T extends SearchableNeteaseAlbum>(items: T[], rawQuery: string): T[] {
  const query = normalizeSearch(rawQuery)
  if (!query) return items
  return items.filter(item => matches(query, [item.name, item.artist ?? '', item.id, item.trackCount]))
}
```

- [ ] **Step 4: Preserve NetEase display metadata in the recommend store**

Extend `PlaylistInfo` in `src/stores/recommend.ts`:

```ts
export interface PlaylistInfo {
  id: string | number
  name: string
  coverUrl: string
  trackCount: number
  playCount?: number
  description?: string
  creator?: string
}

export interface UserAlbumInfo {
  id: string | number
  name: string
  coverUrl: string
  artist: string
  trackCount: number
}
```

Type `userAlbums` as `ref<UserAlbumInfo[]>([])` and map `p.playCount` in the NetEase playlist branch:

```ts
playCount: Number(p.playCount) || 0,
```

Keep other platform mappings unchanged because `playCount` is optional.

- [ ] **Step 5: Register and run the filter test and build**

Add `test:library-netease-filter` to `package.json` and append it to `test:netease`.

Run: `pnpm test:library-netease-filter`

Expected: `library NetEase filter tests passed`.

Run: `pnpm build`

Expected: Vue type checking and Vite build succeed; the existing chunk-size warning is allowed.

- [ ] **Step 6: Commit filtering and metadata**

```powershell
git add package.json scripts/test-library-netease-filter.mjs src/modules/library/neteaseLibraryFilter.ts src/stores/recommend.ts
git commit -m "feat(library): add NetEase category filtering"
```

### Task 3: Make Refresh Coalescing And Stale Results Testable

**Files:**
- Create: `src/modules/library/neteaseLibraryRequest.ts`
- Create: `scripts/test-library-netease-request.mjs`
- Modify: `package.json`

- [ ] **Step 1: Write the failing request-coordinator test**

Create `scripts/test-library-netease-request.mjs`:

```js
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/library/neteaseLibraryRequest.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { NeteaseLibraryRequestCoordinator } = await import(moduleUrl)

const deferred = () => {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}

const coordinator = new NeteaseLibraryRequestCoordinator()
const playlists = deferred()
const albums = deferred()
let playlistCalls = 0
let albumCalls = 0
const first = coordinator.run(
  () => { playlistCalls += 1; return playlists.promise },
  () => { albumCalls += 1; return albums.promise },
)
const coalesced = coordinator.run(
  () => { playlistCalls += 1; return Promise.resolve(true) },
  () => { albumCalls += 1; return Promise.resolve(true) },
)
assert.equal(first.started, true)
assert.equal(coalesced.started, false)
assert.equal(first.promise, coalesced.promise)
assert.equal(playlistCalls, 1)
assert.equal(albumCalls, 1)
playlists.resolve(true)
albums.resolve(false)
assert.deepEqual(await first.promise, { current: true, playlistsOk: true, albumsOk: false })

const stalePlaylists = deferred()
const staleAlbums = deferred()
const stale = coordinator.run(() => stalePlaylists.promise, () => staleAlbums.promise)
coordinator.invalidate()
const fresh = coordinator.run(() => Promise.resolve(true), () => Promise.resolve(true))
stalePlaylists.resolve(true)
staleAlbums.resolve(true)
assert.equal((await stale.promise).current, false)
assert.deepEqual(await fresh.promise, { current: true, playlistsOk: true, albumsOk: true })

console.log('library NetEase request tests passed')
```

- [ ] **Step 2: Run the test and verify RED**

Run: `node scripts/test-library-netease-request.mjs`

Expected: FAIL with `ENOENT` for `neteaseLibraryRequest.ts`.

- [ ] **Step 3: Implement the request coordinator**

Create `src/modules/library/neteaseLibraryRequest.ts`:

```ts
export interface NeteaseLibraryRequestResult {
  current: boolean
  playlistsOk: boolean
  albumsOk: boolean
}

export interface CoordinatedNeteaseLibraryRequest {
  started: boolean
  promise: Promise<NeteaseLibraryRequestResult>
}

export class NeteaseLibraryRequestCoordinator {
  private generation = 0
  private inFlight: Promise<NeteaseLibraryRequestResult> | null = null

  invalidate(): void {
    this.generation += 1
    this.inFlight = null
  }

  run(
    loadPlaylists: () => Promise<boolean>,
    loadAlbums: () => Promise<boolean>,
  ): CoordinatedNeteaseLibraryRequest {
    if (this.inFlight) return { started: false, promise: this.inFlight }

    const generation = this.generation
    let request!: Promise<NeteaseLibraryRequestResult>
    request = Promise.all([
      loadPlaylists().catch(() => false),
      loadAlbums().catch(() => false),
    ])
      .then(([playlistsOk, albumsOk]) => ({
        current: generation === this.generation,
        playlistsOk,
        albumsOk,
      }))
      .finally(() => {
        if (this.inFlight === request) this.inFlight = null
      })
    this.inFlight = request
    return { started: true, promise: request }
  }
}
```

- [ ] **Step 4: Register and run the coordinator test**

Add `test:library-netease-request` to `package.json` and append it to `test:netease`.

Run: `pnpm test:library-netease-request`

Expected: `library NetEase request tests passed`.

- [ ] **Step 5: Commit the coordinator**

```powershell
git add package.json scripts/test-library-netease-request.mjs src/modules/library/neteaseLibraryRequest.ts
git commit -m "test(library): cover NetEase refresh coordination"
```

### Task 4: Replace Five Primary Tabs With Android-Aligned Navigation

**Files:**
- Modify: `scripts/test-library-tab-config.mjs`
- Modify: `src/views/LibraryView.vue`

- [ ] **Step 1: Change the tab contract test to expect the new hierarchy**

Replace the five-key assertions in `scripts/test-library-tab-config.mjs` with:

```js
for (const key of ['local', 'favorites', 'downloads', 'netease']) {
  assert.match(source, new RegExp(`key: ['"]${key}['"]`), `missing public library tab: ${key}`)
}
assert.doesNotMatch(source, /key:\s*['"]netease_playlists['"]/)
assert.doesNotMatch(source, /key:\s*['"]netease_albums['"]/)
assert.match(source, /key:\s*['"]playlists['"]/)
assert.match(source, /key:\s*['"]albums['"]/)
assert.match(source, /resolveLibraryLocation/)
assert.match(source, /buildLibraryQuery/)
assert.match(source, /grid-template-columns:\s*repeat\(4,\s*minmax\(0,\s*1fr\)\)/)
```

Keep the assertions excluding visible Bilibili and YouTube keys.

- [ ] **Step 2: Run the tab and route tests and verify RED**

Run: `pnpm test:library-tab-config`

Expected: FAIL because the view still declares five primary tabs.

Run: `pnpm test:library-route`

Expected: PASS, proving the migration helper itself remains correct.

- [ ] **Step 3: Integrate the route helper and four primary keys**

In `LibraryView.vue`, import the route helpers and types:

```ts
import {
  buildLibraryQuery,
  isCanonicalLibraryLocation,
  resolveLibraryLocation,
  type LibraryTabKey,
  type NeteaseLibraryCategory,
} from '@/modules/library/libraryRoute'
```

Replace the local five-key union, set, and resolver with:

```ts
const initialLocation = resolveLibraryLocation(route.query.tab, route.query.category)
const activeTab = ref<LibraryTabKey>(initialLocation.tab)
const neteaseCategory = ref<NeteaseLibraryCategory>(initialLocation.category)

const tabs = computed<LibraryTabDefinition[]>(() => [
  { label: t('library.tab_local'), icon: 'folder_open', key: 'local' },
  { label: t('library.tab_favorites'), icon: 'favorite', key: 'favorites' },
  { label: t('library.tab_downloads'), icon: 'download', key: 'downloads' },
  { label: t('library.tab_netease'), icon: 'cloud', key: 'netease' },
])

const neteaseCategories = computed(() => [
  { label: t('library.netease_category_playlists'), key: 'playlists' as const },
  { label: t('library.netease_category_albums'), key: 'albums' as const },
])
```

Implement one canonical route writer that preserves unrelated query fields and removes stale library keys:

```ts
function replaceLibraryLocation(location: { tab: LibraryTabKey; category: NeteaseLibraryCategory }) {
  const { tab: _tab, category: _category, ...otherQuery } = route.query
  void router.replace({ query: { ...otherQuery, ...buildLibraryQuery(location) } })
}

function activateTab(tab: LibraryTabKey) {
  activeTab.value = tab
  replaceLibraryLocation({ tab, category: neteaseCategory.value })
}

function activateNeteaseCategory(category: NeteaseLibraryCategory) {
  neteaseCategory.value = category
  replaceLibraryLocation({ tab: 'netease', category })
}

watch(() => [route.query.tab, route.query.category] as const, ([tab, category]) => {
  const location = resolveLibraryLocation(tab, category)
  activeTab.value = location.tab
  neteaseCategory.value = location.category
  if (!isCanonicalLibraryLocation(tab, category)) replaceLibraryLocation(location)
}, { immediate: true })
```

- [ ] **Step 4: Add the secondary selector and merge the NetEase template branch**

Render one `activeTab === 'netease'` container. At its top, add:

```vue
<div class="netease-category-bar" role="tablist" :aria-label="t('library.tab_netease')">
  <button
    v-for="category in neteaseCategories"
    :key="category.key"
    type="button"
    class="netease-category-tab"
    :class="{ active: neteaseCategory === category.key }"
    :aria-selected="neteaseCategory === category.key"
    role="tab"
    @click="activateNeteaseCategory(category.key)"
  >
    {{ category.label }}
  </button>
</div>
```

Move the existing playlist and album content under `neteaseCategory === 'playlists'` and `neteaseCategory === 'albums'` branches. Do not change their network behavior yet.

- [ ] **Step 5: Change the primary grid to four equal tracks**

In scoped SCSS:

```scss
.tab-bar {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.netease-category-bar {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 4px;
  min-height: 40px;
  padding: 4px;
  margin-bottom: 12px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
}

.netease-category-tab {
  min-width: 0;
  min-height: 32px;
  padding: 6px 12px;
  border-radius: var(--radius-sm);
  color: var(--md-on-surface-variant);
  font-size: 13px;
  font-weight: 500;
}

.netease-category-tab.active {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
  font-weight: 600;
}
```

- [ ] **Step 6: Run focused tests and build**

Run: `pnpm test:library-tab-config`

Expected: `library tab config tests passed`.

Run: `pnpm test:library-route`

Expected: `library route tests passed`.

Run: `pnpm build`

Expected: PASS with only the existing chunk-size warning.

- [ ] **Step 7: Commit the navigation hierarchy**

```powershell
git add scripts/test-library-tab-config.mjs src/views/LibraryView.vue
git commit -m "feat(library): align NetEase navigation with Android"
```

### Task 5: Integrate Isolated Load State And One Session Refresh

**Files:**
- Modify: `scripts/test-library-netease-auth-refresh.mjs`
- Modify: `src/views/LibraryView.vue`

- [ ] **Step 1: Rewrite the auth-refresh contract test for the new state model**

Update `scripts/test-library-netease-auth-refresh.mjs` to assert:

```js
assert.match(source, /new\s+NeteaseLibraryRequestCoordinator\(\)/)
assert.match(source, /const\s+neteaseSessionFingerprint\s*=\s*computed/)
assert.match(source, /watch\(neteaseSessionFingerprint/)
assert.doesNotMatch(source, /watch\(\(\)\s*=>\s*auth\.netease\.nickname/)
assert.equal(
  source.match(/watch\([^)]*auth\.netease\.loggedIn/g)?.length ?? 0,
  0,
  'session refresh must not be split across independent auth watchers',
)
assert.match(source, /const\s+neteasePlaylistLoading\s*=\s*ref\(false\)/)
assert.match(source, /const\s+neteaseAlbumLoading\s*=\s*ref\(false\)/)
assert.match(source, /const\s+neteasePlaylistError\s*=\s*ref(?:<[^>]+>)?\(null\)/)
assert.match(source, /const\s+neteaseAlbumError\s*=\s*ref(?:<[^>]+>)?\(null\)/)
assert.match(source, /result\.playlistsOk/)
assert.match(source, /result\.albumsOk/)
assert.match(source, /recommend\.userAlbums\s*=\s*\[\]/)
assert.match(source, /recommend\.userPlaylists\[['"]netease['"]\]\s*=\s*\[\]/)
```

Keep store assertions that `fetchUserPlaylists` and `fetchUserAlbums` return `Promise<boolean>`.

- [ ] **Step 2: Run the auth-refresh test and verify RED**

Run: `pnpm test:library-netease-auth-refresh`

Expected: FAIL because the view still uses one shared loading/error state and three auth watchers.

- [ ] **Step 3: Integrate the request coordinator and split category state**

Import and instantiate the coordinator:

```ts
import { NeteaseLibraryRequestCoordinator } from '@/modules/library/neteaseLibraryRequest'

const neteaseRequestCoordinator = new NeteaseLibraryRequestCoordinator()
const neteasePlaylistLoading = ref(false)
const neteaseAlbumLoading = ref(false)
const neteasePlaylistError = ref<string | null>(null)
const neteaseAlbumError = ref<string | null>(null)
```

Replace `loadNeteaseLibrary` with:

```ts
async function loadNeteaseLibrary() {
  if (!auth.netease.loggedIn) return
  const request = neteaseRequestCoordinator.run(
    () => recommend.fetchUserPlaylists('netease'),
    () => recommend.fetchUserAlbums(),
  )
  if (!request.started) return request.promise

  neteasePlaylistLoading.value = true
  neteaseAlbumLoading.value = true
  neteasePlaylistError.value = null
  neteaseAlbumError.value = null

  const result = await request.promise
  if (!result.current || !auth.netease.loggedIn) return
  neteasePlaylistLoading.value = false
  neteaseAlbumLoading.value = false
  neteasePlaylistError.value = result.playlistsOk ? null : t('player.load_failed')
  neteaseAlbumError.value = result.albumsOk ? null : t('player.load_failed')
}
```

Add a single session fingerprint observer:

```ts
const neteaseSessionFingerprint = computed(() =>
  `${auth.netease.loggedIn ? '1' : '0'}:${auth.neteaseSessionVersion}`,
)

watch(neteaseSessionFingerprint, () => {
  neteaseRequestCoordinator.invalidate()
  neteasePlaylistLoading.value = false
  neteaseAlbumLoading.value = false
  neteasePlaylistError.value = null
  neteaseAlbumError.value = null
  recommend.userPlaylists['netease'] = []
  recommend.userAlbums = []
  if (auth.netease.loggedIn) void loadNeteaseLibrary()
}, { immediate: true })
```

Remove the separate `loggedIn`, nickname, and session-version watchers.

- [ ] **Step 4: Bind each category to its own state and retry**

In the playlist branch, use only `neteasePlaylistLoading` and `neteasePlaylistError`. In the album branch, use only `neteaseAlbumLoading` and `neteaseAlbumError`. Both retry buttons call `loadNeteaseLibrary`; the coordinator coalesces a duplicate click.

Keep successful rows visible during a manual refresh by showing a compact spinner on the refresh action when the corresponding collection is already non-empty. Use the full loading state only when the active collection is empty.

- [ ] **Step 5: Run request, auth-refresh, and build checks**

Run: `pnpm test:library-netease-request`

Expected: PASS.

Run: `pnpm test:library-netease-auth-refresh`

Expected: `library netease auth refresh tests passed`.

Run: `pnpm build`

Expected: PASS with only the existing chunk-size warning.

- [ ] **Step 6: Commit the load-state integration**

```powershell
git add scripts/test-library-netease-auth-refresh.mjs src/views/LibraryView.vue
git commit -m "fix(library): isolate NetEase category refresh state"
```

### Task 6: Match Android Search, Metadata, Empty, And Refresh Behavior

**Files:**
- Modify: `src/views/LibraryView.vue`
- Modify: `src/i18n/en.json`
- Modify: `src/i18n/ja.json`
- Modify: `src/i18n/zh-CN.json`
- Modify: `src/i18n/zh-TW.json`
- Create: `scripts/test-library-netease-surface.mjs`
- Modify: `package.json`

- [ ] **Step 1: Write the failing surface contract test**

Create `scripts/test-library-netease-surface.mjs`:

```js
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const source = await readFile(new URL('../src/views/LibraryView.vue', import.meta.url), 'utf8')
for (const locale of ['en', 'ja', 'zh-CN', 'zh-TW']) {
  const messages = JSON.parse(await readFile(new URL(`../src/i18n/${locale}.json`, import.meta.url), 'utf8'))
  for (const key of [
    'tab_netease',
    'netease_category_playlists',
    'netease_category_albums',
    'netease_search_placeholder',
    'netease_search_empty_title',
    'netease_search_empty_desc',
    'netease_playlist_empty',
    'netease_album_empty',
    'netease_play_count',
    'netease_login',
  ]) {
    assert.equal(typeof messages.library[key], 'string', `${locale} missing library.${key}`)
  }
}

assert.match(source, /filterNeteasePlaylists/)
assert.match(source, /filterNeteaseAlbums/)
assert.match(source, /const\s+neteasePlaylistSearchQuery\s*=\s*ref\(['"]['"]\)/)
assert.match(source, /const\s+neteaseAlbumSearchQuery\s*=\s*ref\(['"]['"]\)/)
assert.match(source, /class="netease-search"/)
assert.match(source, /class="netease-refresh"/)
assert.match(source, /neteaseFilteredPlaylists\.length/)
assert.match(source, /neteaseFilteredAlbums\.length/)
assert.match(source, /npl\.playCount/)

console.log('library NetEase surface tests passed')
```

- [ ] **Step 2: Run the surface test and verify RED**

Run: `node scripts/test-library-netease-surface.mjs`

Expected: FAIL because the locale keys and Android-aligned search surface do not exist.

- [ ] **Step 3: Add typed category filters to the view**

Import the pure filters and add separate queries:

```ts
import {
  filterNeteaseAlbums,
  filterNeteasePlaylists,
} from '@/modules/library/neteaseLibraryFilter'

const neteasePlaylistSearchQuery = ref('')
const neteaseAlbumSearchQuery = ref('')
const neteaseFilteredPlaylists = computed(() =>
  filterNeteasePlaylists(neteasePlaylists.value, neteasePlaylistSearchQuery.value),
)
const neteaseFilteredAlbums = computed(() =>
  filterNeteaseAlbums(recommend.userAlbums, neteaseAlbumSearchQuery.value),
)
const activeNeteaseSearchQuery = computed({
  get: () => neteaseCategory.value === 'playlists'
    ? neteasePlaylistSearchQuery.value
    : neteaseAlbumSearchQuery.value,
  set: value => {
    if (neteaseCategory.value === 'playlists') neteasePlaylistSearchQuery.value = value
    else neteaseAlbumSearchQuery.value = value
  },
})
```

- [ ] **Step 4: Render the inline search and refresh action**

Immediately below the secondary selector, render:

```vue
<div class="netease-tools">
  <label class="netease-search">
    <span class="material-symbols-rounded" aria-hidden="true">search</span>
    <input
      v-model="activeNeteaseSearchQuery"
      type="search"
      :placeholder="t('library.netease_search_placeholder')"
    />
  </label>
  <button
    type="button"
    class="netease-refresh"
    :disabled="neteasePlaylistLoading || neteaseAlbumLoading"
    :title="t('common.refresh')"
    @click="loadNeteaseLibrary"
  >
    <span
      class="material-symbols-rounded"
      :class="{ spinning: neteasePlaylistLoading || neteaseAlbumLoading }"
      aria-hidden="true"
    >refresh</span>
  </button>
</div>
```

Use `neteaseFilteredPlaylists` and `neteaseFilteredAlbums` for the rows. Distinguish source-empty from search-empty by checking the unfiltered and filtered collection lengths separately.

In the logged-out branch, keep the existing explanatory copy and add a real login action:

```vue
<button
  type="button"
  class="retry-btn"
  :disabled="auth.loggingIn === 'netease'"
  @click="auth.loginNetease"
>
  <span class="material-symbols-rounded" aria-hidden="true">login</span>
  <span>{{ t('library.netease_login') }}</span>
</button>
```

- [ ] **Step 5: Match Android row metadata**

For playlist rows, render play count and track count:

```vue
<div class="pl-count">
  {{ t('library.netease_play_count', { count: npl.playCount || 0 }) }}
  <span aria-hidden="true">&middot;</span>
  {{ t('library.track_count', { count: npl.trackCount || 0 }) }}
</div>
```

Album rows keep artist and track count. Continue routing rows to `netease-playlist` and `netease-album`.

- [ ] **Step 6: Add complete locale keys**

Add the following exact keys under `library` in each locale JSON file.

`src/i18n/en.json`:

```json
{
  "tab_netease": "NetEase",
  "netease_category_playlists": "Playlists",
  "netease_category_albums": "Albums",
  "netease_search_placeholder": "Search playlists / albums",
  "netease_search_empty_title": "No matching playlists or albums",
  "netease_search_empty_desc": "Try a name, ID, play count, or track count",
  "netease_playlist_empty": "No NetEase playlists loaded yet",
  "netease_album_empty": "No NetEase albums loaded yet",
  "netease_play_count": "{count} plays",
  "netease_login": "Sign in to NetEase"
}
```

`src/i18n/zh-CN.json`:

```json
{
  "tab_netease": "网易云",
  "netease_category_playlists": "歌单",
  "netease_category_albums": "专辑",
  "netease_search_placeholder": "搜索歌单 / 专辑",
  "netease_search_empty_title": "没有找到匹配的歌单或专辑",
  "netease_search_empty_desc": "试试输入名称、ID、播放数或曲目数",
  "netease_playlist_empty": "还没有加载到网易云歌单",
  "netease_album_empty": "还没有加载到网易云专辑",
  "netease_play_count": "播放 {count} 次",
  "netease_login": "登录网易云"
}
```

`src/i18n/zh-TW.json`:

```json
{
  "tab_netease": "網易雲",
  "netease_category_playlists": "歌單",
  "netease_category_albums": "專輯",
  "netease_search_placeholder": "搜尋歌單 / 專輯",
  "netease_search_empty_title": "找不到符合的歌單或專輯",
  "netease_search_empty_desc": "試試輸入名稱、ID、播放次數或曲目數",
  "netease_playlist_empty": "尚未載入網易雲歌單",
  "netease_album_empty": "尚未載入網易雲專輯",
  "netease_play_count": "播放 {count} 次",
  "netease_login": "登入網易雲"
}
```

`src/i18n/ja.json`:

```json
{
  "tab_netease": "NetEase",
  "netease_category_playlists": "プレイリスト",
  "netease_category_albums": "アルバム",
  "netease_search_placeholder": "プレイリスト / アルバムを検索",
  "netease_search_empty_title": "一致するプレイリストまたはアルバムがありません",
  "netease_search_empty_desc": "名前、ID、再生回数、曲数で検索してください",
  "netease_playlist_empty": "NetEaseのプレイリストはまだ読み込まれていません",
  "netease_album_empty": "NetEaseのアルバムはまだ読み込まれていません",
  "netease_play_count": "再生 {count} 回",
  "netease_login": "NetEaseにログイン"
}
```

Retain the old `tab_netease_playlists` and `tab_netease_albums` keys only if another source reference still uses them; otherwise remove them after `rg` confirms zero consumers.

- [ ] **Step 7: Add stable tool-row styling**

Add scoped SCSS with bounded dimensions:

```scss
.netease-tools {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 40px;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}

.netease-search {
  min-width: 0;
  min-height: 40px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-md);
  color: var(--md-on-surface-variant);
}

.netease-search input {
  width: 100%;
  min-width: 0;
  border: 0;
  outline: 0;
  color: var(--md-on-surface);
  background: transparent;
}

.netease-refresh {
  width: 40px;
  height: 40px;
  display: grid;
  place-items: center;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
}
```

- [ ] **Step 8: Register and run surface tests, aggregate tests, and build**

Add `test:library-netease-surface` to `package.json` and append it to `test:netease`.

Run: `pnpm test:library-netease-surface`

Expected: `library NetEase surface tests passed`.

Run: `pnpm test:netease`

Expected: every NetEase-focused regression passes.

Run: `pnpm build`

Expected: PASS with only the existing chunk-size warning.

- [ ] **Step 9: Commit the Android-parity surface**

```powershell
git add package.json scripts/test-library-netease-surface.mjs src/views/LibraryView.vue src/i18n/en.json src/i18n/ja.json src/i18n/zh-CN.json src/i18n/zh-TW.json
git commit -m "feat(library): match Android NetEase library surface"
```

### Task 7: Verify The Full User Workflow

**Files:**
- Modify only if verification exposes a scoped defect.

- [ ] **Step 1: Run all focused automated verification**

Run each command independently:

```powershell
pnpm test:library-route
pnpm test:library-netease-filter
pnpm test:library-netease-request
pnpm test:library-tab-config
pnpm test:library-netease-auth-refresh
pnpm test:library-netease-surface
pnpm test:netease
pnpm build
git diff --check
```

Expected: every test and build exits `0`; the Vite chunk-size and Git line-ending warnings are acceptable, but test failures, type errors, whitespace errors, and malformed JSON are not.

- [ ] **Step 2: Check whether Rust verification is available**

Run:

```powershell
Get-Command cargo -ErrorAction SilentlyContinue
Get-Command rustc -ErrorAction SilentlyContinue
```

If available, run `cargo test --manifest-path src-tauri/Cargo.toml` and `cargo check --manifest-path src-tauri/Cargo.toml`. If unavailable, record that limitation in the handoff; this feature does not modify Rust.

- [ ] **Step 3: Verify canonical and legacy routes in the running app**

At desktop and 575x898 viewports, inspect:

```text
/library?tab=netease&category=playlists
/library?tab=netease&category=albums
/library?tab=netease_playlists
/library?tab=netease_albums
```

Expected: legacy URLs normalize once to canonical queries; the correct secondary category is selected; no horizontal overflow or selector shift occurs.

- [ ] **Step 4: Verify interactive states**

In the running app:

1. Switch among all four primary tabs and both NetEase categories.
2. Enter different playlist and album queries, switch categories, and confirm each query is retained independently.
3. Clear search and verify the server order returns.
4. Trigger refresh repeatedly and verify only one visible refresh cycle runs.
5. Open one playlist and one album, then navigate back and confirm the selected NetEase category remains correct.
6. Verify logged-out state exposes login and no old account rows.
7. Confirm Bilibili and YouTube entries remain absent from the visible Library navigation.

Expected: all interactions work without console errors, page errors, overlapping text, or stale-account content.

- [ ] **Step 5: Run a final diff audit**

Run:

```powershell
git status --short
git diff --stat
git diff --check
```

Expected: only intended implementation/test/i18n files remain modified; no generated screenshots or unrelated files are added.

- [ ] **Step 6: Commit any verification-only scoped fixes**

Only if Step 3 or Step 4 required a code fix, stage the reviewed implementation paths explicitly and commit them:

```powershell
git add -- src/views/LibraryView.vue src/modules/library/libraryRoute.ts src/modules/library/neteaseLibraryFilter.ts src/modules/library/neteaseLibraryRequest.ts scripts/test-library-route.mjs scripts/test-library-netease-filter.mjs scripts/test-library-netease-request.mjs scripts/test-library-tab-config.mjs scripts/test-library-netease-auth-refresh.mjs scripts/test-library-netease-surface.mjs package.json src/i18n/en.json src/i18n/ja.json src/i18n/zh-CN.json src/i18n/zh-TW.json
git commit -m "fix(library): resolve Android parity verification issues"
```
