# FFmpeg 解码测试素材

全部由 `generate.ps1` 用 lavfi 合成的正弦波编码生成，不含外部音乐或下载内容，遵循仓库许可证。
生成时使用 `-bitexact`，同一版本的 FFmpeg 会得到逐字节相同的文件。

生成环境：FFmpeg n9.0.2-22-g46d8f462ee（BtbN win64 gpl-shared 构建，libavcodec 63.1.102）。

| 文件 | 字节数 | SHA-256 | 用途 |
| --- | ---: | --- | --- |
| aac-stereo-1s.m4a | 17215 | a838236d28b0808d238f286e8b39c0790370cff8a2419e8552f69ff0bc48bcaa | AAC 编码器延迟与尾部填充裁剪 |
| eac3-5.1-center.mp4 | 13218 | c8dce07c84a82a9ae9d9ac368cb69ffe7f892e83683db68a12d670a923199cfd | 只有中置声道的 E-AC-3 5.1 分片 MP4，验证下混不丢对白 |
| flac-s16-stereo-0.5s.flac | 25992 | 6bf8679becedc38f3a113eaa1b09cf066dbe69735025247fc32b31da31349984 | 无损逐样本比对、精确 seek |
| mp3-stereo-1s.mp3 | 17155 | c192bb8cb3379378819f53e46d9ceee4a16953efe056d585313dcd3231d82b82 | LAME 头里的延迟与填充裁剪 |
| opus-stereo-1s.webm | 12376 | a15d7a346cbcb658dc6ec47dbcecd8e1c425f8a0d3f84f2fceecb36f321e03ce | WebM/Opus（YouTube 251 的封装）与 pre-skip |
| pcm-s16-stereo-0.5s.wav | 96044 | e4a62c1a41d82f1a66be000add2fbb185052814abc994b6a349a4c0d3d6b1616 | FLAC 比对的源 |

```powershell
pwsh src-tauri/src/audio/fixtures/ffmpeg/generate.ps1
```
