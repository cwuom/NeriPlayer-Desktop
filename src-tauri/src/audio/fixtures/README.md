# HLS 解码测试素材 / HLS Fixtures

本目录为 [audio/hls.rs](../hls.rs) 的回归测试提供 ADTS、fMP4 和 MPEG-TS 静音素材，全部由 [generate.py](./generate.py) 编码全零 PCM 生成，不包含外部音乐、账号数据或下载内容。生成脚本和素材遵循仓库许可证。

## 参数与文件 / Parameters and Files

编码参数为 AAC-LC、48000 Hz、mono，码率目标 64 kbit/s；输入 8 × 1024 个 PCM 样本。原生成记录中，编码器 priming 产生第 9 个 AAC 包，三种容器各解码出 9216 个 PCM 样本，峰值 0.0。

fMP4 初始化文件包含 `ftyp` 和 `moov`，媒体片段包含 `moof` 和 `mdat`。当前提交文件的字节数和 SHA-256 如下：

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| hls-silence.aac | 114 | 7b31e683cbdbeb9cdaa0a7014db3192f2f44d50c5e957baf017e82401299726e |
| hls-init.mp4 | 728 | 1ece770a7d0e2776ba71f17f6d5a1e57679cfdc8f102fa9dcaca70d0a99e7f2b |
| hls-segment.m4s | 195 | 6813f647ce971fb70b7802230606d71abffd677b3ea0e219d64b010c149becd6 |
| hls-silence.ts | 2632 | 0b5008359fdba93e105608a396534a0e478a2161d799df7496c5d6eb825c3e77 |

## 重新生成 / Regeneration

原生成环境记录为 Python 3.12、PyAV 19.0.0，其 wheel 内置 libavcodec 63.1.102 和 libavformat 63.1.102。PyAV 仅用于重新生成，播放器运行和 Rust 测试不需要 PyAV 或外部 FFmpeg 可执行文件。

以下命令从仓库根目录运行，把 PyAV 安装到任务目录，随后覆盖本目录的四份素材：

```powershell
python -m pip install --only-binary=:all: --no-deps --target .codex/tmp/hls-fixture-tools av==19.0.0
python src-tauri/src/audio/fixtures/generate.py --pyav-path .codex/tmp/hls-fixture-tools
```

## 验证 / Verification

生成脚本会打印每个文件的字节数和 SHA-256，并用 PyAV 解码三种容器，检查 codec、采样率、声道、非空 PCM 和静音峰值。重新生成后应核对差异，并在需要时同步更新上表与生成环境记录。

从仓库根目录运行 HLS 回归测试，检查生产代码使用的 Symphonia 解码路径：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib audio::hls
```

PyAV 的生成校验和播放器解码测试覆盖不同路径，更新素材时应同时检查。

## 参考资料 / References

[PyAV wheel 安装](https://pypi.org/project/av/19.0.0/)、
[音频帧和编码接口](https://pyav.basswood.io/docs/stable/api/audio.html)、
[FFmpeg MOV/MP4 分片参数](https://ffmpeg.org/ffmpeg-formats.html#mov_002c-mp4_002c-ismv)。
