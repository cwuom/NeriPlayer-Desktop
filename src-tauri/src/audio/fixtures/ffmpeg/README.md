# FFmpeg 解码测试素材 / FFmpeg Fixtures

本目录为 [audio/ffmpeg.rs](../../ffmpeg.rs) 和 [audio/decoder.rs](../../decoder.rs) 提供解码、裁剪、下混和 seek 回归素材。全部由 [generate.ps1](./generate.ps1) 使用 lavfi 合成正弦波后编码，不含外部音乐或下载内容，遵循仓库许可证。

## 来源与完整性 / Provenance and Integrity

原生成环境记录为 FFmpeg `n9.0.2-22-g46d8f462ee`，BtbN win64 gpl-shared 构建，libavcodec 63.1.102。脚本使用 `-bitexact` 和元数据清理参数；重新生成时仍需匹配构建及编码器依赖，并核对哈希，不能仅凭 FFmpeg 版本号推定输出完全相同。

| 文件 | 字节数 | SHA-256 | 用途 |
| --- | ---: | --- | --- |
| aac-stereo-1s.m4a | 17215 | a838236d28b0808d238f286e8b39c0790370cff8a2419e8552f69ff0bc48bcaa | AAC 编码器延迟与尾部填充裁剪 |
| eac3-5.1-center.mp4 | 13218 | c8dce07c84a82a9ae9d9ac368cb69ffe7f892e83683db68a12d670a923199cfd | 只有中置声道的 E-AC-3 5.1 分片 MP4，验证下混不丢对白 |
| flac-s16-stereo-0.5s.flac | 25992 | 6bf8679becedc38f3a113eaa1b09cf066dbe69735025247fc32b31da31349984 | 无损逐样本比对、精确 seek |
| mp3-stereo-1s.mp3 | 17155 | c192bb8cb3379378819f53e46d9ceee4a16953efe056d585313dcd3231d82b82 | LAME 头里的延迟与填充裁剪 |
| opus-stereo-1s.webm | 12376 | a15d7a346cbcb658dc6ec47dbcecd8e1c425f8a0d3f84f2fceecb36f321e03ce | WebM/Opus（YouTube 251 的封装）与 pre-skip |
| pcm-s16-stereo-0.5s.wav | 96044 | e4a62c1a41d82f1a66be000add2fbb185052814abc994b6a349a4c0d3d6b1616 | FLAC 比对的源 |

## 重新生成 / Regeneration

从仓库根目录运行以下命令，脚本会覆盖本目录的六份素材并打印字节数和 SHA-256：

```powershell
pwsh src-tauri/src/audio/fixtures/ffmpeg/generate.ps1
```

默认从 `NERI_FFMPEG_DIR/ffmpeg.exe` 查找生成工具；未设置该变量时使用仓库 `.cache/ffmpeg/windows-x86_64/bin/ffmpeg.exe`。也可显式指定：

```powershell
pwsh src-tauri/src/audio/fixtures/ffmpeg/generate.ps1 -Ffmpeg C:/tools/ffmpeg/bin/ffmpeg.exe
```

生成需要包含 lavfi、AAC、E-AC-3、FLAC、libopus 和 libmp3lame 编码能力的 FFmpeg CLI。应用随包的精简共享库不包含 CLI 或编码器，不能用作该生成工具。

## 验证 / Verification

重新生成后核对表中哈希和字节数，再执行原生解码测试。需要的运行时库、`NERI_FFMPEG_DIR` 和 `NERI_REQUIRE_FFMPEG=1` 设置见 [FFmpeg 构建与运行时加载](../../../../native/ffmpeg/README.md#验证与诊断--verification)。

保留现有测试断言；生成成功只说明编码命令完成，不代表应用的解码、尾部裁剪或多声道下混检查已经通过。
