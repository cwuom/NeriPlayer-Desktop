[English](#english) | [中文](#中文)

# neri-common

## 中文

全项目共用的基础设施。不依赖任何项目内 crate，也不依赖 Tauri。

| 模块 | 职责 |
| --- | --- |
| `error` | 统一错误 `AppError` / `AppResult`，实现 `Serialize`，命令层可直接返回给前端 |
| `fsutil` | 同目录临时文件 + rename 的原子写入、私有权限写入、损坏文件隔离。本地数据落盘都走这里 |
| `security` | 凭据存储（登录 Cookie、GitHub Token、WebDAV 密码）。Release 为应用侧加密文件，Debug 为随机路径明文文件；旧 Keychain 凭据首次读取时迁移 |
| `config_backup` | 配置备份的加密编解码（PBKDF2 + AES-256-GCM），格式与 Android 端一致 |
| `sync_failure` | 同步失败分类；`Display` 是前端按代码本地化的稳定字符串 |
| `events` | `EventSink`：领域 crate 向宿主投递事件的出口，宿主实现见 `src-tauri/src/events.rs` |

约束：

- `security` 按 `cfg(debug_assertions)` 切换实现，改动后 debug 与 release 两种配置的 `clippy` 都要通过。
- 这里只放基础设施；共享数据结构放 `neri-model`。
- `config_backup.rs` 会被 `scripts/test-config-backup-codec.mjs` 单独编译，不能引用本 crate 的其它模块。

```bash
cargo test -p neri-common
```

## English

Shared infrastructure. Depends on no project crate and not on Tauri.

| Module | Responsibility |
| --- | --- |
| `error` | Unified `AppError` / `AppResult`; implements `Serialize` so commands can return it to the frontend |
| `fsutil` | Atomic writes (same-directory temp file + rename), private-permission writes, quarantine of corrupt files. All local persistence goes through it |
| `security` | Credential storage (login cookies, GitHub token, WebDAV password). Release uses app-side encrypted files, Debug uses plaintext files at a random path; legacy Keychain entries migrate on first read |
| `config_backup` | Encrypted config backup codec (PBKDF2 + AES-256-GCM), compatible with Android |
| `sync_failure` | Sync failure classification; `Display` yields stable codes the frontend localizes |
| `events` | `EventSink`, the outlet domain crates use to emit host events; the Tauri implementation is `src-tauri/src/events.rs` |

Constraints:

- `security` switches implementation on `cfg(debug_assertions)`; changes must pass `clippy` in both debug and release.
- Infrastructure only; shared data structures belong in `neri-model`.
- `config_backup.rs` is compiled standalone by `scripts/test-config-backup-codec.mjs`, so it must not reference other modules of this crate.

```bash
cargo test -p neri-common
```
