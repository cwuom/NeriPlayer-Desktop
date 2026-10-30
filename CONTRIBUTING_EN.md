[English](./CONTRIBUTING_EN.md) | [中文](./CONTRIBUTING.md)

## Contributing to NeriPlayer Desktop

Thank you for contributing to NeriPlayer Desktop.
This guide describes **the current desktop implementation**. Keep it aligned with source code and runtime behavior.
The project is in preview; use recoverable copies of your data for development and testing.

---

### Scope

- NeriPlayer Desktop is a desktop audio player built with **Tauri 2, Vue 3, and Rust**,
  targeting Windows, macOS, and Linux.
- Online integrations include NetEase Cloud Music, QQ Music, Bilibili, and YouTube Music.
  Account, search, stream resolution, and metadata capabilities differ between sources.
- The Rust backend handles audio and native operations; the Vue WebView handles UI and state orchestration.
  Browser previews cannot replace desktop runtime validation.
- Data is local by default. Optional GitHub / WebDAV sync transfers playlist, favorite, history,
  statistics, and other metadata, without uploading audio files.
- Cloud sync and Listen Together maintain Android compatibility. The Listen Together server lives at
  `np-submodule/NeriPlayer-LTW` in the [Android repository](https://github.com/cwuom/NeriPlayer),
  outside this repository.

---

### Documentation Map

Use the documents according to their purpose:

- [README_EN.md](README_EN.md) / [README.md](README.md)
  - Project scope, capabilities, installation, sync, and privacy for users and new contributors.
- [CONTRIBUTING_EN.md](CONTRIBUTING_EN.md) / [CONTRIBUTING.md](CONTRIBUTING.md)
  - Development environment, modules, extension paths, validation, and contribution requirements.
- [src-tauri/native/ffmpeg/README.md](src-tauri/native/ffmpeg/README.md)
  - Shared libraries, runtime loading, bundle configuration, decoder testing, and distribution licensing.
- [src-tauri/native/dsp/README.md](src-tauri/native/dsp/README.md)
  - Android DSP provenance, desktop bridging, parameter compatibility, and native tests.
- [src-tauri/src/sync/archive/fixtures/README.md](src-tauri/src/sync/archive/fixtures/README.md)
  - Sync archive fixtures and their validation scope.
- [.github/workflows/](.github/workflows/), [PR template](.github/PULL_REQUEST_TEMPLATE.md)
  - Actual CI, test package, release entry points, and review information.
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), [LICENSE](LICENSE), [LICENSES/MIT.txt](LICENSES/MIT.txt)
  - Community standards, current licensing, and the historical MIT notice.

Frontend versions and commands come from `package.json` / `pnpm-lock.yaml`; Rust toolchain and dependencies
come from `rust-toolchain.toml`, `src-tauri/Cargo.toml`, and `Cargo.lock`.
Window, security, resource, and packaging configuration lives in `src-tauri/tauri.conf.json`
and `src-tauri/capabilities/`.

Update both READMEs for user-visible behavior changes. Update both CONTRIBUTING files when development,
module boundaries, protocols, tests, or releases change. Do not describe planned capabilities as implemented.

---

### Development Environment

- **Git**: required for the submodule and build revision metadata.
- **Node.js**: repository CI uses 20; `package.json` does not declare `engines`.
  Use the same major version to reproduce CI and record your environment when validating another version.
- **pnpm**: 9.15.9, pinned by `packageManager`.
- **Rust**: 1.95.0, pinned by `rust-toolchain.toml` with the `minimal` profile.
- **Frontend**: Vue 3, Vite 6, and TypeScript 5, installed through project dependencies.
- **Native build**: Tauri 2, a C++17 compiler, and platform development libraries.
  A global Tauri CLI and system FFmpeg development package are not required for compilation.

Follow the [official Tauri prerequisites](https://v2.tauri.app/start/prerequisites/):

- Windows requires an MSVC Rust toolchain, Microsoft C++ Build Tools with desktop C++ development,
  and WebView2.
- macOS desktop development requires Xcode Command Line Tools.
- Linux requires WebKitGTK 4.1, GTK, OpenSSL, tray, and ALSA development libraries.
  For Ubuntu 22.04, use the example below; map package names using the official guide on other distributions:

  ```bash
  sudo apt-get update
  sudo apt-get install -y build-essential pkg-config curl wget file libssl-dev \
    libxdo-dev libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev \
    patchelf libasound2-dev
  ```

RPM packaging also needs `rpm`. See the FFmpeg guide for its additional tools and target restrictions.
Windows MSI packaging also requires the VBScript optional feature; see the Tauri guide above for troubleshooting.
Install optional Clippy / rustfmt components, which are not included in the `minimal` profile, as needed:

```bash
rustup component add clippy rustfmt
```

---

### Quality Guardrails

Protect these flows when making changes:

- **Playback and asynchronous results**: preserve generation checks for playback requests, seek,
  prefetch, stream caches, and audio information. Old results must not overwrite a newer track.
  Keep expensive engine-locking operations on the existing blocking workers.
- **Downloads and files**: retain cancellation, retry, sidecars, final file size, catalog consistency,
  and recovery. Tag edits preserve copy/write/read-back/replace behavior, lyrics, covers, and NP metadata.
- **Local data**: preserve SQLite transactions, legacy JSON / localStorage migration, startup snapshots,
  and serialized writes. Settings, credentials, media, and folder indexes have separate storage boundaries
  and must not be removed as ordinary cache.
- **Platform APIs**: preserve platform-specific requests, signatures, authentication, and client selection.
  Mock tests do not establish real login, search, or stream availability.
- **Lyrics and UI**: preserve matches, translation / romanization, word timing, offsets, seeking,
  and window lifecycles. Reuse menu, shortcut, animation, cover-cache, and event cleanup patterns.
- **Audio and system integration**: preserve decoder fallback, output devices, DSP parameters,
  and realtime callback constraints. Tray, media keys, desktop lyrics, multiple windows,
  and IME input need native validation.
- **Sync and Listen Together**: preserve ProtoBuf field numbers, three-way merge, causal tokens,
  deletion records, remote concurrency protection, room versions, and reconnection.
  Cross-platform compatibility requires separate acceptance testing.
- **Credentials and diagnostics**: use `security.rs`; never log cookies, tokens, sync passwords,
  or signed URLs. Do not hide failures by ignoring errors, disabling rules, or removing valid tests.

Frontend regressions live in `scripts/`; Rust tests live with their modules.
Start with nearby tests and add coverage that reproduces the issue. Match validation scope to change risk.

---

### Quick Start

1. Clone with the submodule:

   ```bash
   git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
   cd NeriPlayer-Desktop
   ```

   For an existing clone without its submodule, run `git submodule update --init --recursive`.
   Vite references the core source in `vendor/applemusic-like-lyrics` directly; missing files break builds.

2. Install dependencies with the pinned pnpm version:

   ```bash
   pnpm install --frozen-lockfile
   ```

   If Corepack is available, run `corepack enable` first; otherwise prepare that pnpm version
   following its official instructions.

3. Start the native development app:

   ```bash
   pnpm tauri dev
   ```

4. Click the **version number** in Settings seven times to enable developer mode.
   Open `Debug` for probes, logs, and build information.

Distinguish the commands by what they validate:

| Command | Purpose |
| --- | --- |
| `pnpm tauri dev` | Start Vite and the Rust app to exercise full desktop flows |
| `pnpm dev` | Browser frontend only; no native IPC for native playback, login, files, or system integration |
| `pnpm build` | Run `vue-tsc --build` and Vite, producing `dist/` |
| `pnpm preview` | Preview the built frontend without a native backend |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | Check Rust; platform dependencies and a C++ compiler are still required |

`pnpm tauri` runs through `scripts/run-tauri.mjs`. Development defaults to port 1420;
when occupied, it finds an available port in 1420–1469 and updates both Vite and Tauri addresses.
`NERI_DEV_PORT` selects a specific port, which must be available. Direct `pnpm dev` uses Vite's strict
port configuration and does not search for another port.

---

### Release Build

Run from the repository root:

```bash
pnpm tauri build -- --locked
```

This builds the native application and installers, but **does not automatically download or bundle
FFmpeg shared libraries**. Compiling the C++ shim only needs repository headers;
actual FFmpeg decoding requires matching libraries at runtime. Missing libraries trigger an attempt
to use Symphonia, without guaranteeing every format remains playable.

To distribute the full decoder package:

1. Build libraries using the [FFmpeg guide](src-tauri/native/ffmpeg/README.md),
   or obtain matching `ffmpeg-<target>` artifacts from trusted workflows in this repository.
2. Place the four libraries, `LICENSE.txt`, and `BUILDINFO.txt` in `.cache/ffmpeg-build/<target>/`.
   Do not mix architectures or library sources.
3. Generate resource configuration and pass it to Tauri. For Windows x64:

   ```powershell
   node scripts/ffmpeg/bundle-config.mjs windows-x86_64
   pnpm tauri build --config .cache/ffmpeg-build/windows-x86_64/tauri.bundle.json -- --locked
   ```

The generator checks `BUILDINFO.txt` and library hashes. `NERI_FFMPEG_DIR` only controls runtime lookup;
it does not add files to installers. See the FFmpeg guide for full Linux / macOS commands.

| Platform | Rust target | FFmpeg target | Release assets |
| --- | --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` | `windows-x86_64` | NSIS `.exe`, `.msi` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `macos-aarch64` | `.dmg`, minimum system setting 11.0 |
| macOS Intel | `x86_64-apple-darwin` | `macos-x86_64` | `.dmg`, minimum system setting 10.15 |
| Linux x64 | `x86_64-unknown-linux-gnu` | `linux-x86_64` | `.deb`, `.rpm`, `.AppImage.tar.gz` |

Without `--target`, bundles default to `src-tauri/target/release/bundle/`.
Explicit targets use `src-tauri/target/<RustTarget>/release/bundle/`; `CARGO_TARGET_DIR` changes the root.
Linux builds `.AppImage` locally; workflows compress it as `.tar.gz` for upload.

macOS uses ad-hoc signing without Apple notarization. Release checks dylib architectures and signatures,
and adds `-adhoc` to DMG names. The base Tauri minimum system setting is 10.15;
local arm64 release builds should mirror Release by setting `MACOSX_DEPLOYMENT_TARGET=11.0`
and overriding `bundle.macOS.minimumSystemVersion`. The base setting is not an arm64 support promise.

Settings displays `<7-characterGitHash>.<MMddHHmm>` using UTC+8, alongside a build UUID and timestamp.
`scripts/run-tauri.mjs` supplies `NERI_BUILD_EPOCH`; CI shares Git SHA, epoch, and UUID to correlate
platform packages. This does not establish byte-for-byte reproducible binaries.
The `1.0.0` in package / Cargo metadata does not mean the project has left preview.

---

### Project Layout

#### Frontend and native boundaries

- `src/main.ts`, `src/App.vue`, `src/views/`
  - App startup, routing, and pages. Desktop lyrics and tray popups mount through separate window entry points.
- `src/stores/`, `src/modules/`
  - Pinia state and domain logic for playback, libraries, downloads, lyrics, settings, sync, and Listen Together.
  - `modules/persistence/userData.ts` loads snapshots before store creation and serializes persistence writes.
- `src/components/`, `src/composables/`, `src/utils/`
  - Player, AMLL lyrics, menus, shared interactions, and native adapters; translations live in `src/i18n/`.
- `src-tauri/src/main.rs`, `lib.rs`, `state.rs`
  - Tauri application and shared runtime state, command registration, windows, tray, plugins, and native events.
- `src-tauri/src/commands/`
  - Domain IPC entry points for playback, accounts, downloads, local files, storage, sync, and diagnostics.
- `src-tauri/src/audio/`
  - CPAL output, Symphonia / FFmpeg decoding, Range / HLS, buffering, queues, DSP, speed, and media sessions.
- `src-tauri/src/api/`, `auth/`, `lyrics/`
  - Platform requests, authentication, metadata, and lyric parsing / matching.
    Do not duplicate platform signatures or credential logic in the frontend.
- `src-tauri/src/library/`, `stats/`, `db/`, `settings/`
  - Local scans and tags, playlists, statistics, database, settings, and migration.
- `src-tauri/src/sync/`, `src/stores/listenTogether/`, `src-tauri/src/listen_together/`
  - Cloud models, transport, and merge; Listen Together protocol, sessions, queues, and reconnection.
- `src-tauri/src/security.rs`, `logging/`, `fsutil.rs`
  - Credentials, unified logs, and atomic file writes.
- `src-tauri/native/ffmpeg/`, `src-tauri/native/dsp/`
  - C++17 decoder-loading shim and DSP core, compiled by `src-tauri/build.rs`.
- `scripts/`, `.github/workflows/`
  - Frontend regressions, development startup, FFmpeg builds, interoperability helpers, CI, and releases.

Adding IPC requires an implementation in `commands/`, registration in `main.rs` through
`generate_handler!`, and aligned frontend command names, arguments, and return types.
Keep JavaScript arguments consistent with Rust / serde naming. Reuse existing stores / modules for shared state
and dispose event listeners with their component or scope.

#### Persistence boundaries

`src-tauri/src/db/` stores playlists, favorites, history, queue, statistics, download catalog,
and sync metadata in `neri_user_data.db`, maintaining its schema and legacy migration.
Reuse domain transactions rather than creating JSON copies of the same state;
preserve ordering between frontend startup restoration and subsequent writes.

`library/local_index.rs` stores local music folders and track indexes separately in `local-library.json`.
It reuses unchanged tracks by file size and modification time, coalesces native notify events into
incremental scans, and updates indexes on startup. Folder management, direct playback, and playlist import
are separate operations; removing a folder must not delete source files or implicitly modify playlists.

Settings, credentials, caches, audio, and exports do not all belong to SQLite.
Confirm actual locations and ownership before changing storage or cleanup;
reuse `fsutil.rs` and existing atomic replacement procedures for file writes.

---

### Current Boundaries

- **Desktop and browser runtimes differ**: frontend build success establishes type checking and asset
  generation, without establishing native IPC, WebView, login windows, audio devices, or installer behavior.
- **Platform features depend on external services**: authentication, APIs, region, and subscription
  restrictions vary. Success with one source does not establish success with all sources;
  UI mocks cannot replace real request validation.
- **Decoding depends on distributed files**: bundle configuration tests, actual library decoding,
  and installed application testing are separate. Loading from a development cache does not establish
  that Release includes libraries. macOS Intel also has different CI decoder coverage from other targets.
- **Sync covers metadata**: a local path does not make media accessible on another device.
  Cloud sync is not an audio backup, and full configuration export is not a complete database / media backup.
- **V4 has a client upgrade boundary**: current archives use V4 while retaining the publication filename
  `neriplayer-sync-v3.manifest`. Preserve legacy reads, migration approval, and complete object validation;
  do not infer the protocol version from the filename. Compare Android implementations when changing
  ProtoBuf, record kinds, deletion, or ordering semantics.
- **Listen Together spans repositories**: this repository covers the desktop client;
  Worker and Android changes require separate review. Local protocol tests do not establish
  cross-platform room or real-network acceptance.
- **Credential storage differs by build**: Release uses application-encrypted files;
  Debug uses plaintext files at randomized paths. This does not promise protection from malicious processes
  running as the same user. Full configuration exports can contain account and sync credentials and must not be public.

---

### Extension Paths

#### 1. Add a platform or search source

1. Inspect existing requests and stream entry points in `src-tauri/src/api/`, `auth/`,
   `commands/search_cmd.rs`, `commands/explore_cmd.rs`, and `commands/settings_cmd.rs`.
2. Decide whether the source provides search, metadata completion, or playback;
   retain the distinction between original track identity and the resolved playback source.
3. Update frontend source models, search state, routes, login, covers, lyrics, downloads, and statistics identity.
4. Add parser / signature regressions and separately record real platform request validation.
   Do not commit account credentials or signed URLs.

#### 2. Modify playback, queues, or audio effects

1. Trace requests through `src/stores/player.ts`, `src/modules/playback/`, and
   `src-tauri/src/commands/player_cmd.rs`, then inspect `src-tauri/src/audio/`.
2. Check cancellation, rapid track changes, seek, prefetch, fallback, resume, and audio information together.
   Preserve generation checks and source / client-dependent cache identity.
3. DSP changes also require `src/modules/audioEffects/`, `audio/dsp.rs`, `audio/dsp_settings.rs`,
   and parameter layouts / provenance in `native/dsp/`. Do not introduce configuration,
   lock waits, or heap allocations into realtime audio callbacks.
4. Beyond logic tests, verify playback, output device switching, and parameter changes in the native app.
   Record the operating system and device environment.

#### 3. Modify downloads, local music, or tags

1. Inspect `src/modules/download/`, `src/stores/download.ts`, `commands/download_cmd.rs`,
   `download_catalog.rs`, `download_metadata.rs`, and `download_recovery.rs`.
2. For scans and tags, inspect `commands/local_files_cmd.rs`, `library/local_index.rs`,
   and `library/local_file_tags.rs`. Distinguish indexes, playlist entries, and actual files.
3. Preserve cancellation, recovery, sidecars, and catalog consistency.
   Use test files to verify that failure / cancellation does not damage originals.
4. For tag writes, cover lyric, artwork, and NP metadata retention and verify by reading the result back.

#### 4. Add a setting

1. Register it in `AppSettings`, defaults, and migration rules in `src/stores/settings.ts`.
2. Update `src/views/SettingsView.vue`, search mappings in `src/modules/settings/`, and `src/i18n/` translations.
3. Reuse existing IPC and `src-tauri/src/settings/` for backend behavior;
   preserve `formatVersion` and legacy configuration imports.
4. Cover defaults, persistence, migration, and behavior after restart.
   Clearly indicate settings that take effect only at startup.

#### 5. Modify lyrics or desktop lyrics

1. Inspect `src-tauri/src/lyrics/manager.rs`, `src/modules/lyrics/`, `src/stores/currentLyrics.ts`,
   and `src/components/LyricsView.vue`, separating parsing, matching, and rendering responsibilities.
2. Preserve existing matches, translation / romanization, word timing, user offsets, and request generations.
   Sources do not all share one format.
3. For desktop lyrics, also inspect `src/modules/desktopLyrics/`, `src/views/DesktopLyricsView.vue`,
   and `commands/desktop_lyrics_cmd.rs`. Verify locking, click-through, dragging, closing, and restoration.
4. Regression scripts can verify timing logic; multiple-window behavior still needs testing on affected systems.

#### 6. Modify GitHub / WebDAV sync

1. Read `sync/models.rs`, `proto_models.rs`, `serializer.rs`, `archive/`, `merge.rs`, `manager.rs`,
   and `commands/sync_cmd.rs`. Record the Android reference commit.
2. Preserve ProtoBuf field numbers and V4 record kinds. New fields need default reads;
   old field numbers must not be reassigned. Preserve three-way merge, causal tokens,
   deletion records, and membership restoration semantics.
3. Bilibili per-part skip rules bind to `bvid + cid`, using `biliVideoSkipRules` and V4 kind 14.
   Clearing retains a tombstone so older clients cannot restore the rule.
4. Migration approval must match the current backend, destination, credentials, and remote contents.
   Preserve conditional HEAD updates for GitHub. WebDAV supports both finite exclusive collection leases
   and a lease-free compatibility path; retain applicable ETag conditions, manifest fingerprint checks,
   publication read-back, and complete object validation. Legacy single-file migration works with or without
   a lease. Without one, every object and the new manifest use create-only `If-None-Match: *`;
   before publication, the legacy source is read again and checked against the approved upgrade fingerprint.
   Do not bypass these concurrency and migration protections.
5. Exercise Desktop write → Android read → Android write → Desktop read, covering edits, deletion,
   restoration, and conflicts. Fixed codec fixtures do not replace provider or cross-platform runtime testing.

#### 7. Modify Listen Together

1. Inspect `src/stores/listenTogether/`, `src-tauri/src/listen_together/`,
   and `commands/listen_together_cmd.rs` together, then compare the Android client and Worker.
2. Preserve fields, role permissions, room versions, and legacy protocol fallback.
   Incremental queues use `baseRoomVersion`; repeated tracks use `stableKey + occurrence`.
3. Link sharing follows settings and platform filtering. Preserve reconnection, watchdogs,
   and candidate resource handling.
4. Beyond automated regressions, verify Desktop / Android rooms, joining / leaving,
   reconnecting, track changes, seek, and queue conflicts.

#### 8. Modify windows, shortcuts, or system integration

1. Trace lifecycles from `src/main.ts`, `src/components/TitleBar.vue`, `src/modules/shortcuts/`,
   `src/modules/tray/`, and corresponding Rust commands / `main.rs`.
2. Preserve single-instance behavior, window state, event cleanup, IME input, and global shortcut boundaries.
   Avoid unnecessary expansion of Tauri permissions.
3. Check the main window, tray popup, desktop lyrics, media keys, and exit paths in the actual WebView runtime.
   Clearly record tested and untested platforms when changing platform branches.

---

### Debugging & Logs

- Click the version number seven times in Settings to enable developer mode.
  `Debug` exposes platform probes, recent logs, crash reports, and diagnostics;
  see `src/views/DebugView.vue` and `commands/debug_cmd.rs`.
- Frontend `src/utils/logger.ts` and Rust `log` feed unified logging in `src-tauri/src/logging/`.
  Recent logs use a bounded memory buffer independently of file logging.
- Files live under `NeriPlayer/logs/` in the system data directory, with crash reports in `crashes/`.
  File logging changes require restart; log-level changes take effect immediately.
- For FFmpeg, inspect `audio-decoder` logs, directories, architecture, version, dependencies, and signatures.
  Load results are cached per process; restart the app / test process after replacing libraries or changing paths.
- Include OS / architecture, build version / UUID, reproduction steps, source, and actual results in reports.
  Manually review redaction before sharing and remove cookies, tokens, private paths, signed links,
  and other sensitive information.

#### Linux WebKitGTK / NVIDIA

`src-tauri/src/linux_graphics.rs` checks for a loaded NVIDIA proprietary driver through `/proc` or `/sys`
before GTK / WebKit initialization. NVIDIA hardware alone, or nouveau, does not trigger it.
When detected, defaults disable NVIDIA explicit sync, set the driver's in-flight frame limit to 1,
disable the DMA-BUF atlas, and use SHM shared-memory compositing. An in-flight limit is not an FPS guarantee
and does not disable the entire DMA-BUF renderer. The frontend also makes full-page route changes,
now-playing slides, and scrim transitions instant to reduce large-layer repaints.

Explicitly set NVIDIA / WebKit environment variables take priority; the compatibility layer does not overwrite them.
For example, an explicit `WEBKIT_DISABLE_DMABUF_RENDERER` prevents additional forced SHM.
Compare one option at a time and verify changes in a new process:

| Environment variable | Effect |
| --- | --- |
| `NERI_DISABLE_NVIDIA_QUIRK=1` | Skip the entire compatibility layer, including frontend transition fallback triggered by its state |
| `NERI_WEBKIT_KEEP_DMABUF=1` | Keep DMA-BUF hardware compositing without forced SHM; atlas handling remains independent |
| `NERI_WEBKIT_KEEP_DMABUF_ATLAS=1` | Do not automatically disable the DMA-BUF atlas; SHM selection remains independent |
| `NERI_NVIDIA_FRAMES_ALLOWED=N` | Override the default in-flight frame limit of 1; `0` leaves it unset |
| `NERI_NVIDIA_NO_VSYNC=1` | Explicitly disable vsync, potentially causing tearing; off by default |
| `NERI_WEBKIT_DMABUF_NO_GBM=1` | Request bypassing NVIDIA GBM allocation; off by default |
| `NERI_WEBKIT_FORCE_VBLANK_TIMER=1` | Request timer-based vblank; off by default |
| `NERI_WEBKIT_SOFTWARE_COMPOSITING=1` | Disable accelerated compositing as a troubleshooting fallback, potentially reducing performance substantially |

In a Linux shell, set an option only for this development launch, for example:

```bash
NERI_WEBKIT_KEEP_DMABUF=1 pnpm tauri dev
```

Troubleshooting does not require global system configuration changes. Record the driver, desktop session,
WebKitGTK version, actual environment variables, and `linux-graphics` logs.
These compatibility strategies do not replace runtime UI validation on the target system.

---

### Testing & PR

1. Read related source, documentation, and tests; inspect `git status` and preserve existing uncommitted work.
2. Keep changes focused and add regressions that reproduce behavior bugs.
3. Start with targeted checks, then run type checks, Rust checks, native acceptance,
   and packaging as required by impact.
4. Inspect the full diff. Introduce no fixable warnings, build artifacts, secrets, private data,
   or unrelated dependency upgrades.
5. Follow the [PR template](.github/PULL_REQUEST_TEMPLATE.md) strictly, recording actual commands,
   results, unverified checks, and reasons. Include screenshots / recordings for UI changes.
   Use English Conventional Commits for commit messages and PR titles,
   such as `fix(player): preserve track identity during fallback`.

Common checks:

```bash
pnpm test
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml --locked
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --locked
```

`pnpm test` uses `scripts/test-frontend.mjs` to enumerate and sequentially run every `test:*` in `package.json`.
These scripts cover pure logic, stores, component / IPC mocks, and source constraints;
they are not browser end-to-end tests. `test:config-backup-codec` also runs the actual Rust configuration
codec and independent Node interoperability checks using the pinned Rust toolchain, so `pnpm test`
requires Rust. Its working directory is under ignored `docs/tmp/` and is removed when the test exits.
Register new regressions in `package.json` using nearby test patterns.
Examples of targeted checks:

| Area | Example commands |
| --- | --- |
| Playback, queues, resume | `pnpm test:playback-request`, `pnpm test:playback-queue`, `pnpm test:long-form-progress` |
| Downloads, scanning, persistence | `pnpm test:download-cancellation`, `pnpm test:local-scan-store`, `pnpm test:user-data-persistence` |
| Lyrics and desktop lyrics | `pnpm test:lyrics-request`, `pnpm test:lyric-offset-store`, `pnpm test:desktop-lyrics` |
| Audio effects | `pnpm test:audio-effects-model`, `pnpm test:audio-effects-store`, `pnpm test:audio-effects-ui` |
| Sync and per-part rules | `pnpm test:sync-protocol-upgrade`, `pnpm test:history-sync`, `pnpm test:bili-video-skip` |
| Listen Together | `pnpm test:listen-together-protocol`, `pnpm test:listen-together-queue`, `pnpm test:listen-together-reconnect` |
| Native UI and bundle configuration | `pnpm test:tray-popup`, `pnpm test:title-bar-window-state`, `pnpm test:ffmpeg-bundle` |

See `package.json` for the full list. Rust supports module filters:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib sync
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib listen_together
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib library::local_file_tags::tests
```

FFmpeg configuration tests use mock libraries; actual decoder tests require matching shared libraries.
Without `NERI_REQUIRE_FFMPEG=1`, some tests skip when libraries are missing.
Set `NERI_FFMPEG_DIR` and that variable as described in the [FFmpeg guide](src-tauri/native/ffmpeg/README.md),
then separately verify packaged resources and playback after installation.

Android JVM reverse-decoding helper:

```powershell
./scripts/test-sync-android-interop.ps1 -AndroidRoot <AndroidRepositoryPath> -ExportFixtures
```

The script requires PowerShell 7, Python, Java, Android sources, and existing Gradle caches for its
specified Kotlin / kotlinx.serialization versions. It also calls `zstdDecompressSync` from `node:zlib`
directly, so use a Node runtime that provides this API, such as Node 22.15.0 or later in the 22.x series.
The main project's CI Node 20 cannot run this helper step. See the addition version in the
[official Node documentation](https://nodejs.org/download/release/v22.16.0/docs/api/zlib.html#zlibzstddecompresssyncbuffer-options).
Select the Node executable with `-NodePath` if needed. Check that same executable first;
the result should be `function`:

```powershell
node -p "typeof require('node:zlib').zstdDecompressSync"
```

The script reads Android sources / caches and retains audit artifacts,
without installing missing dependencies or accessing cloud services. It does not replace a full Android Gradle build,
provider tests, or device interoperability. Prefer emulators for cross-platform testing;
obtain explicit authorization before operating real phones or other physical devices.

Current workflow boundaries:

- **CI / `build.yml`**: main / master pushes, PRs, or manual dispatch. Runs frontend regressions / build,
  locked Rust checks for four targets, and bundled FFmpeg decoder checks on Windows x64,
  macOS arm64, and Linux x64. It does not run the full Rust unit suite or Clippy.
- **Artifacts / `artifacts.yml`**: main / master pushes or manual dispatch. Produces four-target test packages,
  retaining installers for 3 days and metadata for 14 days, without publishing a GitHub Release.
- **FFmpeg / `ffmpeg.yml`**: manual or reusable workflow producing four `ffmpeg-<target>` artifacts.
- **Release / `release.yml`**: `v*` tags or manual `target_ref`, building and inspecting packages independently.
  Manual runs generate a build-version tag; tag-triggered releases with hyphenated tags are marked prerelease.
  Attachments include FFmpeg source, build scripts, macOS installation notes, and `SHA256SUMS.txt`.

CI and Artifacts ignore documentation-only, license, and template changes; missing runs do not mean checks passed.
GitHub's prerelease flag is separate from the project's preview status.
Workflow package inspection does not replace native acceptance after installation.

---

### Legal & License

- The project is for learning and research. Follow applicable laws and source-platform rules.
  The project and maintainers accept no sponsorships, donations, or commercial funding.
- The current project and contributions use **GPL-3.0-or-later**; see [LICENSE](LICENSE).
  Code released under MIT before 2026-10-07 retains the [original notice](LICENSES/MIT.txt).
- `vendor/applemusic-like-lyrics` is distributed under its own AGPL-3.0 license.
  FFmpeg libraries are built without GPL / nonfree components and distributed under LGPL-2.1-or-later;
  preserve licensing, build information, corresponding source, and build scripts.
- See [native/dsp](src-tauri/native/dsp/README.md) for Android DSP provenance and its GPL-3.0 text.
  Desktop DSP does not use the alternative license offered for Android USB paths.
- Retain copyright and license notices when porting Android or other code;
  identify sources and reference revisions in the commit body. Never commit or publish cookies, tokens,
  passwords, full configuration exports, private libraries, databases, or sensitive logs.

---

### Communication

Use [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues) for bugs and feature suggestions.
Include reproducible steps, build information, and redacted evidence, and follow the
[Code of Conduct](CODE_OF_CONDUCT.md). Discuss platform integrations, sync models,
Listen Together protocols, or major architecture changes in an issue first to align scope and compatibility.

