[English](#english) | [中文](#中文)

# neri-model

## 中文

跨层共享的数据模型，不包含 I/O。

| 模块 | 职责 |
| --- | --- |
| `track` | `TrackInfo` / `TrackSource` / `RepeatMode`：命令层与前端之间的曲目结构（camelCase JSON） |
| `sync::models` | 云同步 JSON 模型（歌单、收藏、最近播放、统计、删除墓碑） |
| `sync::proto_models` | 同步数据的 ProtoBuf 编码 |
| `bili_video_skip` | B 站视频跳过规则 |

约束：

- `proto_models` 的 `#[prost(tag = ...)]` 与 Android `SyncDataModels.kt` 的 `@ProtoNumber` 手工对齐。改字段号或语义会破坏跨端同步，新增字段只能用未占用的编号。
- `TrackInfo` 的 JSON 字段名是 IPC 契约的一部分，改名要同步前端 `src/types/track.ts`。

```bash
cargo test -p neri-model
```

## English

Data models shared across layers. No I/O.

| Module | Responsibility |
| --- | --- |
| `track` | `TrackInfo` / `TrackSource` / `RepeatMode`, the track shape exchanged with the frontend (camelCase JSON) |
| `sync::models` | Cloud sync JSON models (playlists, favorites, recent plays, stats, deletion tombstones) |
| `sync::proto_models` | ProtoBuf encoding of sync data |
| `bili_video_skip` | Bilibili video skip rules |

Constraints:

- `#[prost(tag = ...)]` numbers in `proto_models` are hand-aligned with `@ProtoNumber` in Android `SyncDataModels.kt`. Changing a tag or its meaning breaks cross-device sync; new fields must use unused numbers.
- `TrackInfo` JSON names are part of the IPC contract; renames must be mirrored in `src/types/track.ts`.

```bash
cargo test -p neri-model
```
