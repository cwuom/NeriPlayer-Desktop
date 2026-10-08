[English](./README_EN.md) | [中文](./README.md)

<h1 align="center">NeriPlayer Desktop</h1>

<div align="center">

<h3>✨ Multi-source streaming, local control, rich lyrics, and self-hosted sync — on Windows / macOS / Linux 🎵</h3>

<p>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/cwuom/NeriPlayer-Desktop/total?style=social" />
  </a>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Release" src="https://img.shields.io/github/v/release/cwuom/NeriPlayer-Desktop?include_prereleases&label=Release" />
  </a>
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-desktop-4C8BF5" />
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml">
    <img alt="CI" src="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml/badge.svg" />
  </a>
  <a href="./LICENSE">
    <img alt="License" src="https://img.shields.io/badge/License-GPL--3.0-blue" />
  </a>
  <a href="https://t.me/ouom_pub">
    <img alt="Telegram" src="https://img.shields.io/badge/Telegram-@ouom__pub-blue" />
  </a>
</p>

<p>
  <img src="app-icon.png" width="200" alt="NeriPlayer Desktop logo" />
</p>

<p>
The project name and icon are inspired by "Kazamata Neri" from
"星空鉄道とシロの旅".
</p>

<p>
NeriPlayer Desktop is the desktop port of
<a href="https://github.com/cwuom/NeriPlayer">NeriPlayer (Android)</a>,
built with <strong>Tauri 2 + Vue 3 + Rust</strong>. It shares the same
cloud-sync and Listen Together protocols with the Android app, and focuses
on multi-source exploration, online playback, local control, and
user-owned data.
</p>

🚧 <strong>Work in progress</strong>

</div>

> [!CAUTION]
> **Project & documentation status (read this first)**
>
> - The desktop port is at an early stage of development. A significant
>   part of this document is currently **placeholder text**: the
>   described features may be unimplemented, only partially implemented,
>   or diverge from actual behavior.
> - Some implementations are not yet solid and may, in the worst case,
>   **corrupt or lose data**. Do not treat this app as the only copy of
>   your data; export and back up important playlists and configs first.
> - Maintainer capacity is limited, and this repository **may become
>   unmaintained** in the future. The desktop port is meant to be
>   community-driven — please pick up issues, send PRs, or fork and
>   carry it forward if maintenance stalls.
> - When the documentation and actual behavior disagree, the source code
>   is the truth; documentation fixes are welcome.

---

> [!WARNING]
> This project is for learning and research purposes only. Do not use it
> for illegal purposes.
>
> This project and its maintainer do not accept any form of sponsorship,
> donation, or commercial funding.

---

> [!NOTE]
> NeriPlayer Desktop does not provide a public cloud music library or media
> distribution service. Online audio capabilities depend on your
> authorization on third-party platforms. VIP or restricted content still
> follows the original platform rules.

---

## Start here

If you only want to try the app, start with
[Getting Started](#getting-started).
If you want to understand what makes the project different, read
[Why it stands out](#why-it-stands-out) and [Key Features](#key-features).
If you plan to contribute, read [CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md).
If you care about how this relates to the Android app, jump to
[Relationship with the Android app](#relationship-with-the-android-app).

```text
NeriPlayer Desktop
├── Multi-source playback: NetEase / QQ Music / Bilibili / YouTube Music + local files
├── Local-first data: playlists, favorites, recent plays, stats, downloads, settings
├── User-owned sync: GitHub / WebDAV metadata sync (interoperable with Android)
├── Rich playback: Rust audio engine, AMLL word-synced lyrics, fluid shader background
└── Listen Together: real-time rooms on the same protocol as Android
```

---

## About

NeriPlayer Desktop is a **Tauri 2** application: a **Vue 3** frontend and a
**Rust** backend communicate over the IPC bridge. The Rust side owns audio
playback, all platform network requests, request signing/crypto, the
filesystem, and cloud sync; the frontend never talks to music platforms
directly.

Current positioning:

- **Account as capability**: third-party platform authorization enables
  search, playback, playlists, and favorites. Login uses a built-in WebView
  window that captures cookies automatically (including HttpOnly); manual
  cookie import is also supported.
- **Local-first**: playlists, favorites, recent plays, playback stats,
  downloads, settings, and auth data are stored locally by default.
- **Optional sync**: playlists, favorites, recent plays, and playback stats
  can be synced to your own GitHub repository or a WebDAV remote file — in
  a format fully interoperable with the Android app.
- **Privacy and account safety first**: sync is intentionally
  decentralized; nothing is written back to third-party music platforms,
  avoiding their risk-control systems.
- **Aligned with Android**: features, interactions, sync data models, the
  Listen Together protocol, and the version-name format all track the
  Android app.

---

## Why it stands out

- **A real Rust playback engine, not a wrapper around a system player**:
  `PlayerEngine` uses **cpal** for output and **symphonia** for decoding
  (MP3 / AAC / FLAC / OGG Vorbis / WAV / PCM / ADPCM / MP4 containers),
  with three playback paths: local file, in-memory bytes, and network
  streams. Streams use progressive (growing) buffering and adaptive Range
  fetching, so playback survives slow networks; fragmented audio supports
  fast seeks and silent paused scrubbing.
- **Tunable sound**: fade on pause/resume, crossfade on track change
  (durations configurable), playback speed, loudness gain, per-track
  real-time loudness normalization, and a 5-band equalizer
  (60 / 230 / 910 / 3600 / 14000 Hz, presets + manual, ±15 dB).
- **Apple Music-style lyrics**: rendering is built on
  [applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics)
  (embedded as a submodule), with word-by-word highlighting, translated
  lyrics, lyric blur, font scaling, per-source global offsets plus a
  per-track offset; NetEase YRC word-synced lyrics are wired end to end.
- **Lyrics come from more than one source**: QQ Music → NetEase → LRCLIB,
  preferring platform track IDs and falling back to title/artist/duration
  matching with duration-error scoring.
- **GLSL fluid background**: the Now Playing page renders a WebGL
  `HyperBackground` shader driven by cover colors and the live audio
  level; cover-blur backgrounds and custom background images
  (blur/opacity adjustable) are also supported.
- **Cross-device sync is field-level interop, not just "both speak
  WebDAV"**: the Rust ProtoBuf models align tag-by-tag with the Android
  app's `SyncDataModels.kt` `@ProtoNumber`s, merging is three-way against
  a base snapshot. New writes use Android V4 ProtoBuf manifests and Zstd chunks;
  legacy JSON, GZIP, and V3 archives remain readable.
  The same remote can be read and written alternately by both apps.
- **Listen Together speaks the Android protocol**: the desktop client
  connects to the same Cloudflare Workers server, with rooms, roles,
  queue sync, repeat/shuffle sync, member-control requests, and a
  stream-link sharing switch.
- **Desktop-grade, not a webpage in a box**: frameless custom title bar
  (native unified-toolbar traffic lights on macOS), full keyboard
  shortcuts, pointer drag reordering, multi-select toolbar, context
  menus, system media keys with SMTC / MPRIS integration, and
  single-instance protection.
- **Credential storage designed for the desktop threat model**: release
  builds keep platform cookies, the GitHub token, and the WebDAV password
  in app-side encrypted files (mirroring Android's
  EncryptedSharedPreferences) instead of triggering keychain prompts for
  an unsigned app; logs are sanitized.
- **A diagnostics loop**: the Debug page ships connectivity probes, a
  live log viewer, crash-report management, and debug-report export;
  persistent file logging and log levels are configurable.

---

## Getting Started

### a. Download a Release (recommended)

1. Go to [GitHub Releases](https://github.com/cwuom/NeriPlayer-Desktop/releases)
2. Pick your installer:

| Platform | Asset | Notes |
| --- | --- | --- |
| Windows | `.msi` / `.exe` (NSIS) | x64; needs the WebView2 runtime (bundled with Win10/11) |
| macOS (Apple Silicon) | `*-adhoc.dmg` (arm64) | macOS 11.0+ |
| macOS (Intel) | `*-adhoc.dmg` (x64) | macOS 10.15+ |
| Linux | `.deb` / `.rpm` / `.AppImage.tar.gz` | x64; requires WebKitGTK 4.1 |

> [!IMPORTANT]
> macOS builds are ad-hoc signed (not notarized). If Gatekeeper blocks the
> first launch, right-click → Open in Finder, or run
> `xattr -cr /Applications/NeriPlayer.app` and start again.
> Windows installers are unsigned; a SmartScreen prompt is expected.

### b. Download a CI build

Grab the latest successful artifacts from the `Artifacts` workflow on
[GitHub Actions](https://github.com/cwuom/NeriPlayer-Desktop/actions)
(`NeriPlayer-Windows-x64 / macOS-arm64 / macOS-x64 / Linux-x64`).

### c. Build locally

Prerequisites: **Node.js 20+**, **pnpm 9.15.9** (`corepack enable`),
**Rust 1.95.0** (pinned by `rust-toolchain.toml`), plus platform deps:

- **Windows**: Visual Studio C++ Build Tools, WebView2 runtime
- **macOS**: Xcode Command Line Tools
- **Linux** (Debian/Ubuntu example):
  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
    librsvg2-dev patchelf libasound2-dev
  ```

Build steps:

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install
pnpm tauri dev      # development run (Vite :1420, or the next free port if it is taken + Rust shell)
pnpm tauri build    # production bundle -> src-tauri/target/release/bundle/
```

The repo depends on a Git submodule (`vendor/applemusic-like-lyrics`);
clone with `--recursive` or run
`git submodule update --init --recursive`.

After the first launch, sign in to platforms under Settings. Tapping the
version number **7 times** unlocks developer mode and a `Debug` page in
the sidebar.

---

## Key Features

- 🎧 **Multi-source playback**:
  NetEase Cloud Music, QQ Music, Bilibili, YouTube Music, and local audio.
- 🏠 **Home with recommendations and continue-listening**:
  recent plays, NetEase daily/curated/high-quality playlists, hot and
  radar songs, cross-platform playlist entries; with internationalization
  mode on and YouTube signed in, YouTube Music home shelves come first.
- 🔍 **Layered search**:
  per-platform search in Explore (NetEase / Bilibili / YouTube Music)
  plus Bilibili and YouTube discovery shelves; Now Playing metadata and
  lyric completion use NetEase + QQ Music with LRCLIB as an external
  source.
- 🗂️ **Library browsing by category**:
  Local / Favorites (playlists + followed artists) / Downloads / NetEase
  (playlists + albums) / Bilibili favorite folders / YouTube Music
  playlists — each tab has its own search box that survives switching.
- 🧠 **Playback core**:
  queue management, shuffle/repeat, generation-guarded playback requests,
  failure recovery, and optional progress/mode restore.
- 🌊 **Streaming**:
  progressive buffering, adaptive Range fetching, fast fragmented seeks,
  silent paused scrubbing, in-flight request dedup, and prefetch.
- 🎚️ **Audio effects**:
  playback speed, loudness gain, per-track loudness normalization, and a
  5-band EQ (presets + manual).
- ⬇️ **In-app downloads**:
  the Library Downloads tab shows queued, resolving, transferring and
  metadata-processing tasks, with live byte counts and percentages when
  available, individual/bulk cancellation, and retries for failed tasks.
  Concurrency is adjustable from Downloads or Settings: 1-8 tasks, default 6.
  Download quality follows playback by default, or can be configured per platform.
  Audio stays in the selected download directory; lyrics, translations and
  romanization go in `Lyrics/`, covers in `Covers/`, and temporary files in `.tmp/`.
  Each adjacent `<audio filename>.npmeta.json` stores NP metadata, source identity
  and sidecar references. Metadata completion is enabled by default; standardized
  lyric embedding is disabled by default. Original lyrics remain in sidecars and
  NP metadata when standardization is enabled. Filename templates, custom paths,
  corruption validation and reveal-in-file-manager are also supported.
- 🩷 **Local playlists and favorites**:
  create/rename/delete/reorder, multi-select bulk actions, pointer drag
  reordering, NetEase like/unlike, and favorites that open via their
  source-platform routes.
  The Local Files category provides directory scan previews, scan progress and
  cancellation, search, multi-select, and filters for files already in playlists
  or with duplicate metadata. Selected files are manually added to an existing
  or new playlist; scanning does not create a default Local Files playlist.
  Title, artist and album tags can be saved to the audio file after verifying
  that existing lyrics, covers and other tags survive the write.
- 🧑‍🎤 **NetEase artists**:
  artist detail with paged top songs and albums, plus an artists category
  on the favorites tab.
- 📊 **Playback stats**:
  play counts, listening time, and daily buckets keyed by stable song
  identity, with an overview view; stats participate in cloud sync.
- 🕘 **Recent plays**:
  a dedicated page; deletions sync across devices without resurrection.
- ☁️ **GitHub / WebDAV sync**:
  playlists, favorites, recent plays, and stats with three-way merging
  and Android V4 chunked archives; playlist JSON and full-config import/export.
- 🎧 **Listen Together**:
  create or join rooms with real-time WebSocket sync, member-control
  switch, auto-pause on member changes, repeat/shuffle sync, stream-link
  sharing switch, and a custom server URL.
- 🌈 **Personalization**:
  theme color and dynamic color, smooth theme transitions, custom
  background image (blur/opacity), UI display toggles, default start
  page; UI languages: 简体中文, 繁體中文, English, 日本語.
- ✨ **Now Playing effects and lyrics**:
  WebGL fluid background (audio-reactive), cover-blur background, AMLL
  word-synced lyrics, translations, lyric blur and font scale,
  per-source + per-track lyric offsets.
- 🪟 **Desktop integration**:
  system media keys with SMTC / MPRIS sessions, single instance, and a
  frameless custom title bar (native traffic lights on macOS).
- ⌨️ **Keyboard shortcuts**: see
  [Keyboard Shortcuts](#keyboard-shortcuts).
- 🧾 **Friendly login**:
  built-in WebView login windows capture cookies automatically
  (including HttpOnly); manual cookie import and status checks included.
- 🧯 **Storage management**:
  grouped usage stats and cache cleanup that never touches your
  downloads.
- 🛠️ **Developer mode and diagnostics**:
  connectivity probes, live logs, crash reports, and debug-report
  export.

---

## Platform Status

- **NetEase Cloud Music**:
  WebView login, search, daily/curated/high-quality playlists, user
  playlists and starred albums, playlist/album detail, artist detail
  (paged top songs/albums), like/unlike, multi-quality streaming, lyrics
  (including word-synced YRC and translations), downloads.
- **QQ Music**:
  search, multi-quality streaming, lyric and metadata completion; no
  account login yet, so capabilities are limited to guest APIs.
- **Bilibili**:
  WebView login, video search, created/subscribed favorite folders, DASH
  audio streaming, cover proxying (Referer handling), downloads,
  discovery shelves.
- **YouTube Music**:
  WebView login, home feed, playlist browsing and detail, search,
  streaming, downloads, account profile refresh.
- **Local audio**:
  directory scanning and import, local playlists, local artist grouping
  with detail pages.
- **LRCLIB**:
  external lyric source with duration-precise matching.

---

## Relationship with the Android app

The desktop app deliberately mirrors
[NeriPlayer (Android)](https://github.com/cwuom/NeriPlayer):

- **Sync interop**: ProtoBuf tags align one-to-one with Android's
  `SyncDataModels.kt` `@ProtoNumber`s; the same GitHub repo / WebDAV
  remote can be read and written alternately by both apps with identical
  three-way merge semantics (including deletions and membership tokens).
- **Listen Together interop**: both clients connect to the same
  Cloudflare Workers server and can share a room.
- **Interaction parity**: library categories, settings, the debug page,
  and lyric behavior are ported with Android as the reference.
- **Same version format**: `<git-short-hash>.<MMddHHmm>`.

Mobile-only capabilities of the Android app (USB exclusive output,
floating/status-bar lyrics, SAF directories, safe mode, etc.) are out of
scope on desktop. The desktop app keeps catching up elsewhere — please
file an issue when the two apps disagree.

---

## Implementation Notes

### Build and versioning

- Frontend: Vue 3.5 + Pinia + Vue Router + vue-i18n + Vite 6 +
  TypeScript 5.8
- Backend: Rust 2021 (toolchain pinned to `1.95.0`) + Tauri 2
- Package manager: pnpm `9.15.9` (pinned); CI uses Node 20
- Version name: `<git-short-hash>.<MMddHHmm>` (Asia/Taipei), injected by
  `src-tauri/build.rs` as `BUILD_UUID / BUILD_TIMESTAMP / BUILD_VERSION`
  and surfaced via the `get_build_info` command
- CI (GitHub Actions):
  - `CI`: frontend `vue-tsc + vite build`; backend
    `cargo check --locked` across Windows / macOS (arm64+x64) / Linux
  - `Artifacts`: four-platform bundles on every push to main
  - `Release`: pushing a `v*` tag builds and publishes a GitHub Release
- Bundles: Windows MSI + NSIS; macOS DMG (Intel 10.15+ / arm64 11.0+,
  ad-hoc signed); Linux deb + rpm + AppImage

### Processes and IPC

- The frontend never calls music platforms; every platform request goes
  through a Rust `#[tauri::command]`. All commands are registered in a
  single `generate_handler![...]` block in `src-tauri/src/main.rs`
  (~140 commands across player / library / search / lyrics / settings /
  auth / recommend / sync / download / listen_together / stats /
  storage / debug).
- The backend pushes state via events: `player:position`,
  `player:audio-level`, `player:track-ended`, `download-progress`,
  `playlists-changed`, `playback-stats-changed`, `lt:*`, `media:*`;
  a background ticker thread polls the player for progress/ended.
- `AppState` is the single managed state: player engine, play queue, a
  rebuildable shared `reqwest` client (proxy-bypass toggle), a shared
  cookie jar, per-platform auth, the Listen Together session, and the
  download-task registry.

### Audio engine

- `cpal` output + `symphonia` decoding, `lofty` for tags; playback paths
  cover local files, in-memory bytes, and network streams (progressive
  buffering + adaptive Range).
- Fades and crossfades are implemented in the engine; EQ, loudness
  normalization, loudness gain, and speed run in the decode pipeline.
- Playback requests carry a generation so stale requests can never
  clobber newer ones; seek results are adopted by generation, too.
- System media sessions go through `souvlaki` (SMTC on Windows, MPRIS on
  Linux, Now Playing on macOS), with media-key actions relayed to the
  frontend.

### Platform APIs and request signing

- One module per platform (`api/netease`, `api/qq`, `api/bilibili`,
  `api/youtube`) plus `api/lrclib`, with signing isolated:
  - `netease/crypto.rs` — WEAPI / EAPI / linuxapi AES + RSA
  - `bilibili/wbi.rs` — WBI mixin-key signing
  - `auth/youtube_hash.rs` — SAPISIDHASH header generation
- Login opens the platform page in a built-in WebView window and polls
  cookies (including HttpOnly), closing automatically once the session
  cookie appears.

### Cloud sync

- Synced objects: local playlists, favorite playlists, recent plays
  (with deletions), playback stats (with daily buckets and clear
  markers), playlist-song deletions, and the sync log.
- `merge.rs` performs a three-way merge against a base snapshot (never
  last-write-wins); songs carry causal membership tokens so deletions
  and restores don't cancel each other across devices.
- New cloud writes use V4 manifests and Zstd chunks; legacy JSON, GZIP, and
  V3 archives remain readable. Existing legacy archives require confirmation
  in Settings before migration; update clients on other devices first.
- The GitHub token and WebDAV password live in app-side encrypted
  storage (below), never in plaintext config.

### Credentials and local data

- Release builds store credentials (cookies, GitHub token, WebDAV
  password) in app-side encrypted files, matching the Android
  EncryptedSharedPreferences threat model; legacy keychain entries are
  migrated on first read.
- Playlists, favorites, play history, the playback queue, playback stats,
  the download catalog, sync metadata and detail/lyrics caches live in the
  SQLite user database `neri_user_data.db` (tables mirror Android's
  `NeriUserDataDatabase`; WAL with an fsync per commit). Legacy JSON files and
  WebView localStorage data are imported once per domain on first launch and
  the originals are moved to `legacy-json-backup/`. Settings, sign-in state
  and sync configuration stay in key-value stores.
- Logs are sanitized before hitting the file log (level and switch
  configurable); crash reports are stored separately.

---

## Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| `Space` | Play / pause |
| `Ctrl/Cmd + →` / `Ctrl/Cmd + ←` | Next / previous track |
| `→` / `←` | Seek forward / back (`Shift` for a larger step) |
| `↑` / `↓` | Volume up / down |
| `M` | Mute |
| `S` | Shuffle |
| `R` | Repeat mode |
| `Ctrl/Cmd + P` | Toggle Now Playing |
| `Ctrl/Cmd + F` | Search |
| `Esc` | Layered close (lyrics → Now Playing → panels) |

---

## Cloud Sync (GitHub / WebDAV)

NeriPlayer Desktop syncs local metadata to **your own GitHub repository**
or a WebDAV remote file. Configure it under Settings → Backup & Sync.

Synced objects:

- Local playlists (including custom track names/artists/covers, matched
  lyrics, and per-track lyric offsets)
- Favorite playlists
- Recent plays and their deletion records
- Playback stats (including daily buckets)

Details:

- 🔒 **Local secure storage**: the GitHub token and WebDAV password are
  kept in app-side encrypted storage.
- 🧩 **Conflict handling**: three-way merging against a base snapshot for
  playlists, favorites, history, deletions, and stats; songs carry causal
  tokens so restored content is not re-deleted by stale records.
- 🪶 **Chunked archives**: Android V4 `neriplayer-sync-v3.manifest` and Zstd
  objects; migration approval is bound to each backend's current target and content.
- 🔄 **Cross-device interop**: the same remote can be read and written
  alternately by the desktop and Android apps.
- 🚫 **Sync boundary**: audio files, downloads, cookies, and playback
  tokens are never uploaded.
- 🧪 **Alignment evidence**: `scripts/test-sync-android-interop.ps1` checks Kotlin
  decoding. Automated codec tests do not replace real-account, provider, or device tests.
- 📦 **Remote format**: GitHub repos / WebDAV files are not end-to-end
  encrypted backups; you are responsible for the remote.

Usage (GitHub):

1. Create a GitHub Personal Access Token with `repo` scope.
2. Validate the token in-app, then create a default private repository or
   attach an existing one.
3. Sync manually, or let local changes trigger a sync.

Playlist JSON import/export and full-config import/export (settings,
authorization, and sync config — for personal migration only, do not
share publicly) are also available.

---

## Listen Together

- Create or join rooms; playback, pause, progress, track changes, and
  repeat/shuffle modes sync in real time over WebSocket. Member-control
  switch, auto-pause on member changes, stream-link sharing switch, and a
  custom server URL are supported.
- The server is shared with the Android app and runs on
  **Cloudflare Workers + Durable Objects**:
  - In-repo source: the Android repo's
    [np-submodule/NeriPlayer-LTW](https://github.com/cwuom/NeriPlayer/tree/master/np-submodule/NeriPlayer-LTW)
  - Public deployment template:
    [TheSmallHanCat/NeriPlayer-LTW](https://github.com/TheSmallHanCat/NeriPlayer-LTW)
- Room codes are 6 readable characters; nicknames are 1-24 characters of
  Chinese, letters, or digits (matching the server constraints).

---

## Roadmap

### Directions

Adjusted by maintenance capacity, platform availability, and community
feedback; no fixed schedule is promised.

- [ ] QQ Music account login and library pages
- [ ] System tray and OS-level global shortcuts
- [ ] In-app auto-update channel
- [ ] Resumable downloads with queue recovery
- [ ] Continuous parity with new Android features

### Recently landed

- [x] User data moved to a SQLite database (aligned with Android Room) with automatic legacy import
- [x] NetEase artist detail page and a favorites artists category
- [x] YouTube Music home shelves first in internationalization mode
- [x] Layered ESC close, cursor-anchored menus, and menu polish
- [x] Pointer drag reordering, multi-select toolbar, stable drop indicator
- [x] Adaptive Range streaming, fast fragmented seeks, silent scrubbing
- [x] Smooth theme-color transitions and settings animations
- [x] Native unified-toolbar traffic lights on macOS
- [x] Merge semantics aligned with Android; data-loss paths closed
- [x] Atomic file persistence and corrupt-store protection
- [x] Encrypted credential storage and log redaction
- [x] Debug console aligned with the Android debug home
- [x] Single-instance protection and Linux dependency fixes
- [x] AMLL word-synced lyrics, YRC wiring, and lyric context menus
- [x] Listen Together repeat/shuffle sync and session guards

---

## Bug Report

- Enable developer mode first (tap the **version number** 7 times in
  Settings); a `Debug` page appears in the sidebar.
- The Debug page shows live logs and crash reports and can export a
  debug report; Settings → Logs enables persistent file logging.
- File an [Issue](https://github.com/cwuom/NeriPlayer-Desktop/issues)
  with your OS and version, the app version (copyable from Settings),
  reproduction steps, and key logs.

---

## Known Issues

### Installation and signing

- macOS builds are ad-hoc signed; see
  [Getting Started](#getting-started) if Gatekeeper blocks the first
  launch.
- Windows installers are unsigned; SmartScreen prompts are expected.
- Linux requires the WebKitGTK 4.1 runtime.

### Network

- Configure proxy rules carefully; a global proxy may cause some
  third-party APIs to return abnormal data. A "bypass system proxy"
  switch is available in Settings.

### Capability boundaries

- QQ Music has no account login yet; guest APIs limit its capabilities.
- Downloads are not resumable yet; cancelling cleans up.
- Android-only mobile capabilities (USB exclusive output,
  floating/status-bar lyrics, SAF, safe mode) are out of scope.
- GitHub / WebDAV sync is not end-to-end encrypted; full-config exports
  contain authorization data — keep them private.

---

## Privacy

- No public cloud media service, no ad SDKs, no third-party analytics or
  crash-reporting SDKs.
- Playlists, favorites, recent plays, stats, downloads, settings, and
  auth data stay on your machine by default.
- If you enable GitHub / WebDAV sync, only metadata (playlists,
  favorites, history, stats) is synced to a remote you choose and own.
- Audio files, downloads, cookies, and playback tokens are never
  uploaded to the developer.
- For account safety, local playback history and stats are never written
  back to third-party music platforms.
- Full-config exports contain settings, authorization, and sync config —
  suitable for personal migration, not for public sharing.
- Third-party platforms handle their own access logs and risk control
  under their own privacy policies.

---

## Reference

<table>
<tr>
  <td><a href="https://github.com/cwuom/NeriPlayer">NeriPlayer (Android)</a></td>
  <td>✨ A native Android audio player that combines multi-source streaming, local control, rich lyrics, and self-hosted sync 🎵</td>
</tr>
<tr>
  <td><a href="https://github.com/amll-dev/applemusic-like-lyrics">applemusic-like-lyrics</a></td>
  <td>An Apple Music style lyric player component, with React &amp; Vue support</td>
</tr>
<tr>
  <td><a href="https://github.com/chaunsin/netease-cloud-music">netease-cloud-music</a></td>
  <td>✨ NetEase Cloud Music implementation in Golang 🎵</td>
</tr>
<tr>
  <td><a href="https://github.com/SocialSisterYi/bilibili-API-collect">bilibili-API-collect</a></td>
  <td>Bilibili API collection and documentation</td>
</tr>
<tr>
  <td><a href="https://lrclib.net">LRCLIB</a></td>
  <td>An open lyrics database service</td>
</tr>
</table>

---

## Update Cycle

- Work in progress; releases ship manually per feature batch.
- Playback, local data, and sync paths are maintained first.
- Third-party capabilities may be affected by platform policy changes;
  issues, PRs, and reproduction logs are welcome.

---

## Support

- Due to the nature of the project, donations are not accepted in any
  form.
- Support the project by filing issues, sending PRs, or sharing your
  experience.

---

## License

NeriPlayer Desktop is released under the **GPL-3.0** license
(GPL-3.0-or-later), the same as the Android app. See [LICENSE](./LICENSE)
for the full terms.

> Code committed before 2026-10-07 was released under the MIT license; that
> notice is kept in [LICENSES/MIT.txt](./LICENSES/MIT.txt). The
> `vendor/applemusic-like-lyrics` submodule is AGPL-3.0, and the parts shipped
> with the app follow its own license.

---

# Contributing

Please read [CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md) and
[CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) before contributing.



