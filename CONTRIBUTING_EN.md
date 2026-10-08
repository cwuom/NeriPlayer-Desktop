[English](./CONTRIBUTING_EN.md) | [中文](./CONTRIBUTING.md)

## Contributing to NeriPlayer Desktop

Thanks for contributing to NeriPlayer Desktop. This document describes
**the actual current implementation of the desktop client** — keep it in
sync with the source code and runtime behavior.

> [!CAUTION]
> The desktop port is at an early stage: parts of this document and the
> README are placeholder text, implementations may be missing or
> unpolished, and in the worst case data may be corrupted. The
> repository may become unmaintained in the future — it is meant to be
> community-driven. When docs and code disagree, trust the code and fix
> the docs along the way.

---

### Scope

- NeriPlayer Desktop is the **Tauri 2 desktop port** of NeriPlayer
  (Android), not a public cloud music service.
- Online content comes from **NetEase Cloud Music**, **QQ Music**,
  **Bilibili**, and **YouTube Music**; lyric completion also uses
  **LRCLIB**.
- Data stays local by default; GitHub / WebDAV sync is **optional** and
  covers metadata (playlists, favorites, recent plays, stats), not media
  files.
- The cloud-sync data model and the Listen Together protocol **must stay
  compatible with the Android app** — this is the most important external
  constraint of this repository.
- The Listen Together server lives in the Android repo's
  `np-submodule/NeriPlayer-LTW` (Cloudflare Workers + Durable Objects);
  this repo only implements the client.

---

### Documentation Map

- `README.md` / `README_EN.md` — for users and new contributors:
  positioning, capability boundaries, install/build, sync, privacy.
- `CONTRIBUTING.md` / `CONTRIBUTING_EN.md` — for developers: real module
  boundaries, extension paths, testing, and PR requirements.
- `scripts/test-sync-android-interop.ps1` — Kotlin reverse decoding with
  read-only Android sources and existing Gradle caches.
- `CODE_OF_CONDUCT.md` — community standards.
- `CLAUDE.md` / `AGENTS.md` — repo guides for AI coding agents; human
  contributors can use them as an architecture primer.
- `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md` —
  issue and PR templates.

If a behavior change affects user understanding, update the README; if it
affects extension paths, testing, or module boundaries, update
CONTRIBUTING.

---

### Development Environment

- **Node.js**: 20+ (CI uses 20)
- **pnpm**: `9.15.9` (pinned; `corepack enable`)
- **Rust**: `1.95.0` (pinned by `rust-toolchain.toml`)
- **Tauri**: 2.x; **Vue**: 3.5; **Vite**: 6; **TypeScript**: 5.8
- **Version name format**: `<git-short-hash>.<MMddHHmm>` (Asia/Taipei)

Platform dependencies:

- **Windows**: Visual Studio C++ Build Tools, WebView2 runtime
- **macOS**: Xcode Command Line Tools
- **Linux** (Debian/Ubuntu):
  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
    librsvg2-dev patchelf libasound2-dev
  ```

Notes:

- The repo depends on the `vendor/applemusic-like-lyrics` submodule
  (AMLL lyric components). Clone with `--recursive` or run
  `git submodule update --init --recursive`.
- The build reads the Git short commit for the version name — make sure
  Git is installed.
- `pnpm tauri` is forwarded through `scripts/run-tauri.mjs`, which
  injects `NERI_BUILD_EPOCH` so `src-tauri/build.rs` can generate a
  reproducible build timestamp.

---

### Quick Start

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install

pnpm tauri dev      # full app (Vite :1420, or the next free port if it is taken + Rust shell)
pnpm dev            # frontend only (no Tauri backend; IPC calls fail)
pnpm build          # vue-tsc type check + vite build -> dist/
pnpm tauri build    # production bundle -> src-tauri/target/release/bundle/
```

Rust backend (from `src-tauri/`):

```bash
cargo check          # fast type check
cargo build          # debug build
cargo clippy         # lint (zero warnings expected for delivery)
```

Sign in to platforms in Settings after the first launch. Tap the version
number **7 times** to unlock developer mode and the `Debug` page
(platform probes / live logs / crash reports / build info).

---

### Release Build

- Local: `pnpm tauri build`; bundles land in
  `src-tauri/target/release/bundle/` (Windows: `msi/` + `nsis/`;
  macOS: `dmg/`; Linux: `deb/` + `rpm/` + `appimage/`).
- Bundles ship a minimal FFmpeg (it decodes Opus, E-AC-3 and similar
  formats): run `bash scripts/ffmpeg/build-ffmpeg.sh <target>` (the
  Windows build cross-compiles on Linux / WSL), then
  `node scripts/ffmpeg/bundle-config.mjs <target>`, and pass
  `--config .cache/ffmpeg-build/<target>/tauri.bundle.json` when
  building. Without it the bundle has no FFmpeg and the frontend avoids
  those formats. See `src-tauri/native/ffmpeg/README.md`.
- CI: pushing a `v*` tag triggers `.github/workflows/release.yml`,
  building on Windows x64 / macOS arm64 / macOS x64 / Linux x64 and
  publishing a GitHub Release with the FFmpeg source attached; macOS
  bundles are ad-hoc signed (file names carry an `-adhoc` suffix).
- Pushes to main trigger the `Artifacts` workflow producing the same
  matrix for testing.

Security reminders:

- Never commit cookies, tokens, passwords, or other secrets.
- Never paste full authorization data into issues/PRs.
- Full-config exports contain platform authorization and sync
  credentials — they must not be attached publicly.

---

### Project Layout

Two processes communicate over Tauri's IPC bridge; the frontend never
talks to music platforms directly.

#### Frontend (`src/`)

- `main.ts` — app entry and the route table (all pages lazy-loaded); the
  main window starts hidden and is shown after Vue mounts to avoid a
  white flash.
- `stores/` — Pinia stores are the only place IPC lives. `player.ts` is
  the playback hub (control, fade/crossfade orchestration, queue,
  progress); `settings.ts` persists all settings;
  `listenTogether/` mirrors the Rust protocol with protocol/mapper
  submodules; other stores cover auth / library / likedSongs / history /
  playbackStats / download / recommend / search / sync / toast /
  lyricOffset.
- `views/` — route-level pages: Home / Explore / Library / Settings /
  Downloads / Recent / PlaybackStats / Debug, plus per-platform
  playlist, album, and artist detail pages.
- `components/` — `NowPlaying`, `LyricsView` (AMLL), `MiniPlayer`,
  `QueuePanel`, `SideNav`, `TitleBar` (custom window controls),
  `HyperBackground` (WebGL fluid background), `CoverBlurBackground`,
  `ListenTogetherPanel`, `TrackSelectionToolbar`, `LocateTrackFab`,
  `WaveformSlider`, `StorageManagementDialog`; `ui/` holds Material 3
  primitives (M3Dialog / M3Input / ContextMenu / CustomSelect, etc.).
- `modules/` — pure-logic modules by domain: `playback/` (queue,
  requests, policy, prefetch, state, stats tracking), `lyrics/`
  (format, cache, offsets, karaoke lines, sync payload), `library/`,
  `shortcuts/` (global shortcuts), `youtube/`. Most have matching
  `scripts/test-*.mjs` pure-logic tests.
- `shaders/` — `hyperBackground.vert/.frag` imported as strings (see
  `shaders.d.ts`).
- `i18n/` — `zh-CN` (fallback) / `zh-TW` / `en` / `ja`.
- `utils/`, `composables/` — cover caching, color extraction, logging
  (with sanitizer), persistent cache, spring animation, multi-select /
  pointer reorder / locate-current-track composables.

#### Backend (`src-tauri/src/`)

- `main.rs` — the single `generate_handler![...]` registration point,
  event ticker, media-key channel, single-instance plugin, and runtime
  decoration removal (macOS keeps the native Overlay title bar).
- `state.rs` — `AppState`: player engine, play queue, a rebuildable
  shared `reqwest::Client` (proxy-bypass toggle), shared cookie jar,
  per-platform auth, the Listen Together session, and the download-task
  registry; `TrackInfo` / `TrackSource` are the canonical track types
  across the IPC boundary.
- `commands/*_cmd.rs` — command implementations grouped by domain:
  player / library / search / lyrics / settings / auth / recommend /
  sync / download / listen_together / stats / storage / image / debug.
- `commands/download_cmd.rs`, `commands/download_metadata.rs` —
  `stores/download.ts` and `modules/download/downloadQueue.ts` schedule tasks;
  `DownloadsView.vue` exposes queued/resolving/transferring/metadata-processing
  progress, cancellation and failed-task retries in the Downloads tab.
  Concurrency is clamped to 1-8, default 6. Quality follows playback by default
  or uses independent per-platform download settings. Audio stays in the chosen
  root, `Lyrics/` holds lyrics/translations/romanization, `Covers/` holds covers,
  and `.tmp/` holds temporary files. `<audio filename>.npmeta.json` stores NP
  metadata and asset references. Metadata completion defaults on; standardized
  lyric embedding defaults off and retains original text in sidecars/NP metadata.
  The manifest records the final file size after tagging to avoid false corruption
  reports caused by tag writes.
- `commands/local_files_cmd.rs`, `library/local_file_tags.rs` —
  `LocalFilesView.vue` and `stores/library.ts` provide cancellable scan previews,
  search, multi-select and filters for existing playlist files/duplicate metadata.
  Scanning does not write playlists; users manually import into existing or new
  playlists. Title/artist/album edits are prepared on a copy, then atomically
  replace the original after verifying that lyrics, pictures and other tags
  survive. Unmodified NP sidecar fields are retained. Editing a downloaded file
  also updates its manifest entry and emits `downloads-changed`; failed commits
  roll back. Focused tests: `node scripts/test-local-scan-preview.mjs`,
  `node scripts/test-local-scan-store.mjs`, and
  `cargo test --manifest-path src-tauri/Cargo.toml --lib library::local_file_tags::tests`.
- `audio/` — `player.rs` (`PlayerEngine`: play/seek/fades/crossfades),
  `queue.rs` (shuffle/repeat), `effects.rs` (5-band EQ, loudness
  normalization, loudness gain), `growing.rs` (progressive buffering),
  `remote.rs` (Range streaming), `analyzer.rs` (audio level),
  `media_session.rs` (SMTC/MPRIS).
- `api/` — `netease/` (`crypto.rs`: WEAPI/EAPI/linuxapi), `qq/`,
  `bilibili/` (`wbi.rs`: WBI signing), `youtube/`
  (session/playback/refresh), `lrclib.rs`, `transport.rs`.
- `auth/` — cookie storage, auth state models, `youtube_hash.rs`
  (SAPISIDHASH).
- `sync/` — `models.rs` (payloads), `proto_models.rs` (ProtoBuf models
  tag-aligned with Android), `merge.rs` (three-way merge),
  `serializer.rs` (legacy JSON / GZIP), `archive/` (V4 manifests and chunks),
  `cloud.rs` (publication), `github_api.rs`, `webdav_archive.rs`,
  `webdav_gc.rs`, and `manager.rs`.
- `listen_together/` — `protocol.rs` (events and models), `session.rs`,
  `ws_client.rs`.
- `library/` (local scanning, playlist storage), `lyrics/` (multi-source
  lyric manager and parser), `settings/`, `stats/`, `logging/`.
- `security.rs` — credential storage: app-side encrypted files in
  Release, plaintext files at random paths in Debug; legacy keychain
  entries migrate on first read.
- `fsutil.rs` — atomic-write helper (temp file + rename); all local data
  persistence must go through it.

---

### IPC Contract

- Adding a command touches three places: the implementation in
  `commands/*_cmd.rs`, registration in `main.rs`
  `generate_handler![...]`, and the frontend caller. The registration
  list is the single source of truth for the IPC surface.
- Frontend calls `invoke('command_name', { camelCaseArgs })`; Tauri maps
  camelCase JS args to snake_case Rust params. Keep payload field naming
  consistent with existing conventions (structs mostly use
  `#[serde(rename_all = "camelCase")]`) — mismatches drop fields
  silently, which has bitten playback-stats payloads before.
- Backend-to-frontend communication is events only (`app.emit(...)`);
  never make the frontend poll. See README "Processes and IPC" for the
  current event list.

---

### Quality Guardrails

Protect these paths before submitting:

- **Player commands must not block the IPC thread**: every operation in
  `player_cmd.rs` that touches the `PlayerEngine` lock must go through
  `run_player_blocking`. Taking the lock directly inside an async
  command has caused 30-second UI freezes.
- **Playback generations and seek adoption**: playback requests and seek
  results carry a generation; changes to the playback path must never
  let a stale request overwrite a newer one.
- **Signing/crypto must match exactly**: WEAPI / EAPI / linuxapi, WBI,
  and SAPISIDHASH are the fragile parts of platform requests — match
  the existing scheme precisely.
- **Sync compatibility is a hard constraint**: `proto_models.rs` tags
  align one-to-one with Android `SyncDataModels.kt` `@ProtoNumber`s —
  **changing a tag breaks cross-device sync**; `merge.rs` is a
  three-way merge and must not degrade to last-write-wins; remote
  snapshots must tolerate missing fields from older versions.
- **Listen Together protocol compatibility**: protocol changes must stay
  compatible with both the Android client and the Worker server;
  nicknames allow Chinese characters, letters, and digits
  (**no hyphens** — the server sanitizes them); never expose stream
  URLs when `shareAudioLinks=false`.
- **Atomic writes for local data**: all JSON persistence goes through
  `fsutil.rs`; never `fs::write` over user data directly.
- **UI state changes must animate**: a hard UX requirement of this
  project — any visible state switch (pages, panels, covers, theme
  colors) needs a transition, no hard cuts; cover loading uses a
  peek-first pattern (probe the cache before rendering) to avoid
  placeholder flashes.
- **Credentials and logs**: credentials only go through `security.rs`;
  log output passes the sanitizer — never log cookies, tokens, or
  signed URL parameters.

---

### Extension Paths

#### 1. Adding an IPC command

1. Implement `#[tauri::command]` in the matching
   `src-tauri/src/commands/*_cmd.rs` (create the file and export it in
   `commands/mod.rs` for a new domain).
2. Register it in `main.rs` `generate_handler![...]`.
3. Wrap the `invoke` call in the corresponding Pinia store — components
   never call `invoke` directly.
4. Align payload types on both sides (camelCase ↔ snake_case).

#### 2. Adding a streaming/search platform

1. Model on `api/bilibili/` or `api/youtube/`: create
   `api/<platform>/client.rs` with signing/crypto isolated in its own
   file.
2. Extend auth state in `auth/state.rs`; reuse the WebView login flow in
   `auth_cmd.rs` for cookie capture.
3. Wire search into the platform dispatch in `search_cmd.rs`; wire
   streaming into the `get_*_url` family in `settings_cmd.rs`.
4. Add the platform tab and route pages in Explore / Library.
5. Keep downloads, lyrics, covers, and stats mappings cleanly bounded.

#### 3. Adding a setting

1. Register the key, type, and default in `src/stores/settings.ts`.
2. Put the UI in the matching section of `SettingsView.vue`.
3. Settings that affect backend behavior go through existing commands
   (like `set_bypass_proxy`) — don't invent a second persistence
   channel.
4. Mind the `formatVersion` migration logic; old configs must upgrade
   losslessly.

#### 4. Changing the lyrics pipeline

1. Source priority lives in `src-tauri/src/lyrics/manager.rs`:
   QQ (with song_mid) → NetEase (with id) → LRCLIB → title/artist/
   duration search fallback; keep duration-error scoring intact.
2. Frontend parsing and caching live in `src/modules/lyrics/`; matched
   lyrics from the sync payload take priority over re-fetching.
3. Keep YRC word-synced structures and translation lines compatible with
   the AMLL rendering layer.
4. Related tests: `pnpm test:lyrics-format`, `pnpm test:lyric-offset`,
   `pnpm test:lyrics-request`.

#### 5. Changing GitHub / WebDAV sync

1. Understand `sync/models.rs` and `sync/proto_models.rs` first;
   **ProtoBuf tags are append-only**, and new fields must tolerate
   missing-field payloads from old JSON and ProtoBuf data.
2. Merge logic is in `sync/merge.rs` (three-way + causal tokens). When
   changing delete/restore semantics, self-test the alternating
   scenario: desktop writes → Android reads → Android writes → desktop
   reads.
3. Credentials go through `security.rs`, never back into plaintext
   config.
4. V4 migration approval is bound to the backend, target, credential fingerprint,
   and current remote content. GitHub uses conditional HEAD updates; WebDAV V4
   manifests require strong ETags or verified finite exclusive collection leases.
   Legacy single-file migration requires a finite collection lease. Redirects, unconditional writes,
   and local object caches must not replace remote closure validation.
5. Run `pnpm test:sync-protocol-upgrade` and Rust `sync` tests. Run the Kotlin
   reverse decoder with `./scripts/test-sync-android-interop.ps1 -AndroidRoot <AndroidRepo> -ExportFixtures`.
   This does not replace full Gradle, provider, or device tests.

#### 6. Changing Listen Together

1. Client logic lives in `src-tauri/src/listen_together/` and
   `src/stores/listenTogether/`; both protocol models must change
   together.
2. Server constraints (6-char room codes, 1-24-char hyphen-free
   nicknames, event semantics) are defined by the Worker — don't fix
   only the UI validation.
3. Related tests: `pnpm test:listen-together-mapper` and
   `node scripts/test-listen-together-protocol.mjs`.

---

### Testing & PR

Correctness gates for this repo:

1. Frontend type check + build (required):
   ```bash
   pnpm build
   ```
2. Backend type check (required) and lint (recommended, zero warnings):
   ```bash
   cargo check --manifest-path src-tauri/Cargo.toml --locked
   cargo clippy --manifest-path src-tauri/Cargo.toml
   ```
3. Pure-logic test scripts (Node, no build needed; run what you touched):
   ```bash
   pnpm test:player-state
   pnpm test:playback-request
   pnpm test:playback-source
   pnpm test:track-cover
   pnpm test:lyrics-format
   pnpm test:lyric-offset
   pnpm test:lyrics-request
   pnpm test:listen-together-mapper
   pnpm test:youtube-playlist-parse
   pnpm test:bilibili-cover-cache
   node scripts/playback-queue.test.mjs
   node scripts/test-listen-together-protocol.mjs
   node scripts/test-now-playing-background.mjs
   ```
4. For behavior changes in playback, login, downloads, or sync, verify
   manually on at least one desktop platform and state the platform and
   scenarios in the PR.
5. When touching pure logic in `modules/`, add or update the matching
   `scripts/test-*.mjs`; register new scripts as `test:*` entries in
   `package.json`.

A PR should include (template at `.github/PULL_REQUEST_TEMPLATE.md`):

- Motivation and key implementation points
- Risk and compatibility impact (especially cross-device sync / Listen
  Together protocol)
- Verification commands actually run, with results
- Screenshots or recordings for UI changes

Do not commit:

- Build artifacts (`dist/`, `src-tauri/target/`), local IDE config
- Authorization cookies, tokens, full-config backups, personal data
- Anything under `.report/` or `docs-private/`

Commit messages follow Conventional Commits, e.g.
`feat(player): ...`, `fix(sync): ...`, `docs: ...`.

---

### Legal & License

- The project is for learning and research only; do not use it for
  illegal purposes.
- This project is licensed under **GPL-3.0** (GPL-3.0-or-later); by
  contributing you agree to distribute your changes under
  GPL-3.0-or-later. Code contributed under MIT before 2026-10-07 keeps its
  original notice in `LICENSES/MIT.txt`.
- The `vendor/applemusic-like-lyrics` submodule is AGPL-3.0 and follows its
  own license.
- The Android repository is GPL-3.0 as well. When porting an Android
  implementation, keep its copyright notices and name the source in the
  commit body.

---

### Communication

- [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues): bugs,
  feature requests, discussion
- [README_EN.md](./README_EN.md): features and usage
- [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md): community standards

For larger structural changes (especially anything touching the sync
data model or the Listen Together protocol), open an issue first to
align on direction.

