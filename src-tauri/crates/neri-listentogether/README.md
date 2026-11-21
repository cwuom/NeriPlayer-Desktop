[English](#english) | [中文](#中文)

# neri-listentogether

## 中文

一起听。依赖 `neri-common`，不依赖 Tauri。

| 模块 | 职责 |
| --- | --- |
| `protocol` | 与 Android 对齐的房间消息模型（`LtSocketEnvelope` 等） |
| `session` | 房间会话状态、HTTP 凭据与 WebSocket 客户端的生命周期 |
| `ws_client` | WebSocket 读写循环、超时与断线事件 |

`ws_client` 通过 `neri_common::events::SharedEventSink` 投递 `lt:connected`、`lt:message`、`lt:protocol_error`、`lt:disconnected` 事件；每条连接带独立 `connectionId`，旧连接的收尾事件不会影响新连接。

约束：

- 协议字段与 Android 端保持一致；昵称等字段的校验规则以 Android 为准。
- 断开或 Drop 时必须终止读写两个任务，避免半开连接泄漏 socket。

```bash
cargo test -p neri-listentogether
```

## English

Listen Together. Depends on `neri-common`, not on Tauri.

| Module | Responsibility |
| --- | --- |
| `protocol` | Room message models aligned with Android (`LtSocketEnvelope` and friends) |
| `session` | Room session state, HTTP credentials, and the WebSocket client lifecycle |
| `ws_client` | WebSocket read/write loops, timeouts, and disconnect events |

`ws_client` emits `lt:connected`, `lt:message`, `lt:protocol_error`, and `lt:disconnected` through a `neri_common::events::SharedEventSink`. Every connection carries its own `connectionId`, so teardown events of an old connection never affect a new one.

Constraints:

- Keep protocol fields identical to Android; Android is the source of truth for validation rules such as nicknames.
- Both read and write tasks must stop on disconnect or drop, so half-open connections never leak sockets.

```bash
cargo test -p neri-listentogether
```
