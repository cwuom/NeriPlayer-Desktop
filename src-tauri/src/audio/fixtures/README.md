# HLS 解码测试素材

这些素材由 `generate.py` 直接编码全零 PCM 生成，不包含外部音乐、账号数据或下载的媒体内容。
生成脚本和素材遵循仓库许可证。播放器运行和 Rust 测试均不需要 PyAV 或外部 FFmpeg 可执行文件。

参数：AAC-LC，48000 Hz，mono，64 kbit/s 编码目标；输入 8 × 1024 个 PCM 样本。
编码器 priming 产生第 9 个 AAC 包；ADTS、fMP4 和 MPEG-TS 均实际解码为 9216 个 PCM 样本，峰值 0.0。
fMP4 初始化文件包含 `ftyp` 和 `moov`，媒体片段包含 `moof` 和 `mdat`。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| hls-silence.aac | 114 | 7b31e683cbdbeb9cdaa0a7014db3192f2f44d50c5e957baf017e82401299726e |
| hls-init.mp4 | 728 | 1ece770a7d0e2776ba71f17f6d5a1e57679cfdc8f102fa9dcaca70d0a99e7f2b |
| hls-segment.m4s | 195 | 6813f647ce971fb70b7802230606d71abffd677b3ea0e219d64b010c149becd6 |
| hls-silence.ts | 2632 | 0b5008359fdba93e105608a396534a0e478a2161d799df7496c5d6eb825c3e77 |

生成环境：Python 3.12，PyAV 19.0.0；其 wheel 内置 FFmpeg 库 libavcodec 63.1.102、libavformat 63.1.102。
PyAV 仅用于重新生成素材，可安装在任务临时目录，避免修改全局 Python：

```powershell
python -m pip install --only-binary=:all: --no-deps --target .codex/tmp/hls-fixture-tools av==19.0.0
python src-tauri/src/audio/fixtures/generate.py --pyav-path .codex/tmp/hls-fixture-tools
```

脚本生成后使用 PyAV 实际解码三种容器，并检查 codec、采样率、声道、非空 PCM 和静音峰值。
项目的 HLS 回归测试还应经过实际 Symphonia 解码，以覆盖生产播放依赖。

参考官方资料：[PyAV wheel 安装](https://pypi.org/project/av/19.0.0/)、
[音频帧和编码接口](https://pyav.basswood.io/docs/stable/api/audio.html)、
[FFmpeg MOV/MP4 分片参数](https://ffmpeg.org/ffmpeg-formats.html#mov_002c-mp4_002c-ismv)。
