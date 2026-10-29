[English](./README_EN.md) | [中文](./README.md)

<h1 align="center">NeriPlayer Desktop (音理音理!)</h1>

<div align="center">

<h3>A desktop audio player for multi-source streaming, local libraries, word-synced lyrics, and user-controlled sync</h3>

<p>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/cwuom/NeriPlayer-Desktop/total?style=social" />
  </a>
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/releases">
    <img alt="Release" src="https://img.shields.io/github/v/release/cwuom/NeriPlayer-Desktop?include_prereleases&label=Release" />
  </a>
  <img alt="Preview" src="https://img.shields.io/badge/status-preview-orange" />
  <img alt="Platforms" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-desktop-4C8BF5" />
  <a href="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml">
    <img alt="CI" src="https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/build.yml/badge.svg" />
  </a>
  <a href="https://t.me/ouom_pub">
    <img alt="Telegram" src="https://img.shields.io/badge/Telegram-@ouom__pub-blue" />
  </a>
</p>

<p>
  <img src="app-icon.png" width="200" alt="NeriPlayer Desktop" />
</p>

<p>
The project name and icon are inspired by Kazamata Neri (风又音理) from “星空鉄道とシロの旅”.
</p>

<p>
This project is a desktop port of <a href="https://github.com/cwuom/NeriPlayer">NeriPlayer Android</a>,
built with <strong>Tauri 2 + Vue 3 + Rust</strong> for Windows, macOS, and Linux.
Development focuses on multi-source discovery, native playback, local control, and cross-device sync.
</p>

<strong>Preview / Under active development</strong>

</div>

> [!WARNING]
> This project is for learning and research only. Do not use it for illegal purposes.
> Access, play, or save content only where you have the necessary rights or authorization, or where the source platform permits it.
> The project does not provide media content, keys, methods to bypass payment, DRM, or regional restrictions, or a public media proxy or redistribution service.
>
> The project and its maintainers do not accept sponsorships, donations, or commercial funding.

---

> [!NOTE]
> NeriPlayer Desktop does not provide a public cloud music library or media distribution service.
> Online features depend on third-party APIs, connectivity, accounts, and content permissions. Membership and other restricted content remain subject to the source platform's rules.
> This is still a Preview release. Back up important playlists and configuration before updating or migrating.
> The packaging metadata version `1.0.0` does not indicate a stable release. Refer to the relevant build and release notes for changes.

---

## Start here

To try the app, see [Getting Started](#getting-started).
To learn what it offers, see [Why it stands out](#why-it-stands-out)
and [Key Features](#key-features).
To contribute code, start with [CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md).
To host a Listen Together server, see
[Listen Together Deployment](#listen-together-deployment).
Before using multiple devices, read the compatibility notes in
[GitHub Sync](#github-sync) and [WebDAV Sync](#webdav-sync).

```text
NeriPlayer Desktop
├── Multi-source streaming: NetEase / Bilibili / YouTube Music
├── Local-first data: library, downloads, playlists, history, queue, and stats
├── Optional user-controlled sync: GitHub / WebDAV metadata sync
├── Desktop playback: native audio, word-synced lyrics, effects, tray, media keys, and desktop lyrics
└── Diagnostics and feedback: live logs, file logs, crash reports, and connectivity probes
```

## About

NeriPlayer Desktop uses **Vue 3 + TypeScript** for its interface,
**Rust** for platform requests, audio decoding, playback, storage, and sync,
and **Tauri 2** for desktop integration.
It brings together NetEase Cloud Music, Bilibili, YouTube Music, and local audio,
covering discovery, queue playback, lyrics, file management, and metadata sync.

The current scope:

- **Local-first**: playlists, favorites, history, the queue, playback stats, and caches
  are stored on the device by default. Importing local files does not require a platform login or cloud sync.
- **Native audio**: Rust decodes audio and sends it to `cpal` for output.
  The Now Playing WebView handles the interface, word-synced lyrics, and animated backgrounds.
- **Accounts and content permissions**: sign in to a source platform as needed to access content available to that account.
  QQ Music currently offers limited guest APIs and lyrics/metadata capabilities.
- **Optional sync**: save metadata to your own GitHub repository or WebDAV target.
  Manual and automatic sync use three-way merging to retain edits from each device.
- **Cross-device collaboration**: sync and Listen Together follow Android's data models and protocols.
  Compatible versions can sync playlists, history, and stats, or join the same room to listen together.
- **Desktop experience**: independent desktop lyrics, tray, system media sessions, media keys,
  configurable shortcuts, output device selection, native DSP, and macOS menu bar lyrics.

---

## Why it stands out

- **Multi-source playback with explicit fallback policies**:
  search by platform in Explore, and bring platform playlists, favorites, and local content together in Library.
  When a NetEase song cannot play in full, optional settings can match local audio first, then try a Bilibili alternative.
  The queue keeps the original song's identity, and downloads still use its original platform.
- **An audio pipeline independent of the WebView**:
  built-in Symphonia handles common formats, while bundled FFmpeg libraries extend decoding and demuxing.
  Choose an output device, adjust speed and pitch independently, normalize loudness, and use fades or crossfades.
- **Simple controls backed by native DSP**:
  29 built-in presets, a 10-band graphic equalizer, parametric EQ, AutoEQ import, tone and spatial processing,
  compression, limiting, and speaker optimization, with effects remembered per output device.
  See the [DSP notes](./src-tauri/native/dsp/README.md) for native DSP sources and desktop extensions.
- **Rich word-synced lyrics**:
  AMLL provides word highlighting, translations, and phonetics, with automatic multi-source retrieval,
  manual matching, editing, and offset adjustment. Now Playing includes a cover-colored WebGL fluid background and audio response.
- **A manageable local library**:
  multiple indexed folders, file monitoring and incremental scans, playback from filtered lists,
  playlists and artist grouping, tag editing, and download management work together.
  Removing a library folder does not delete its audio files.
- **Sync that preserves edits and deletions**:
  GitHub/WebDAV use three-way merges and V4 chunked archives, retaining deletion and clearing records
  for playlist membership, recent plays, stats, and skip ranges rather than replacing the whole library with the last write.
- **Everyday desktop controls**:
  independent desktop lyrics, the system tray, close-to-tray behavior, media keys, system media sessions,
  configurable in-app and global shortcuts, and macOS menu bar lyrics.
- **Diagnostics available on demand**:
  developer mode provides live logs, crash reports, and connectivity probes.
  File logging can be enabled separately, and reports can identify the exact build.

---

## Getting Started

### a. Download a Release build (recommended)

1. Visit [GitHub Releases](https://github.com/cwuom/NeriPlayer-Desktop/releases) and read the release notes
2. Choose the package for your OS and architecture:

   | Platform | Architecture | Release package | Requirements |
   | --- | --- | --- | --- |
   | Windows | x64 | `.msi` / NSIS `.exe` | WebView2 runtime required |
   | macOS | Apple Silicon / arm64 | `*-adhoc.dmg` | Release build minimum configured as macOS 11.0 |
   | macOS | Intel / x64 | `*-adhoc.dmg` | Release build minimum configured as macOS 10.15 |
   | Linux | x64 | `.deb` / `.rpm` / `.AppImage.tar.gz` | Compatible graphical environment and system libraries required |

3. Releases include `SHA256SUMS.txt`, macOS installation notes, and source/build scripts for the bundled FFmpeg libraries
4. Sign in to NetEase, Bilibili, or YouTube in Settings as needed, or start by importing local music
5. Adjust the output device, playback quality, download directory, lyrics, and shortcuts; configure sync separately if needed

Windows installers are currently unsigned. macOS packages use ad-hoc signing and are not notarized by Apple.
Read the included `macOS-install.txt` first; for system security prompts, see
[Apple's instructions](https://support.apple.com/en-us/102445).

Extract the Linux AppImage archive, then make the `.AppImage` executable.
Install deb/rpm packages with your distribution's package manager; the packaging configuration includes
dependencies such as WebKitGTK 4.1, GTK 3, and ALSA.
The OS minimums above come from build configuration and do not mean every version and desktop environment has been verified.

### b. Download a CI build

Open the [Artifacts workflow](https://github.com/cwuom/NeriPlayer-Desktop/actions/workflows/artifacts.yml),
choose a successful run, download the artifact for your platform, and extract its installer.
Downloading Actions artifacts generally requires a GitHub login, and artifacts have a limited retention period.

CI and Release builds may use different commits. When reporting issues, include the build version copied from
Settings → About, rather than only `1.0.0` or the package filename.

### c. Local build

Install Git, Node.js, **pnpm 9.15.9**, rustup, and the
[Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).
The repository pins **Rust 1.95.0**, and CI uses **Node.js 20**.
Native builds also need a C++17 compiler.

```bash
git clone --recursive https://github.com/cwuom/NeriPlayer-Desktop.git
cd NeriPlayer-Desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

If an existing clone is missing the lyrics submodule, run `git submodule update --init --recursive`.
`pnpm tauri dev` starts both Vite and the native app, using port `1420` by default and choosing a free port if needed.

> `pnpm dev` starts only the interface preview, without Rust IPC, native audio, or system integration.
> Debug credential storage and performance also differ from Release builds. Browser previews do not replace native validation.

For local Release builds and FFmpeg packaging, read
[CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md)
and the [FFmpeg build notes](./src-tauri/native/ffmpeg/README.md).
A plain `pnpm tauri build` does not automatically download or bundle FFmpeg shared libraries.

---

## Key Features

- **Home and Explore**: recent plays, frequently opened playlists, recommendations by section,
  NetEase/Bilibili/YouTube Music search, and manageable search history.
  International mode supports YouTube Music home shelves for signed-in accounts.
- **Library and artists**: local playlists, favorite playlists and artists, downloads,
  NetEase playlists/albums, Bilibili favorites, and YouTube Music playlists;
  category search, pagination, multi-selection, drag-and-drop, and context menus.
- **Queue and resume**: shuffle, repeat, reordering, saved playback progress, and long-form resume.
  Shuffle shows the actual playback order; switch to the original order to drag tracks into place.
- **Native audio and effects**: output devices, channel balance, 0.1x–4x speed, 0.25x–2x independent pitch,
  vinyl mode, loudness normalization, graphic/parametric EQ, AutoEQ, tone and spatial processing,
  user presets, and per-device memory.
- **Lyrics display and matching**: line/word sync, translations, romanization, fonts and sizes, lyric blur,
  local/embedded lyrics, and per-source/per-track offsets.
  Manually match lyrics from NetEase, QQ Music, Kugou, LRCLIB, and AMLL TTML.
- **Independent desktop lyrics**: one/two/three lines, word or whole-line display, translations/romanization,
  fonts, gradients, outlines, shadows, opacity, backgrounds, saved position and size, locking, and click-through.
- **Local files**: multiple library folders, incremental scans, scan previews and cancellation,
  search and multi-selection, local artist grouping, direct playback, playlist import,
  and editing of title, artist, and album tags.
  Sort ascending or descending by default order, file modification time, title, artist, album, or duration.
  Sorting preferences are saved, and Play all and Play selected follow the displayed order.
- **Library interactions**: single-click playback by default, with double-click playback available
  in Settings → Playback Settings → Double-click to play tracks.
  The current track keeps its full-row highlight and outline while paused.
- **Download management**: individual and batch downloads, 1–8 concurrent tasks (default 6),
  separate quality settings, directories and filename templates, cancellation, retry, redownload,
  file location, and startup recovery.
- **Download metadata**: optional completion and read-back verification of tags, artwork, lyrics,
  translations, and romanization. Standardized lyric embedding has its own setting.
  Lyrics go in `Lyrics/`, artwork in `Covers/`, and source references in `.npmeta.json` beside the audio.
- **Comments**: NetEase and Bilibili comment browsing, reply threads, copying, and signed-in posting,
  replies, and likes. An uncertain request result keeps the draft without automatic resubmission.
- **Bilibili parts and skip ranges**: choose a starting part, queue all parts, and edit, preview,
  or sync multiple skip ranges identified by `bvid + cid`. Automatic skipping pauses during Listen Together.
- **Backups and stats**: playlist JSON import/export, full configuration import/export, recent-play management,
  play counts and listening time, and GitHub/WebDAV metadata sync.
- **Listen Together**: create/join rooms, invitations, playback and queue sync, member controls,
  reconnection, repeat/shuffle modes, custom servers, and configurable session playback URL sharing.
- **Appearance and languages**: Simplified Chinese, Traditional Chinese, English, and Japanese;
  light/dark themes, cover colors, custom/fluid backgrounds, interface/lyric fonts,
  interface scaling, and settings search.
- **Desktop integration and diagnostics**: tray, close to tray, single instance, system media sessions,
  media keys, in-app/global shortcuts, macOS menu bar lyrics, live logs, and diagnostic reports.

### Default shortcuts

`Mod` means `Ctrl` on Windows/Linux and `Cmd` on macOS.
Record, clear, or restore bindings in Settings → Shortcuts; conflicts and registration failures are shown there.

| In-app shortcut | Action |
| --- | --- |
| `Space` | Play/pause |
| `Mod + →` / `Mod + ←` | Next/previous track |
| `→` / `←` | Seek forward/backward |
| `Shift + →` / `Shift + ←` | Seek forward/backward by a larger step |
| `↑` / `↓` | Volume up/down |
| `M` / `S` / `R` | Mute/shuffle/repeat mode |
| `L` / `D` | Like/unlike; toggle desktop lyrics |
| `Mod + P` / `Mod + F` | Expand Now Playing; search |
| `Esc` | Close dialogs, panels, or Now Playing in order |

Global shortcuts are disabled by default. Once enabled, `Mod + Alt + P` controls playback,
`Mod + Alt + ←/→` changes tracks, `Mod + Alt + ↑/↓` changes volume,
`Mod + Alt + L` toggles like, `Mod + Alt + D` toggles desktop lyrics,
and `Mod + Alt + N` shows/hides the window.

Text fields and active controls take priority over the relevant keys.
`Esc` and `Tab` are reserved for closing and focus navigation and cannot be rebound.

---

## Platform Status

| Source | Current capabilities | Limits |
| --- | --- | --- |
| NetEase Cloud Music | Login, search, recommendations, playlists/albums, artists, likes, multiple playback qualities, downloads, YRC/translated/romanized lyrics, comments | Membership, regional, and account permissions still apply; alternative sources are an optional playback policy |
| Bilibili | Login, search, discovery, favorites, creators, DASH audio with MP4 fallback, video parts, downloads, skip ranges, comments | Primarily audio playback from videos; content/account permissions affect streams, and lyrics are matched externally |
| YouTube Music | Login, home, search, playlists and artists, multiple stream clients, downloads, lyrics | Depends on connectivity, region, sessions, and client availability; compatibility logic does not bypass platform restrictions |
| QQ Music | Backend search, streaming within guest API availability, downloads, lyrics/metadata matching | Depends on guest API and content availability |
| Local audio | Multi-folder scans, direct playback, playlist and artist management, tags and lyrics | Paths must exist on the current device; formats depend on available decoders |

The desktop app combines native audio output, device selection, and DSP effects with independent desktop lyrics,
tray controls, system media sessions, media keys, and configurable shortcuts for in-window and background playback control.

---

## Implementation Notes

### Build and versions

- Stack: Tauri 2, Vue 3, TypeScript, Pinia, Vite, Rust, and C++17
- Package manager: `pnpm@9.15.9`; Rust toolchain: `1.95.0`; CI Node.js: `20`
- In-app version: `<git-short-hash>.<MMddHHmm>`, with build time and UUID also available
- Native configuration: `src-tauri/tauri.conf.json`; release matrix: `.github/workflows/release.yml`
- Lyrics component: the `vendor/applemusic-like-lyrics` Git submodule
- Packages with extended decoding require an additional FFmpeg resource configuration; local development does not require FFmpeg development packages

### Module layout

| Path | Responsibility |
| --- | --- |
| `src/views/`, `src/components/` | Pages, player, lyrics, menus, and settings |
| `src/stores/`, `src/modules/` | Frontend state, playback coordination, lyrics, shortcuts, and Listen Together |
| `src-tauri/src/commands/` | Tauri commands and frontend/backend data boundaries |
| `src-tauri/src/api/`, `src-tauri/src/auth/` | Platform requests, login, signing, and sessions |
| `src-tauri/src/audio/`, `src-tauri/native/` | Native playback, decoding, output, FFmpeg, and DSP |
| `src-tauri/src/lyrics/`, `src-tauri/src/library/` | Lyric parsing/matching, library, tags, and stats |
| `src-tauri/src/db/`, `src-tauri/src/sync/` | SQLite, migration, sync models, merging, and V4 archives |
| `src-tauri/src/listen_together/` | WebSocket sessions and protocol |
| `scripts/`, `.github/workflows/` | Regression checks, builds, packaging, and CI |

### Entry point and navigation

The main interface includes Home, Explore, Library, Settings, and a Debug page in developer mode.
Mini Player, Now Playing, the queue, and comments panels work with shared playback state.
Desktop lyrics and the tray popup use separate windows synchronized through native commands and events.

### Playback, cache, and service

- `src-tauri/src/audio/` handles playback, seeking, buffering, decoding, output, and media sessions.
  Online playback supports HTTP Range streaming and segmented seeking; this is separate from download file resume.
- Symphonia is the built-in decoding backend. FFmpeg shared libraries load in a runtime search order,
  with fallback attempted if libraries are missing or incompatible. Some formats may remain unavailable without them.
- DSP covers equalization, tone, spatial, and dynamics processing.
  Multichannel decoding and downmixing do not imply Atmos or spatial audio output.
- Cache cleanup and download deletion are separate. Normal cache cleanup preserves audio intentionally downloaded by the user.

### Search and data sources

Explore has NetEase, Bilibili, and YouTube Music categories.
Rust platform APIs and the lyrics manager retrieve playback URLs, quality details, artwork, and lyrics.
QQ Music, Kugou, LRCLIB, and AMLL TTML also participate in lyric matching.
External matches depend on the recording version, artist, and duration;
check live performances, covers, and edited versions manually.

### Local data and security

- `neri_user_data.db` stores playlists, favorites, history, the queue, stats, download records,
  sync metadata, and detail/lyric caches.
- Library folders and the file index are stored separately in `local-library.json`;
  settings and credentials also have separate storage.
- Legacy JSON and localStorage migrate by data domain; old JSON backups go in `legacy-json-backup/`.
- Release builds use application-encrypted credential files; Debug builds use plaintext files at randomized paths.
  This is not system hardware-backed key protection and does not promise protection from other processes running as the same user on the same machine.
- Full configuration exports are plaintext JSON and may contain platform authorization and sync passwords.
  They are not suitable as publicly shared attachments.

### Downloads, local import, and backups

- Play local files directly or import them into playlists. Scanning does not automatically create a default playlist.
- Downloads resolve the source, transfer audio, process metadata, and verify it by reading it back.
  When metadata completion is enabled, artwork or tag processing failures mark the task as failed and allow retry.
- Metadata completion is enabled by default; standardized lyric embedding is disabled by default.
  Information absent from the source is not invented.
- Startup restores unfinished tasks and resolves/downloads them again.
  Cancellation and retry do not provide byte-range resume.
- Playlist JSON migrates playlists; full configuration exports migrate settings and authorization.
  Neither includes complete audio files or a full database backup.

See [CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md) for implementation paths, extension points, and validation commands.

---

## Listen Together Deployment

NeriPlayer Desktop includes a Listen Together client with the default server address
`https://neriplayer.hancat.work`. You can change it to a compatible self-hosted server.
The server is based on **Cloudflare Workers + Durable Objects** and synchronizes room state over WebSocket.

Server source and deployment entry points:

- [np-submodule/NeriPlayer-LTW](https://github.com/cwuom/NeriPlayer/tree/master/np-submodule/NeriPlayer-LTW)
  in the Android repository
- Public deployment template:
  [TheSmallHanCat/NeriPlayer-LTW](https://github.com/TheSmallHanCat/NeriPlayer-LTW)

The desktop repository does not include the server source.
Refer to the server repository for deployment and protocol requirements.

### Deploy to Cloudflare Workers

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/TheSmallHanCat/NeriPlayer-LTW)

After deployment, enter and test the server address in Settings → Listen Together,
then create or join a room from the Listen Together panel.

- Play a shareable online track before creating a room; current room queues exclude local files and QQ Music tracks
- Rooms synchronize play, pause, position, the queue, and repeat/shuffle modes, with member controls, pause on membership changes, and reconnection
- The first join requires a valid invitation; preserve all invitation details rather than sending only the room ID
- Temporary session playback URL sharing is enabled by default (`shareAudioLinks`); this provides the URLs to the server and room members, and can be disabled in Settings
- Cross-platform rooms require compatible Android, desktop, and server protocols; each member still needs access to the relevant content

---

## GitHub Sync

Sync local metadata to **your own GitHub repository**.
Repositories created in the app default to private; existing repositories are also supported.

Current sync targets:

- Local playlists, track metadata, custom information, matched lyrics, and per-track lyric offsets
- Favorite playlists and artists stored as favorite records
- Playlist last-opened times, open counts, and deletion records
- Recent plays, deletion records, and long-form resume information
- Play counts, listening time, daily stat buckets, and clearing markers
- Bilibili skip ranges identified by `bvid + cid`, including clearing markers

### Technical details

- **V4 archives**: current writes use a ProtoBuf manifest and Zstd chunks.
  The filename remains `neriplayer-sync-v3.manifest`; it does not indicate the protocol version.
- **Merge policy**: three-way merging, causal membership records, and deletion markers handle conflicts
  rather than replacing the whole library with the last write.
- **Conditional publication**: reads use a fixed commit, and publication checks the branch head.
  Branch conflicts are reported or trigger a new merge rather than force-overwriting another device's commit.
  The GitHub service must support the relevant GraphQL publication endpoints.
- **Legacy formats**: reads support legacy JSON, GZIP, historical Base64, and V3 archives.
  Corrupt current manifests or unknown versions fail explicitly rather than silently loading an old snapshot.
- **Cross-device upgrades**: every participating client should support V4.
  Older clients that keep reading/writing legacy files cannot automatically converge with V4 archives.
  Update every client and keep backups before migration.
- **Security boundary**: remote archives are not end-to-end encrypted.
  Audio, downloaded files, platform cookies, and playback tokens are not uploaded.
  Local paths, output devices, and similar configuration must be prepared separately on another device.
- **Protocol interoperability**: the repository provides frozen Android vectors and a
  [Kotlin interoperability script](./scripts/test-sync-android-interop.ps1).
  These help verify archive compatibility across clients.

### How to use

1. Open Settings → Backup & Sync
2. Follow the in-app instructions to create and validate a GitHub Personal Access Token.
   The current classic-token flow uses `repo` permission; see [GitHub's documentation](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens) for token types and permissions
3. Create the default private repository or choose an existing sync repository
4. Connect devices to the same target. If a format-upgrade prompt appears, update the other clients and back up first, then confirm migration
5. Run a manual sync, then enable automatic sync as needed. Background sync is eventually consistent, not real-time push

---

## WebDAV Sync

WebDAV uses the same sync targets, V4 archives, and merge rules as GitHub.

1. Select WebDAV in Settings → Backup & Sync and enter the remote URL, username, and password
2. Test read/write access; HTTPS is recommended
3. Review backup and format-upgrade prompts before the first sync, and use the same target across devices

WebDAV stores the manifest and content objects at the target location, protecting publication with
leases, ETag conditional writes, and read-back verification.
A compatibility path can allow synchronization without a lock when the server refuses the initial lock request.
Without a usable ETag, content fingerprints and read-back checks can detect conflicts but cannot guarantee atomic concurrent writes.
Automatic cleanup of old chunks still requires a finite lease and a strong ETag.

WebDAV remote files are not end-to-end encrypted backups.
Protect usernames, passwords, and configuration exports yourself.
Sync does not transfer audio files or automatically repair local paths on another device.

---

## Roadmap

### Exploring

These directions may change with maintainer availability, platform changes, and feedback.
They have no fixed schedule.

- [ ] Fuller QQ Music account, discovery, and library capabilities
- [ ] Byte-range download resume
- [ ] In-app automatic updates
- [ ] Compatibility validation across more operating systems, desktop environments, and multi-device scenarios

### Shipped recently

- [x] Native audio output, FFmpeg extended decoding, and full DSP settings
- [x] AMLL word-synced lyrics, manual multi-source matching, and independent desktop lyrics
- [x] Multi-folder local libraries, incremental scans, artist management, and tag editing
- [x] SQLite user data and legacy storage migration
- [x] GitHub/WebDAV V4 archives, three-way merging, and Android protocol vectors
- [x] Listen Together rooms, queues, and playback mode sync
- [x] NetEase/Bilibili comment browsing and signed-in interactions
- [x] Bilibili parts, download management, and skip ranges
- [x] Tray, media sessions, global shortcuts, and a multilingual interface

---

## Bug Report

1. Copy the build version from Settings → About, and record your OS, architecture, and reproduction steps
2. Click the **version number seven times** in Settings to enable developer mode, then open Debug for live logs, crash reports, or diagnostic report export
3. For persistent logs, enable file logging in Settings and restart the app before reproducing the issue. The file logging switch takes effect after restart; log-level changes apply immediately
4. Open [Issues](https://github.com/cwuom/NeriPlayer-Desktop/issues) and use the appropriate template

For playback or lyric issues, include the platform, recording version, and actions taken.
For interoperability issues, include desktop, Android, and server versions.
Check logs for personal paths, temporary playback URLs, and credentials before sharing;
do not attach full configuration exports or databases.

---

## Known Issues

### Network

- Search, login, playback, and lyrics may be affected by connectivity, proxies, region, account permissions, or API changes.
  Check whether the content is available on the source platform, then check the app's login state and proxy settings.
- Proxy settings include a “Bypass system proxy” option. Choose according to your network rather than always enabling or disabling it.
- Debug connectivity probes only show that the transport received a response; they do not prove successful API requests, account authorization, or playback.

### Limitations

- **Preview and updates**: there is no in-app automatic updater. Install the relevant Release manually.
- **Decoding and audio devices**: format support depends on built-in and loaded FFmpeg decoders;
  output and media-key behavior also depend on the OS and device.
- **Download recovery**: unfinished tasks download again without byte-range resume.
  Metadata processing failures can also leave an already-transferred task marked as failed.
- **Lyric matching**: word timing, translations, and phonetics depend on source data.
  Complete timing or translations cannot be generated when the corresponding content is absent.
- **System integration**: tray behavior, click-through, global shortcuts, and media sessions depend on the OS and desktop environment.
  Shortcuts may conflict with other applications.
- **Linux rendering**: proprietary NVIDIA drivers trigger WebKitGTK compatibility handling and reduced full-page transitions.
  If flickering remains, follow the [contribution guide](./CONTRIBUTING_EN.md).
- **Sync boundary**: remote sync transfers metadata only. Prepare local audio, downloads, device paths, and account authorization separately.
  Back up and confirm client compatibility before V4 upgrades or concurrent multi-device use.

---

## Privacy

- The project does not provide its own public cloud media distribution service or include advertising SDKs, third-party analytics, or third-party crash-analysis SDKs.
- Playlists, favorites, history, the queue, playback stats, settings, caches, and authorization data are stored on your device by default.
  This local-storage boundary does not prevent network requests or session sharing when you use online features.
- When you enable GitHub/WebDAV sync, metadata is written to your configured remote target.
  Audio, downloaded files, platform cookies, and playback tokens are not uploaded to the maintainers.
- Local playback history and stats are not written back to third-party music platforms.
  Login, search, playback, comments, and other requests are still processed by the respective platforms.
- Listen Together exchanges session information with the configured server and room members.
  The default server is `https://neriplayer.hancat.work`, and temporary playback URL sharing is enabled by default.
  You can change the server and disable URL sharing in Settings.
- Release builds use application-encrypted files for platform authorization and sync credentials.
  Debug builds use plaintext credential files; the two have different security boundaries.
- Full configuration exports are plaintext JSON that may contain cookies, tokens, and passwords.
  Store them securely and avoid sharing them publicly.
  Local databases, logs, and crash reports may also contain personal library details and paths.
- Third-party platforms and sync services control their own access logs, retention policies, and risk-control rules.

---

## Reference

<table>
<tr>
  <td><a href="https://github.com/cwuom/NeriPlayer">NeriPlayer Android</a></td>
  <td>Features, sync models, Listen Together protocol, and native DSP sources</td>
</tr>
<tr>
  <td><a href="https://github.com/amll-dev/applemusic-like-lyrics">applemusic-like-lyrics</a></td>
  <td>Word-synced lyric rendering and fluid background components</td>
</tr>
<tr>
  <td><a href="https://github.com/chaunsin/netease-cloud-music">netease-cloud-music</a></td>
  <td>NetEase API implementation reference</td>
</tr>
<tr>
  <td><a href="https://github.com/SocialSisterYi/bilibili-API-collect">bilibili-API-collect</a></td>
  <td>Bilibili API documentation</td>
</tr>
<tr>
  <td><a href="https://github.com/yt-dlp/ejs">ejs</a></td>
  <td>YouTube JavaScript challenge-solving reference and bundled resources</td>
</tr>
<tr>
  <td><a href="https://lrclib.net">LRCLIB</a></td>
  <td>Open lyrics service</td>
</tr>
<tr>
  <td><a href="https://ffmpeg.org/">FFmpeg</a></td>
  <td>Extended audio decoding and demuxing</td>
</tr>
</table>

---

## Update Cycle

- The project is under active Preview development. Releases follow feature work and maintenance needs.
- Core playback, local data, lyrics, sync, and cross-platform compatibility remain under maintenance.
- CI builds let you try work in progress. Refer to the relevant commit and release notes for changes and known issues.

---

## Support

- The project and its maintainers do not accept sponsorships, donations, or commercial funding.
- Support the project with reproducible Issues, PRs, documentation improvements, or Preview validation.

---

## License

NeriPlayer Desktop is released under **GPL-3.0-or-later**. See [LICENSE](./LICENSE) for the terms.

- Code released under MIT before 2026-10-07 retains its original notice in [LICENSES/MIT.txt](./LICENSES/MIT.txt).
- `vendor/applemusic-like-lyrics` is distributed under its AGPL-3.0 license.
- The ported native DSP follows its source GPL-3.0 license; see the [DSP notes](./src-tauri/native/dsp/README.md).
  The Android USB code's alternative license does not apply.
- The bundled minimal FFmpeg build does not enable GPL components and is distributed under LGPL-2.1-or-later.
  See the [FFmpeg notes](./src-tauri/native/ffmpeg/README.md) and Release attachments for sources, build options, and source access.
- Third-party resources and source code retain their own licenses.
  The repository's main license does not relicense every dependency.

---

# Contributing to NeriPlayer Desktop

Before contributing, read the full [CONTRIBUTING_EN.md](./CONTRIBUTING_EN.md) and follow the
[Code of Conduct](./CODE_OF_CONDUCT.md).

---

## Star History

<a href="https://www.star-history.com/?repos=cwuom%2FNeriPlayer-Desktop&type=timeline&legend=bottom-right">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=cwuom/NeriPlayer-Desktop&type=timeline&theme=dark&legend=bottom-right" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=cwuom/NeriPlayer-Desktop&type=timeline&legend=bottom-right" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=cwuom/NeriPlayer-Desktop&type=timeline&legend=bottom-right" />
  </picture>
</a>
