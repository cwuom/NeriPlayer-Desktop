# FFmpeg 垫片

`neri_ffmpeg.cpp` 由 `build.rs` 用 `cc` 编进程序，运行时从指定目录加载 FFmpeg 动态库
（Windows：`avutil-61.dll`、`avcodec-63.dll`、`avformat-63.dll`；avcodec 还依赖同目录的
`swresample-7.dll`），找不到或版本不符时返回原因，播放器回退到 symphonia。Rust 侧只通过这里
导出的 C 接口访问 FFmpeg。

## 动态库位置

依次查找：

1. 环境变量 `NERI_FFMPEG_DIR`；
2. 安装包的资源目录下的 `ffmpeg/`（应用启动时按 Tauri 的 `resource_dir()` 设置，各平台、各包格式不同）；
3. 可执行文件旁的 `ffmpeg/` 与可执行文件所在目录；macOS 另查 `../Frameworks`（安装包放在这里）；
4. 仅调试构建：`.cache/ffmpeg-build/<os>-<arch>/`（构建脚本的输出），再是
   `.cache/ffmpeg/<os>-<arch>/bin`（Windows）或 `lib`，开发机把同一主版本的共享库放在那里即可。

`NERI_REQUIRE_FFMPEG=1` 时测试找不到动态库会失败而不是跳过。

## 随包分发的构建

`scripts/ffmpeg/build-ffmpeg.sh <target>` 从固定版本、核对过签名的 FFmpeg 源码包构建只含解码与
解封装的共享库（不加 `--enable-gpl`，LGPL-2.1-or-later），输出到
`.cache/ffmpeg-build/<target>/`，并写出 `LICENSE.txt` 与记录配置参数、各库 SHA-256 的
`BUILDINFO.txt`。configure 会悄悄丢掉缺依赖的组件，脚本会逐个核对，缺了就失败。

- `linux-x86_64`：本机 gcc；`windows-x86_64`：在 Linux 上交叉编译（CI 用 llvm-mingw，
  本地可在 WSL 里用同一份工具链，设 `CROSS_CC=x86_64-w64-mingw32-clang`）；
  `macos-aarch64`、`macos-x86_64`：在 macOS 上用 clang。都需要 nasm。
- 出安装包时 `node scripts/ffmpeg/bundle-config.mjs <target>` 核对动态库后生成
  `tauri.bundle.json`，`tauri build --config` 合并它：Windows、Linux 放进资源目录的 `ffmpeg/`，
  macOS 放进 `Frameworks`。不写进 `tauri.conf.json`，否则没编 FFmpeg 的环境连 `cargo build` 都过不了。
- CI 的 `.github/workflows/ffmpeg.yml` 构建四个目标；`build.yml` 用它们在 Windows、macOS、Linux
  上跑解码测试，发布工作流把它们打进安装包并在发布里附上 FFmpeg 源码包。

## 头文件

`include/` 是垫片实际用到的 32 个 FFmpeg 公共头文件，取自 FFmpeg n9.0.2-22-g46d8f462ee
（libavutil 61.1.102、libavcodec 63.1.102、libavformat 63.1.102）。它们只决定编译期的
结构体布局与函数签名；每个文件按其文件头声明以 LGPL-2.1-or-later 授权。

运行时要求同一主版本、次版本不低于这里的头文件，`neri_ff_load` 会校验。

## 更新头文件

1. 取新版本完整的 `include/` 目录；
2. 用 MSVC 加 `/showIncludes` 编译 `neri_ffmpeg.cpp`，列出其中位于该目录下的头文件；
3. 用这份清单替换 `include/` 下的文件，并更新上面的版本号；主版本变化时同步改 Rust 侧的说明与测试，
   以及 `scripts/ffmpeg/build-ffmpeg.sh` 里的版本与 SHA-256。
