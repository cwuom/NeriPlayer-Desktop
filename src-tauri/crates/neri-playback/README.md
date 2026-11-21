[English](#english) | [中文](#中文)

# neri-playback

## 中文

播放引擎。依赖 `neri-common`、`neri-model`、`neri-platform`（取流请求头策略），不依赖 Tauri。

| 模块 | 职责 |
| --- | --- |
| `player` | `PlayerEngine`：cpal 输出、控制循环、交叉淡化与无缝切换 |
| `remote` | 远程拉流：分段并发预取、磁盘缓存、seek 索引、`SymphoniaAudioDecoder` |
| `hls` | HLS 播放列表与分片（含加密分片） |
| `decoder` / `ffmpeg` | symphonia 解码，以及运行时加载 FFmpeg 的兜底解码 |
| `effects` / `dsp` / `dsp_settings` / `stretch` | 原生 DSP 音效链、参数与变速不变调 |
| `analyzer` / `metrics` | 推给前端可视化的电平数据；输出回调与起播耗时指标 |
| `queue` | 播放队列与随机/循环策略 |
| `media_session` | 系统媒体会话（souvlaki） |
| `runtime` | 宿主注入的 tokio 运行时；未注入时（单元测试）惰性创建独立运行时 |

`build.rs` 用 `cc` 编译 `native/dsp`（C++17 DSP 引擎）和 `native/ffmpeg`（FFmpeg 运行时加载垫片，只依赖仓库内固定版本的头文件）。

约束：

- 宿主必须在启动时调用 `runtime::install`，让远程拉流与命令处理共用一个运行时和 reqwest 连接池。
- 宿主侧触碰 `PlayerEngine` 锁的命令走 `run_player_blocking`，不要在异步命令里直接持锁。
- 音频/解码线程上不能 panic；测试用的音频样本在 `src/fixtures/`，生成方法见其中的 README。

```bash
cargo test -p neri-playback
```

## English

Playback engine. Depends on `neri-common`, `neri-model`, and `neri-platform` (stream request header policy); not on Tauri.

| Module | Responsibility |
| --- | --- |
| `player` | `PlayerEngine`: cpal output, control loop, crossfade and gapless transitions |
| `remote` | Remote streaming: concurrent ranged prefetch, disk cache, seek index, `SymphoniaAudioDecoder` |
| `hls` | HLS playlists and segments (including encrypted segments) |
| `decoder` / `ffmpeg` | symphonia decoding and the runtime-loaded FFmpeg fallback |
| `effects` / `dsp` / `dsp_settings` / `stretch` | Native DSP effect chain, parameters, and time stretching |
| `analyzer` / `metrics` | Level data for frontend visualizers; output callback and startup latency metrics |
| `queue` | Play queue with shuffle/repeat policies |
| `media_session` | System media session (souvlaki) |
| `runtime` | Host-injected tokio runtime; falls back to a private runtime when none is installed (unit tests) |

`build.rs` compiles `native/dsp` (C++17 DSP engine) and `native/ffmpeg` (runtime FFmpeg loader shim that only needs the pinned headers in the repository) with `cc`.

Constraints:

- The host must call `runtime::install` at startup so remote streaming and command handling share one runtime and reqwest connection pool.
- Host commands that touch the `PlayerEngine` lock use `run_player_blocking`; never hold the lock inside an async command.
- Never panic on the audio/decode threads. Test audio samples live in `src/fixtures/`; see the README there for how they were generated.

```bash
cargo test -p neri-playback
```
