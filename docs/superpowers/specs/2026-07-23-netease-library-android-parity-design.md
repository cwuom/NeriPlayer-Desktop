# NetEase Library Android-Parity Design

## Goal

Make the desktop library follow the current Android NetEase information architecture while keeping desktop-only downloads and the already-working NetEase account, playback, playlist-detail, and album-detail flows.

The visible library hierarchy becomes:

- Local
- Favorites
- Downloads
- NetEase
  - Playlists
  - Albums

This replaces the current five peer-level tabs, where NetEase playlists and NetEase albums are separate primary tabs.

## Confirmed Product Decisions

- The Android project is authoritative for the NetEase library interaction model.
- Desktop keeps a Downloads primary tab because downloads are a real desktop workflow even though Android does not expose that tab in the same position.
- NetEase is one primary tab with Playlists and Albums as secondary categories.
- NetEase playlists remain a flat server-ordered list. This release does not invent Created, Subscribed, or Liked groups because Android does not present those groups.
- Favorites remains NeriPlayer's own cross-source favorite collection. It is not relabeled as the NetEase cloud liked list.
- Existing Bilibili and YouTube backend compatibility remains in place, but those platforms are not restored to the visible primary library navigation.
- Existing playlist and album detail routes remain the destination for NetEase rows.

## Constraints And Licensing

`NeriPlayer-Desktop` is the implementation target. `NeriPlayer` for Android is GPL-3.0 and is used only as a behavioral and architectural reference. No Kotlin, Java, Compose, or other GPL implementation code will be copied into the desktop project.

The change should reuse the current Vue, Pinia, Vue Router, and Tauri command patterns. It adds no new runtime dependency and no new NetEase network endpoint.

## Navigation Contract

The canonical primary tab keys are:

- `local`
- `favorites`
- `downloads`
- `netease`

The canonical NetEase category keys are:

- `playlists`
- `albums`

The route query contract is `?tab=netease&category=playlists|albums`. The category query is omitted for non-NetEase tabs.

Old desktop links remain compatible:

- `?tab=netease_playlists` normalizes to `?tab=netease&category=playlists`.
- `?tab=netease_albums` normalizes to `?tab=netease&category=albums`.
- Invalid primary or category values normalize to `local` and `playlists` respectively.

Changing primary or secondary categories uses `router.replace` so category browsing does not fill browser history with presentation-only state. Playlist and album row clicks continue to use normal route navigation.

## Architecture

### Library View

`LibraryView.vue` remains the composition owner for all four primary categories. It gains a NetEase-only secondary segmented control, category-specific inline search, and a single refresh action that follows Android behavior by refreshing both NetEase playlists and albums.

The primary and secondary selectors have separate state and route normalization. They must not share numeric indexes; string keys remain the public contract.

### NetEase Data

`recommend.ts` continues to own the fetched NetEase playlist and album collections so Home and Library share account-aware cache invalidation.

`PlaylistInfo` is extended only with server metadata needed for Android-parity display and search, including play count. The existing flattened list and server order remain intact.

The Library view derives filtered rows from the store collections. Filtering is pure and category-specific:

- Playlist search matches name, ID, play count, or track count.
- Album search matches name, artist, ID, or track count.
- Playlist and album search queries are retained separately when switching categories, matching Android's two-query behavior.

### Request State

Playlist and album loads have independent success and error results. A playlist failure must not make the album category appear failed, and vice versa.

One library refresh cycle owns a generation token. Logout, login, or account replacement invalidates the generation before clearing old account data. A response may commit only when its generation and current authenticated session still match.

The existing `loggedIn`, display-name, and `neteaseSessionVersion` observers are replaced by one coalesced session fingerprint observer so one account event starts one refresh cycle instead of several duplicate cycles.

## Interaction Design

### Primary Navigation

The four primary choices are equal-width and retain the existing compact desktop style. They remain readable at the supported narrow viewport without horizontal overflow.

Switching away from Local exits local multi-select mode. Switching away from NetEase preserves each NetEase search query and selected category for the current page lifetime.

### NetEase Header

The NetEase surface starts with:

1. a two-option Playlists/Albums segmented control;
2. a category-specific inline search field; and
3. a refresh icon action with an accessible tooltip.

The refresh action fetches playlists and albums together, as Android does. It is disabled only while the same refresh cycle is active.

### NetEase Rows

Playlist rows show cover, name, play count, and track count. Album rows show cover, name, artist, and track count. Rows remain unframed list items and do not introduce nested cards.

Clicking a playlist opens `netease-playlist`. Clicking an album opens `netease-album`. Existing cover fallback and failed-image handling remain in use.

### Loading, Empty, And Login States

- Initial load uses a stable loading state that does not resize the selector or search field.
- An empty unfiltered playlist category reports that no NetEase playlists were loaded.
- An empty unfiltered album category reports that no NetEase albums were loaded.
- A non-empty source list with zero search matches reports a search-specific empty state and keeps the query editable.
- A logged-out session shows a NetEase login action and does not display stale rows from the previous account.
- A same-account manual refresh keeps the last successful rows visible while indicating refresh progress.

## Failure And Edge Cases

- Playlist request fails, album succeeds: Playlists shows its own retry state; Albums remains usable.
- Album request fails, playlist succeeds: Albums shows its own retry state; Playlists remains usable.
- One request finishes after logout or account replacement: discard it without changing rows, errors, or loading state for the new session.
- Rapid primary/category switching: route state and visible category remain consistent; no request is started merely by switching the secondary selector.
- Old five-tab URLs: normalize once to the canonical four-tab contract without a redirect loop.
- Missing cover, play count, artist, or track count: use existing cover fallback and neutral metadata defaults without dropping the row.
- Search text containing whitespace or mixed case: trim and compare case-insensitively.
- Refresh while a refresh is active: coalesce into the in-flight cycle rather than launch duplicate playlist and album calls.

## Testing And Verification

### Pure And Source Contract Tests

- Update the library tab contract test to require four primary keys and the NetEase secondary keys.
- Test legacy query normalization for both previous NetEase primary tab keys.
- Add focused tests for playlist and album filtering, including name, ID, metadata, whitespace, and no-match cases.
- Extend the auth refresh regression to prove one account transition starts one refresh cycle and stale responses cannot commit.
- Test category-specific failure so one failed collection does not hide the successful collection.
- Keep the existing tests proving Bilibili and YouTube placeholders are absent from the visible Explore and Library surfaces.

### Build And Runtime Verification

- Run `pnpm test:netease` plus the new focused library tests.
- Run `pnpm build` and `git diff --check`.
- Run Rust unit tests and `cargo check` when a Rust toolchain is available. This feature itself should not require Rust source changes.
- Inspect `/library` at desktop and 575x898 viewports.
- Verify the four primary choices and two secondary choices do not overflow or shift.
- Verify both legacy URLs normalize correctly.
- Verify playlist/album search, refresh, login, empty, partial-failure, and detail navigation states.
- Confirm no browser console or page errors.

## Non-Goals

- NetEase QR login.
- NetEase artist detail.
- Remote playlist create, rename, delete, subscribe, or unsubscribe mutations.
- A new cloud-liked playlist grouping.
- Removing Bilibili or YouTube backend modules.
- Refactoring unrelated Local, Favorites, Downloads, player, or settings behavior.

## Success Criteria

- Library exposes exactly four primary choices: Local, Favorites, Downloads, and NetEase.
- NetEase exposes Playlists and Albums as secondary choices in one primary surface.
- The visible NetEase structure, search behavior, row metadata, refresh behavior, and empty states match Android's current library behavior.
- Old NetEase playlist/album tab URLs remain usable through canonical normalization.
- Playlist and album failures are isolated, account changes cannot leak stale data, and duplicate auth observers do not trigger duplicate refresh cycles.
- Existing NetEase playlist and album detail navigation remains functional.
- Visible Bilibili and YouTube library entries remain absent while backend compatibility is preserved.
- Focused tests, the production frontend build, diff checks, and browser verification pass.
