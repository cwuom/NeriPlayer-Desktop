[English](./README_EN.md) | [简体中文](./README.md)

<h1 align="center">NeriPlayer Desktop</h1>

<div align="center">

<h3>🎵 Multi-source streaming, local music management, word-synced lyrics and sync to your own cloud</h3>

<p>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/cwuom/NeriPlayer-Desktop/total?style=social" /></a>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases"><img alt="Preview release" src="https://img.shields.io/github/v/release/cwuom/NeriPlayer-Desktop?include_prereleases&label=Preview" /></a>
  <img alt="Status: Preview" src="https://img.shields.io/badge/status-preview-orange" />
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-desktop-4C8BF5" />
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml"><img alt="CI" src="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml/badge.svg" /></a>
  <a href="./LICENSE"><img alt="License" src="https://img.shields.io/badge/License-GPL--3.0-blue" /></a>
  <a href="https://t.me/ouom_pub"><img alt="Telegram" src="https://img.shields.io/badge/Telegram-@ouom__pub-blue" /></a>
</p>

<p><img src="app-icon.png" width="200" alt="NeriPlayer Desktop logo" /></p>

<p>A desktop port of <a href="https://github.com/cwuom/NeriPlayer">NeriPlayer (Android)</a>, built with <strong>Tauri 2 + Vue 3 + Rust</strong> for Windows, macOS and Linux.</p>

<p>🚧 <strong>Currently a Preview release</strong></p>

<p>The name and icon are inspired by Kazamata Neri (风又音理) from <i>星空鉄道とシロの旅</i>.</p>

</div>

> [!IMPORTANT]
> The current version is available for testing and feedback and has not reached the stable-release stage. This document describes the current repository implementation; third-party APIs, system integration and cross-device compatibility may still change.
> Export important playlists and configuration and keep backups before updating or migrating. Consult the release notes for changes in each build.

> [!WARNING]
> This project is for learning and research only. Do not use it for illegal purposes. The project and its maintainers do not accept sponsorships, donations or commercial funding in any form.

NeriPlayer Desktop does not provide a public cloud music library or media distribution service. Online features depend on third-party APIs, accounts, regions and content permissions; membership and restricted content remain subject to the original platform's rules.

## 🧭 Documentation

- [Installation and first use](#installation)
- [Features and platform capabilities](#features)
- [Local music and downloads](#library-downloads)
- [Lyrics and desktop integration](#lyrics-desktop)
- [Keyboard shortcuts](#shortcuts)
- [Backup, sync and Android compatibility](#sync)
- [Listen Together](#listen-together)
- [Data and privacy](#data-privacy)
- [Known limitations and troubleshooting](#limitations)
- [Local development](#development)
- [Contribution guide](./CONTRIBUTING_EN.md) / [简体中文贡献指南](./CONTRIBUTING.md)

<a id="installation"></a>
## 📦 Installation and first use

### Download a Preview release

Visit [GitHub Releases](https://github.com/cwuom/NeriPlayer-Desktop/releases), read the release notes and choose the package for your platform and architecture. The release workflow targets the following distributions:

| Platform | Architecture | Package | Requirements |
| --- | --- | --- | --- |
| Windows | x64 | `.msi` / `.exe` (NSIS) | WebView2 Runtime |
| macOS | Apple Silicon / arm64 | `*-adhoc.dmg` | Build target: macOS 11.0+ |
| macOS | Intel / x64 | `*-adhoc.dmg` | Build target: macOS 10.15+ |
| Linux | x64 | `.deb` / `.rpm` / `.AppImage.tar.gz` | Runtime dependencies including WebKitGTK 4.1, GTK 3 and ALSA |

These minimum versions come from the packaging configuration and do not mean that every OS version has passed runtime acceptance testing. Releases also include `SHA256SUMS.txt`, macOS installation instructions, and source code and build scripts for the bundled FFmpeg.

- **Windows**: installers are currently unsigned and may trigger SmartScreen. Install the WebView2 Runtime first if it is missing.
- **macOS**: packages are ad-hoc signed and are not notarized by Apple. Drag the app into Applications and try opening it. If blocked, follow [Apple's official instructions](https://support.apple.com/en-us/102445) to choose “Open Anyway” in System Settings → Privacy & Security. The release's `macOS-install.txt` contains instructions specific to that build.
- **Linux**: extract the AppImage and make it executable; install deb/rpm packages with your distribution's package manager. AppImage still depends on the host's graphical and runtime environment.

### CI builds

To try changes that have not been released yet, visit the [Artifacts workflow](https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/artifacts.yml), select a successful build and download the artifact for your platform. Downloading Actions artifacts usually requires signing in to GitHub. Extract the outer artifact archive first, then use the installer inside.

CI builds and Releases may correspond to different commits. When reporting issues, include the copyable build version from Settings → About, rather than only the installer name or `1.0.0`.

### First use

1. Sign in to the platforms you need in Settings: NetEase Cloud Music, Bilibili and YouTube. Use the built-in login window or import cookies manually.
2. Search or browse in Explore; open platform playlists and favorite folders or import local files in Library.
3. Configure playback quality, audio output device, download directory, lyrics and shortcuts as needed.
4. Configure Settings → Backup & Sync when you need metadata across devices. Local playback does not require cloud sync.

<a id="features"></a>
## 🎧 Features and platform capabilities

### Feature overview

| Area | Current implementation |
| --- | --- |
| Home | Continue Playing at the top shows recently opened playlists, ranked by last-opened time and open count; recently played tracks and recommendation sections; YouTube Music home shelves appear in internationalization mode when signed in to YouTube |
| Explore | Separate search and discovery for NetEase, Bilibili and YouTube Music; optional search history; select a matching version from search results for playback |
| Library | Local playlists, favorite playlists and artists, downloads, NetEase playlists/albums, Bilibili favorite folders and YouTube Music playlists; search within categories, pagination, multi-select, dragging and context menus |
| Artists | NetEase, Bilibili and YouTube details and following; local artist grouping, detail search and playback of filtered results |
| Playback | Drag to reorder the queue while keeping the current track and position (Up/Down arrow keys also work on the reorder handle), shuffle/repeat, remembered position and mode, long-audio resume, progressive buffering, Range streaming and fragmented-audio seeking |
| Audio | Output device, channel balance, independent speed and pitch, vinyl effect, loudness gain and normalization, 10-band graphic EQ, 10-band parametric EQ and AutoEQ, 29 presets, tone and space processing, compressor and limiter, speaker optimization, per-output memory, pause/resume fades, track crossfades and multichannel dynamic-range compression |
| Lyrics | AMLL word-synced rendering, translation/romanization, automatic multi-source retrieval and manual matching, editing, local lyrics, per-source and per-track offsets, and a separate desktop lyrics window |
| Desktop | System tray, close to tray, configurable global shortcuts, system media sessions, single instance and macOS menu bar lyrics |
| Data | SQLite persistence, playlist and full-configuration import/export, GitHub / WebDAV metadata sync, playback stats and recent-play management |
| Interface | 简体中文, 繁體中文, English and 日本語; light/dark themes, cover-derived colors, custom backgrounds, fluid animation and audio response, and settings search; Appearance offers persisted UI scaling (macOS 11+), with display-aware limits and a minimum window size that follows the scale |
| Diagnostics | Connectivity probes, live logs, file logs, crash reports and debug-report export |

### Platform capabilities

| Source | Account and browsing | Playback, downloads and lyrics | Current boundaries |
| --- | --- | --- | --- |
| NetEase Cloud Music | Login, search, daily recommendations, playlists/albums, artists and like/unlike | Multiple qualities, YRC word-synced/translated/romanized lyrics and downloads | Account permissions, membership and regional restrictions still apply |
| QQ Music | Backend search and metadata/lyric matching | Streaming, lyrics and downloads within the limits of guest APIs | No QQ account login or dedicated Library/Explore category yet |
| Bilibili | Login, video search, discovery, created/subscribed favorite folders and creator details | DASH audio, HTML5 MP4 fallback, video parts, downloads and skip intervals | Removed videos and account/audio-track permissions may affect streaming; lyrics are matched from external sources |
| YouTube Music | Login, home, search, playlists and artists | Streaming through multiple clients, downloads, and native or externally matched lyrics | Network, region, platform checks and client availability may change |
| Local audio | Multi-folder library, incremental scanning, direct playback, playlist and artist management | Local decoding, tag reading/writing, embedded/sidecar lyrics | File paths must be valid on the current device; extended codec support depends on the decoder components |
| External lyrics | QQ Music, NetEase, Kugou, LRCLIB and AMLL TTML | Automatic retrieval, word-synced completion and manual matching | Matching accuracy and word timings depend on the source |

### Playback sources and audio information

- Audio Effects on the playback page provides speed, independent pitch, vinyl effect and presets. Vinyl effect makes pitch follow speed; turning it off restores the saved independent pitch. Speed ranges from **0.1x to 4x**, pitch from **0.25x to 2x**, with **0.95x / 1.05x** quick speed options.
- Detailed Audio Settings opens Settings → Audio Effects & Speed, with collapsible sections for 10-band graphic EQ, up to 10 parametric filters of 7 types, AutoEQ / Equalizer APO text or file import, theoretical frequency response, tone and vocals, stereo and reverb, full compressor and limiter controls, speaker optimization and processing quality. All numeric parameters provide sliders and direct input; disabling a section keeps its settings, and the master switch supports A/B comparison with original sound.
- 29 built-in presets are grouped by style, tone, scene and device. Save named user presets, delete them or import / export JSON; manual adjustments appear only as the current state. Sound profiles can follow actual output device names. The detailed page displays native DSP load and gain reduction; browser previews cannot report real native runtime status.
- Sound presets appear as direct cards: Common offers quick starting points and All exposes every built-in preset. Simple tuning starts with bass, vocals, brightness, warmth and space, using finer control near neutral and automatic headroom and peak protection. Advanced tuning retains the full parameters. Headphone correction and other active processing are preserved; bypassed sections restart from neutral. Speaker sizes are labeled Small, Medium and Large. Speaker loudness, protection frequency and compressor makeup gain change smoothly in the native chain.
- Bass processing follows Android's enhancement limiting and subsonic protection. Added bass, treble and virtual-bass content is reduced at peaks to limit whole-track pumping with kick drums. Automatic headroom is lighter, and both bass presets and the response curve follow the updated processing. Negative gains retain their attenuation, and saved custom effect parameters stay unchanged.
- Settings → Playback Sources lets you select a preferred YouTube client: Auto, visionOS, Android VR, Web Remix, TV HTML5 or Web Creator. Other available clients are tried after a failure.
- When NetEase does not allow full playback or only offers a preview, local-audio matching and Bilibili fallback audio can be enabled separately. Local matching takes priority; the original track's queue and sync identity are preserved, while downloads still use the original platform source.
- New defaults show the actual bitrate and format. Channels, sample rate and bit depth can each be enabled in personalization settings. Unknown parameters are omitted, and existing display preferences are preserved.
- Rust's `cpal` handles output, and the built-in Symphonia decoder handles common formats. Bundled FFmpeg extends decoding to Opus, AC-3/E-AC-3, ALAC, APE, WavPack, DSD and more. Actual support depends on the loaded decoders; if FFmpeg is missing, playback falls back to built-in decoding. Multichannel decoding and downmixing do not imply spatial-audio/Atmos output.

<a id="library-downloads"></a>
## 🗂️ Local music and downloads

### Local music

Library → Local Files lets you add multiple music folders to a persistent library, updated through native file watching and incremental scanning. It supports scan progress, cancellation, rescanning, search and multi-select, with filters for files already in playlists and duplicate metadata. Play the current list directly or add tracks to an existing or new playlist. Removing a library folder does not delete files from disk, and scanning does not automatically create a default playlist.

Local playlists support creation, renaming, deletion, sorting, dragging and bulk actions. Local artist details support search, and playback after filtering uses the current filtered list. Title, artist and album can be written back to audio files; the write process checks existing lyrics, covers and other tags.

The Library title bar provides entries for playback stats and recent plays; Home's Recently Played entry also opens the history page. You can browse and delete records, and deletion markers participate in cloud sync.

### Downloads

Track context menus and bulk selection provide download actions. The Downloads page shows the queue, source resolution, transfer, metadata processing and completed/failed states, with cancellation, retry, deletion, redownload and reveal-in-file-manager actions.

- Concurrency is **1–8**, default **6**. Download quality follows playback by default, or can be configured per platform.
- Custom directories and filename templates are supported. Download validation checks decodable content, duration and available length/checksum information, including fragmented audio.
- Metadata completion is enabled by default; standardized lyric embedding is disabled by default. Original lyrics are preserved when it is enabled. Downloads embed and verify the title, artist, album and available artwork, lyrics, translations and romanization. Artwork retrieval or tag-writing failures are reported as failed downloads that can be retried; completion requires successful verification. Unavailable source information is not invented, and disabling metadata completion makes embedding optional.
- Unfinished queues are recovered on restart by resolving the URL again and downloading again. **Byte-range download resume is not supported.**
- Before an operation on a downloaded file that is playing, its local decoder releases the file; playback then resumes according to the latest playback intent. Deletion can switch playback back to an online source.

| Content | Location |
| --- | --- |
| Audio | Selected download directory |
| Lyrics, translations and romanization | `Lyrics/` |
| Covers | `Covers/` |
| NP source and metadata references | Adjacent `<audio filename>.npmeta.json` |
| Temporary download files | `.tmp/` |

Clearing the app cache does not delete audio you downloaded. Cloud sync does not transfer downloaded files; download them again or copy them yourself when switching devices.

<a id="lyrics-desktop"></a>
## ✨ Lyrics and desktop integration

### Now Playing lyrics

Now Playing uses [applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics) for character/word highlighting, translation, romanization, blur and font-size adjustment, alongside a cover-colored WebGL fluid background, audio response or a blurred-cover background.

Settings → Appearance provides separate interface and Now Playing lyrics fonts, with common presets, custom installed font names and live previews. Defaults retain the original fonts, and the interface font does not affect lyrics. MiSans and Inter are bundled; other fonts require system installation and fall back to the default when unavailable. Desktop lyrics retain their independent font setting.

Hover over the lyrics to reveal translation and romanization toggles at the bottom center. Each toggle appears only when its content is available; romanization includes both line and word annotations. Each can be enabled independently or together, sharing the display preferences in Settings → Lyrics. The controls also appear on keyboard focus and stay visible on touch devices. Right-click the controls and select the hide action. Right-click any lyric line to show or hide the controls from the existing copy and seek menu; dismissing the menu leaves their visibility unchanged.

Settings → Lyrics → Long Note Animation Threshold allows values from 300 to 2000ms, with a default of 700ms. Lower values trigger the effect more often; advanced word animations must be enabled.

Automatic retrieval combines local lyrics, platform track IDs, caches and multi-source matching. Settings → Lyrics lets you select a default lyric source and control word-synced priority, falling back to automatic selection if no suitable result is found. With word-synced priority enabled, Bilibili tries word-synced sources first. Manual matching supports NetEase, QQ Music, Kugou, LRCLIB and AMLL TTML: edit keywords, select sources, preview and confirm results as candidates arrive.

When Kugou lyrics lack translation or romanization, background completion tries NetEase first, then QQ Music if translation is still missing. Only high-confidence results matching the title, artist and duration are used. Completion preserves Kugou's original lyrics, word timings and offset, updates the cache, and respects secondary tracks the user has confirmed or cleared.

Lyric displays filter explicit Chinese and English production, instrument, distribution and promotion credits, including MIDI programming, backing vocals, multi-word English roles and `by:` uploader credits. Delimited title-and-artist lines at the start or end are also filtered against the current track, including cached, local and synced lyrics. Ambiguous short labels and ordinary lyric sentences are kept, along with their word timings, translations and romanization. Display filtering does not rewrite local files or existing stored lyrics.

Offsets can be set per source or per track. Check the track version and duration when manually matching, editing or importing lyrics; live performances, covers and edited versions may need a new match.

### Desktop lyrics, tray and media keys

- Toggle the separate desktop lyrics window through Settings, the tray or shortcuts. It supports one/two/three lines, word-synced/full-line display, translation/romanization, fonts, gradient colors, outlines, shadows, opacity and backgrounds; window position and size are remembered.
- Desktop lyrics can be locked with click-through enabled, while an unlock control remains available on hover.
- With Close to Tray enabled, closing the main window keeps playback running in the background. The tray provides window, playback, lyrics and quit actions.
- macOS menu bar lyrics can be enabled independently; the track title appears when lyrics are unavailable, and long text is truncated.
- System media keys and sessions use `souvlaki` for Windows SMTC, Linux MPRIS and macOS Now Playing. Presentation depends on the OS and desktop environment.

<a id="shortcuts"></a>
## ⌨️ Keyboard shortcuts

Settings → Shortcuts configures in-app and system-wide bindings separately, with recording, clearing, reset-to-default and conflict hints. `Mod` means `Ctrl` on Windows/Linux and `Cmd` on macOS.

### Default in-app shortcuts

| Shortcut | Action |
| --- | --- |
| `Space` | Play/pause |
| `Mod + →` / `Mod + ←` | Next/previous track |
| `→` / `←` | Seek forward/back |
| `Shift + →` / `Shift + ←` | Seek forward/back with a larger step |
| `↑` / `↓` | Volume up/down |
| `M` / `S` / `R` | Mute/shuffle/repeat mode |
| `L` | Like/unlike supported tracks |
| `D` | Toggle desktop lyrics |
| `Mod + P` | Expand/collapse Now Playing |
| `Mod + F` | Search |
| `Esc` | Close dialogs, panels or Now Playing by layer |

Input fields and controls being operated take over their relevant keys. `Esc` and `Tab` are reserved for closing and focus navigation and cannot be rebound.

### Default system-wide shortcuts

Global shortcuts are **disabled by default**. Once enabled, they work when the window is unfocused or minimized to the tray.

| Shortcut | Action |
| --- | --- |
| `Mod + Alt + P` | Play/pause |
| `Mod + Alt + ←` / `Mod + Alt + →` | Previous/next track |
| `Mod + Alt + ↑` / `Mod + Alt + ↓` | Volume up/down |
| `Mod + Alt + L` | Like/unlike |
| `Mod + Alt + D` | Toggle desktop lyrics |
| `Mod + Alt + N` | Show the main window |

If a combination is taken by the OS or another app, Settings reports a registration failure so you can choose another combination. Input methods and desktop environments may also affect activation.

<a id="sync"></a>
## ☁️ Backup, sync and Android compatibility

### Sync scope

Settings → Backup & Sync connects to your own GitHub repository or WebDAV target, using data models and merge rules aligned with Android.

| Data | Cloud sync behavior |
| --- | --- |
| Local playlists | Track metadata, custom titles/artists/covers, matched lyrics, per-track offsets and membership deletion records |
| Favorites | Favorite playlists and artists stored as favorite records |
| Playlist usage | Last-opened time, open count and deletion records, used by Home's Continue Playing |
| Recent plays | Records and deletion markers |
| Playback stats | Play counts, listening time, daily buckets and clear markers |
| Bilibili skip intervals | Multiple intervals and clear markers per video part, keyed by `bvid + cid`; editing drafts stay local |
| Audio and device configuration | Audio, downloaded files, platform cookies and playback tokens are not uploaded; device settings and paths must be configured separately |

Syncing local track metadata does not give another device the files. Download directories, output devices and local paths cannot be migrated directly into a working environment either.

### Setup and format upgrades

1. **GitHub**: configure and validate a Personal Access Token as instructed in the app, then create the default private repository or select an existing one. The current UI's classic-token flow uses the `repo` scope. See [GitHub's official documentation](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens) for token types and permissions.
2. **WebDAV**: enter the server URL, account and password, and confirm that the remote target is readable and writable.
3. Sync manually after setup; local changes can also trigger automatic sync. Connect all devices to the same target.
4. Confirm the upgrade when an older format is detected. Update clients on other devices first and keep a remote backup.

Current writes use **V4 ProtoBuf manifests + Zstd chunked archives**, with support for reading legacy JSON, GZIP and V3. The manifest is still named `neriplayer-sync-v3.manifest`; the filename does not mean its contents are V3. Merging retains deletion/clear markers to prevent older devices from restoring deleted content.

Archive leaf chunks download in batches of up to four concurrent requests. GitHub reuses local objects after checking them against a fixed commit tree, publishes small changes with one conditional commit, and retains staged uploads for larger changes. WebDAV continues to verify remote content and protect writes with leases and conditions.

WebDAV follows Android's Nutstore compatibility behavior: an initial LOCK response of 403/405/501 permits unlocked sync, while targets that previously granted a finite lease cannot downgrade. Reads request identity encoding, and Nutstore's unquoted ETags are sent unchanged in conditional writes. The manifest is checked before publication and read back afterwards. Without a usable ETag, fingerprint checks and readback cannot guarantee atomic concurrent writes; automatic removal of old chunks still requires a finite lease and strong ETags.

Legacy Bilibili metadata in the form `Bilibili|CID|BV-id` uses only the CID as the sub-audio identity, matching Android so lyric references resolve to their independent overrides. Archives with missing overrides or insufficient revisions are still rejected.

Cross-platform use requires both clients to support the current protocol. The repository includes frozen Android test vectors and a [Kotlin interoperability validation script](./scripts/test-sync-android-interop.ps1). Codec checks do not replace testing real accounts, a shared remote and alternating sync across multiple devices.

### Manual backups

- **Playlist JSON import/export**: for playlist backups and migration.
- **Full-configuration import/export**: migrates settings, authorization, sync configuration and more. It is **plaintext JSON** that may contain cookies, tokens and passwords, not a complete backup of audio files or the database.
- **GitHub / WebDAV**: continuously syncs the metadata listed above; remote archives are not end-to-end encrypted.

### Bilibili skip intervals

The More menu in Now Playing and Bilibili track context menus provide interval editing per video part, with multiple intervals, start/end times from the current position, preview and clearing. Saved intervals are skipped automatically, and clearing is also synced. **Automatic skipping is paused inside Listen Together rooms** so one client's jumps do not affect room progress.

Android's mobile-specific features, including USB exclusive playback, SAF and floating/status-bar lyrics, are outside the desktop scope. Desktop provides its own tray, desktop lyrics, media sessions and shortcuts.

<a id="listen-together"></a>
## 🎧 Listen Together

Create or join rooms to sync playback, pause, position, queue, track changes and repeat/shuffle modes over WebSocket. Member control, auto-pause on member changes, direct audio-link sharing and custom servers are configurable.

Desktop and Android use the same Listen Together protocol, requiring an available compatible server and content that each client can play. Room sharing does not replace platform account permissions. Before enabling direct-link sharing, confirm that you accept providing temporary audio URLs to members.

Start playing a shareable online track before creating a room. The Listen Together queue currently excludes local files and QQ Music tracks.

The server runs on Cloudflare Workers + Durable Objects. See [NeriPlayer-LTW](https://github.com/TheSmallHanCat/NeriPlayer-LTW).

<a id="data-privacy"></a>
## 🔒 Data and privacy

- Playlists, favorites, recent plays, the queue, playback stats, the download catalog, sync metadata and detail/lyric caches are managed by the local SQLite database `neri_user_data.db`. Downloaded audio stays in the selected directory.
- Legacy JSON and WebView localStorage are migrated by data domain; legacy JSON backups are kept in `legacy-json-backup/` under the user data directory. Local library folders and the file index are stored separately in `local-library.json`; settings, sign-in state and sync configuration use their own stores.
- **Release builds** use app-side encryption for platform cookies, GitHub tokens and WebDAV passwords. **Debug builds** use plaintext credential files at randomized paths. Encryption does not promise protection against processes running as the same user on the same machine. Logs are redacted, but should still be checked before sharing.
- Data stays on the machine by default. Enabling sync writes to the selected GitHub/WebDAV remote; Listen Together exchanges session data with the configured server and members.
- No ad SDKs, third-party analytics or crash-analysis SDKs are integrated, and local play history and stats are not written back to music platforms. Login, search, playback and lyric requests remain subject to each platform's privacy policy and access-log rules.

<a id="limitations"></a>
## ⚠️ Known limitations and troubleshooting
| Scenario | Explanation and action |
| --- | --- |
| Search, login or streaming fails | APIs, network, regions or account permissions may change; confirm the login state and that the content works on the original platform, then check proxy settings |
| QQ Music capabilities are limited | No account login yet; guest API restrictions apply |
| Tray, desktop lyrics or media keys behave differently | OS and desktop-environment differences apply; source support does not mean every environment has passed acceptance testing |
| Global shortcuts do not respond | Enable the global switch, inspect registration errors and check conflicts with the OS or other programs |
| Older clients fail to sync | Update all clients; back up the remote and read the confirmation prompt before upgrading |
| App updates | No in-app auto-update is provided yet; download and install the corresponding Preview release manually |

Proxy settings include a Bypass System Proxy switch; adjust it to your network and test. Debug connectivity probes only check whether the transport layer receives a response. They do not prove valid login, playable content or successful business API calls.

### Reporting issues

1. Copy the build version from Settings → About, and record your OS, architecture and reproduction steps.
2. Click the version number **7 times** to enable developer mode, then open `Debug` for live logs, crash reports or debug-report export.
3. Enable file logging and select a level in Settings if persistent logs are needed.
4. File an [Issue](https://github.com/cwuom/NeriPlayer-Desktop/issues) using the template. For cross-platform issues, include both Desktop and Android versions. Remove personal information and credentials before sharing.

<a id="development"></a>
## 🛠️ Local development

The repository pins **pnpm 9.15.9** and **Rust 1.95.0**; CI uses **Node.js 20**. Install Git, Node.js, pnpm, rustup and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). The project also needs a toolchain to compile the FFmpeg C++ shim.

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

For an existing clone missing the lyrics submodule, run `git submodule update --init --recursive`. `pnpm tauri dev` starts Vite and Tauri on the default port `1420` and automatically selects another available port if it is occupied.

| Command | Purpose |
| --- | --- |
| `pnpm dev` | Starts Vite only, without Rust IPC/native playback |
| `pnpm build` | Checks Vue/TypeScript and generates frontend `dist/` |
| `pnpm test` | Runs the aggregate frontend regression scripts |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | Rust static compilation checks |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked` | Rust tests; FFmpeg checks require available runtime libraries |
| `pnpm tauri build` | Native compilation and packaging; FFmpeg shared libraries are not added automatically by default |

Packages with extended decoding also require FFmpeg shared libraries matching the target. Run `scripts/ffmpeg/bundle-config.mjs` and pass the generated configuration to `tauri build --config`. See the [contribution guide](./CONTRIBUTING_EN.md) and [FFmpeg build instructions](./src-tauri/native/ffmpeg/README.md) for platform setup, commands, output paths and CI/Release workflows.

The in-app build version is `<git-short-hash>.<MMddHHmm>`, with a build time and UUID also available. The packaging metadata version `1.0.0` does not indicate a stable release. Release frequency depends on feature work and maintenance; see Releases and commit history for changes.

## 🤝 Contributing and future directions

Reproducible issues, documentation improvements, Preview testing and code contributions are welcome. Read [CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md) and [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) before contributing.

Work continues on playback stability, lyric matching, cross-platform data compatibility, download recovery and the desktop experience across OSes. QQ account login, in-app auto-update and byte-range download resume remain future directions with no promised schedule; the tray, global shortcuts and desktop lyrics are already available.

## 🙏 Acknowledgments

- [NeriPlayer (Android)](https://github.com/cwuom/NeriPlayer): feature, protocol and interaction reference
- [applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics): word-synced lyrics component
- [netease-cloud-music](https://github.com/chaunsin/netease-cloud-music): reference for NetEase API implementation
- [bilibili-API-collect](https://github.com/SocialSisterYi/bilibili-API-collect): Bilibili API documentation
- [LRCLIB](https://lrclib.net): open lyrics service
- [FFmpeg](https://ffmpeg.org/): extended decoding and demuxing

## 📄 License

This project is released under **GPL-3.0-or-later**. See [LICENSE](./LICENSE). Code released under MIT before 2026-10-07 retains its original license notice in [LICENSES/MIT.txt](./LICENSES/MIT.txt).

`vendor/applemusic-like-lyrics` uses AGPL-3.0 and is distributed under its own license. The bundled minimal FFmpeg build does not enable GPL components and is distributed under LGPL-2.1-or-later. See the [FFmpeg notes](./src-tauri/native/ffmpeg/README.md) and Release attachments for licenses, build information and source access.
