这些二进制测试向量复制自 Android NeriPlayer 提交 `3e1abcb7` 的
`modules/sync/src/test/resources/sync/archive/{v3-frozen,v4-frozen}`。

对应 Android 测试为 `SyncArchiveVersionMigrationTest`。它们包含同一曲目的两份历史歌词、正文池、列式数字编码和不同历史日桶元数据，用于验证跨端解码与 V4 迁移，而不是由桌面端编码器自行产生的测试数据。
