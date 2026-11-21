[English](#english) | [中文](#中文)

# neri-library

## 中文

用户数据与同步。依赖 `neri-common`、`neri-model`、`neri-platform`、`neri-playback`，不依赖 Tauri。

| 模块 | 职责 |
| --- | --- |
| `db` | SQLite 用户库（schema、迁移、旧版 JSON 导入、缓存表） |
| `library` | 歌单、收藏、播放历史、播放队列快照、歌词偏移、本地音乐扫描与文件夹监视索引 |
| `lyrics` | 歌词获取（网易云/QQ/LRCLIB/内嵌/旁挂）、匹配、清洗、LRC/TTML 解析与生成 |
| `stats` | 播放统计的存储与查询 |
| `download_metadata` / `downloads` | 下载文件的元数据旁挂、标签写入，以及下载完整性校验（长度/MD5/可解码时长） |
| `sync` | 云同步：基线快照三路合并、GitHub/WebDAV 后端、归档格式、ProtoBuf 序列化 |

宿主注入：

- `sync::state_store::SyncStateStore`：设备 ID 与因果计数器的存储（桌面端为 `sync-state.json`，实现见 `src-tauri/src/sync_state.rs`）。
- `library::local_index::start` 接收 `neri_common::events::SharedEventSink`，用于投递扫描进度与变更事件。

约束：

- 同步合并是三路合并（本地、远端、上次同步基线），不能改成最后写入者胜。
- 所有落盘走 `neri_common::fsutil` 的原子写入。
- `sync/archive/fixtures/` 是 Android 冻结样本，用于验证跨端兼容，不要重新生成。

```bash
cargo test -p neri-library
```

## English

User data and sync. Depends on `neri-common`, `neri-model`, `neri-platform`, and `neri-playback`; not on Tauri.

| Module | Responsibility |
| --- | --- |
| `db` | SQLite user database (schema, migrations, legacy JSON import, cache tables) |
| `library` | Playlists, favorites, play history, queue snapshots, lyric offsets, local music scanning and the watched-folder index |
| `lyrics` | Lyrics fetching (NetEase/QQ/LRCLIB/embedded/sidecar), matching, sanitizing, LRC/TTML parsing and writing |
| `stats` | Playback statistics storage and queries |
| `download_metadata` / `downloads` | Sidecar metadata and tag writing for downloads, plus download integrity checks (length/MD5/decodable duration) |
| `sync` | Cloud sync: three-way merge against a base snapshot, GitHub/WebDAV backends, archive format, ProtoBuf serialization |

Host injection:

- `sync::state_store::SyncStateStore` stores the device ID and causal counter (`sync-state.json` on desktop; see `src-tauri/src/sync_state.rs`).
- `library::local_index::start` takes a `neri_common::events::SharedEventSink` for scan progress and change events.

Constraints:

- Sync merging is three-way (local, remote, last synced base); never replace it with last-write-wins.
- All persistence goes through the atomic writes in `neri_common::fsutil`.
- `sync/archive/fixtures/` holds frozen Android samples used for cross-device compatibility; do not regenerate them.

```bash
cargo test -p neri-library
```
