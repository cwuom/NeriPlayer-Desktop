# 同步归档测试向量 / Sync Archive Fixtures

本目录保存从 Android 端复制的冻结归档，为桌面端提供独立于自身编码器的跨端兼容性输入。

## 来源 / Provenance

测试向量来自 Android NeriPlayer 提交 `3e1abcb7` 的 `modules/sync/src/test/resources/sync/archive/{v3-frozen,v4-frozen}`，对应 Android 测试 `SyncArchiveVersionMigrationTest`。

| 目录 | 内容与用途 |
| --- | --- |
| [v3-frozen](./v3-frozen/) | 冻结 V3 manifest、对象和历史二进制输入，验证旧归档读取与迁移 |
| [v4-frozen](./v4-frozen/) | 冻结 V4 manifest 和对象，验证现有归档读取与迁移后的数据保留 |

向量包含同一曲目的两份历史歌词、正文池、列式数字编码和不同历史日桶元数据。它们不是由桌面端编码器自行生成的往返测试数据，应保留来源版本和文件内容。

## 验证 / Verification

桌面回归入口是 [archive/mod.rs](../mod.rs) 中的 `android_frozen_v3_and_v4_archives_preserve_both_historical_variants`。该用例分别读取两组归档，检查历史歌词候选和日桶元数据，并在迁移为 V4 后重新读取检查。

从仓库根目录运行这一针对性检查：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib android_frozen_v3_and_v4_archives_preserve_both_historical_variants
```

更新同步协议时，应保留冻结向量作为兼容基线。若引入新的 Android 来源向量，需记录来源提交，并补充相应解码或迁移断言。
