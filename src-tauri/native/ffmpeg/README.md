# 🎧 FFmpeg 构建与运行时加载

NeriPlayer Desktop 预览版使用 `neri_ffmpeg.cpp` 提供 C 接口，由 `src-tauri/build.rs` 通过 `cc` 编入程序。编译垫片只需要仓库头文件和 C++17 编译器，**不需要本机安装 FFmpeg 开发包**；实际解码在运行时加载共享库。它不是 `ffmpeg` 命令行程序，也不从系统 PATH 查找命令。

Rust 入口为 [audio/ffmpeg.rs](../../src/audio/ffmpeg.rs)，解码选择见 [audio/decoder.rs](../../src/audio/decoder.rs)。缺库或版本不兼容会记录原因并尝试内置 Symphonia 后端，不能因此保证所有格式仍可播放。普通 `pnpm tauri build` 不会自动下载或加入这些库；正式分发流程需合并资源配置。

## 📂 共享库与搜索顺序

| 平台 | 需要同目录放置的共享库 |
| --- | --- |
| Windows | `avutil-61.dll`、`swresample-7.dll`、`avcodec-63.dll`、`avformat-63.dll` |
| Linux | `libavutil.so.61`、`libswresample.so.7`、`libavcodec.so.63`、`libavformat.so.63` |
| macOS | `libavutil.61.dylib`、`libswresample.7.dylib`、`libavcodec.63.dylib`、`libavformat.63.dylib` |

垫片直接加载 avutil、avcodec、avformat，swresample 是 avcodec / Opus 解码所需依赖，不能遗漏。动态库需与应用的操作系统、架构一致。当前头文件版本为 avutil 61.1.102、avcodec 63.1.102、avformat 63.1.102；运行时检查主版本相同、次版本不低于头文件版本。仅文件名相同并不足以证明 ABI 与组件兼容。

依次尝试以下目录：

1. `NERI_FFMPEG_DIR` 环境变量指定的目录。
2. 应用启动时由 Tauri `resource_dir()` 设置的资源目录下 `ffmpeg/`。
3. 可执行文件旁的 `ffmpeg/`，以及可执行文件所在目录；macOS 另查 `../Frameworks`。
4. 仅 Debug：仓库 `.cache/ffmpeg-build/<os>-<arch>/`，再查 `.cache/ffmpeg/<os>-<arch>/bin`（Windows）或 `lib`（其它平台）。
5. 仅 Linux，最后兜底：发行版的系统库目录 `/usr/lib/<多架构三元组>`、`/usr/local/lib`、`/usr/lib64`、`/usr/lib`、`/lib64`、`/lib`。

匹配后使用第一份成功加载的库。结果在进程内缓存，调整目录或替换库后重启应用/测试进程。Release 不自动查开发缓存；在开发缓存里放库不能证明安装包已经包含它们。

第 5 条排在随包目录之后，只是给「发行版已经装了同版本 FFmpeg」的机器兜底：Arch 的 `ffmpeg 9.0.2` 正好提供 avutil 61 / avcodec 63 / avformat 63，实测可直接解码 Opus、E-AC-3、FLAC。版本检查仍然生效——主版本不同或次版本低于头文件时该目录会被判为失败并记进原因，不会被误用，也不会与随包库混用（一次只加载同一个目录里的三个库）。

## 🏗️ 准备共享库

从仓库根目录运行 [build-ffmpeg.sh](../../../scripts/ffmpeg/build-ffmpeg.sh)：

```bash
bash scripts/ffmpeg/build-ffmpeg.sh <FFmpeg目标>
```

脚本固定下载 FFmpeg **9.0.2** 源码并校验脚本中的 SHA-256；源码签名曾被核对的记录在脚本注释中，构建时执行的是哈希校验，不是重新在线验签。它只启用所列音频解码器、解封装器和 parser，不构建 CLI、网络、编码器或滤镜；不会加入 `--enable-gpl` / `--enable-nonfree`。

| FFmpeg 目标 | 应用 Rust 目标 | 构建方式 |
| --- | --- | --- |
| `linux-x86_64` | `x86_64-unknown-linux-gnu` | Linux 本机 GCC、make、binutils |
| `windows-x86_64` | `x86_64-pc-windows-msvc` | Linux / WSL 交叉编译，CI 使用 llvm-mingw |
| `macos-aarch64` | `aarch64-apple-darwin` | macOS clang，默认最低系统配置 11.0 |
| `macos-x86_64` | `x86_64-apple-darwin` | macOS clang，默认最低系统配置 10.15 |

工具还包括 Bash、curl、tar/xz、make、SHA-256 工具与常规 Unix 工具。x86 汇编优化需要 nasm，CI 在 Linux/macOS 均安装 nasm；macOS 还使用 codesign。llvm-mingw 版本、下载地址与校验值以 [ffmpeg.yml](../../../.github/workflows/ffmpeg.yml) 为准。

Linux 本机示例：

```bash
bash scripts/ffmpeg/build-ffmpeg.sh linux-x86_64
```

在已安装 llvm-mingw 的 Linux / WSL 环境构建 Windows 库：

```bash
CROSS_CC=x86_64-w64-mingw32-clang bash scripts/ffmpeg/build-ffmpeg.sh windows-x86_64
```

也可从本仓库可信的 FFmpeg / CI / Artifacts / Release 工作流下载匹配目标的 `ffmpeg-<目标>` artifact，将内容解压至 `.cache/ffmpeg-build/<目标>/`。保留四个共享库、`LICENSE.txt` 和 `BUILDINFO.txt`；不要混合不同架构或来源的库。

脚本输出：

- `.cache/ffmpeg-build/<目标>/`：四个共享库、LGPL `LICENSE.txt`、`BUILDINFO.txt`，后者记录源码地址、哈希、configure 参数及每个库的 SHA-256。
- `.cache/ffmpeg-build/work/`：源码、构建和安装中间目录。

脚本检查所有指定组件是否实际启用，缺组件会失败；Linux 校验 `$ORIGIN` RUNPATH，macOS 对 strip 后的 dylib 重新 ad-hoc 签名。脚本重建会替换自身的目标输出与中间目录，不要把用户文件放在那里。

可选配置：第二个参数改变输出根目录，`FFMPEG_WORK_DIR` 改工作目录，`JOBS` 控制并发，`CROSS_PREFIX` / `CROSS_CC` 改 Windows 工具链，`MACOSX_DEPLOYMENT_TARGET` 改 macOS 下限。`FFMPEG_DISABLE_ASM=1` 可用于无 nasm 的本地检查，但会降低性能；分发构建应与工作流保持一致。多个目标同时构建时分配不同 `FFMPEG_WORK_DIR`，避免共享源码目录被相互删除。

## 📦 合并安装包配置

[bundle-config.mjs](../../../scripts/ffmpeg/bundle-config.mjs) 根据 `BUILDINFO.txt` 逐个核对库文件和 SHA-256，输出 Tauri 合并配置。所有命令从仓库根目录运行。

Windows PowerShell（先准备上面的 Windows 库）：

```powershell
node scripts/ffmpeg/bundle-config.mjs windows-x86_64
pnpm tauri build --config .cache/ffmpeg-build/windows-x86_64/tauri.bundle.json -- --locked
```

Linux：

```bash
node scripts/ffmpeg/bundle-config.mjs linux-x86_64
pnpm tauri build --config .cache/ffmpeg-build/linux-x86_64/tauri.bundle.json -- --locked
```

macOS Apple Silicon（需要相应 Rust target）：

```bash
bash scripts/ffmpeg/build-ffmpeg.sh macos-aarch64
node scripts/ffmpeg/bundle-config.mjs macos-aarch64
MACOSX_DEPLOYMENT_TARGET=11.0 pnpm tauri build --target aarch64-apple-darwin --bundles dmg \
  --config .cache/ffmpeg-build/macos-aarch64/tauri.bundle.json \
  --config '{"bundle":{"macOS":{"minimumSystemVersion":"11.0"}}}' -- --locked
```

Intel 对应 `macos-x86_64`、`x86_64-apple-darwin` 和 `10.15`。Windows/Linux 库放进 Tauri 资源目录 `ffmpeg/`；macOS 库放入 `Contents/Frameworks`，许可证与构建信息放入 `Contents/Resources/ffmpeg/`。基础 `tauri.conf.json` 不直接登记这些资源，保持没有共享库的开发环境仍能编译。

自定义目录时：

```bash
node scripts/ffmpeg/bundle-config.mjs <目标> <输出根目录> <配置文件路径>
pnpm tauri build --config <配置文件路径> -- --locked
```

`NERI_FFMPEG_DIR` 只控制运行时查找，不会将库加入安装包。macOS 使用 ad-hoc 签名，非 Apple 公证；Release 工作流会核查随包 dylib 签名和架构。自行修改签名方式时，保持应用与共享库的签名及加载约束一致。

## 🧪 验证与诊断

配置生成逻辑检查：

```bash
pnpm test:ffmpeg-bundle
```

该测试使用模拟库文件验证配置与校验逻辑，不能证明实际 FFmpeg 可以加载或解码。Windows 原生解码测试示例：

```powershell
$env:NERI_FFMPEG_DIR = (Resolve-Path .cache/ffmpeg-build/windows-x86_64).Path
$env:NERI_REQUIRE_FFMPEG = '1'
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib audio::ffmpeg
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib audio::decoder
Remove-Item Env:NERI_FFMPEG_DIR
Remove-Item Env:NERI_REQUIRE_FFMPEG
```

Linux/macOS：

```bash
NERI_FFMPEG_DIR="$PWD/.cache/ffmpeg-build/<目标>" NERI_REQUIRE_FFMPEG=1 \
  cargo test --manifest-path src-tauri/Cargo.toml --locked --lib audio::ffmpeg
NERI_FFMPEG_DIR="$PWD/.cache/ffmpeg-build/<目标>" NERI_REQUIRE_FFMPEG=1 \
  cargo test --manifest-path src-tauri/Cargo.toml --locked --lib audio::decoder
```

未设 `NERI_REQUIRE_FFMPEG=1` 时，部分依赖 FFmpeg 的测试缺库会跳过。`build.yml` 使用随包构建产物和该变量，在 Windows x64、macOS arm64、Linux x64 上执行解码检查；macOS Intel 当前只有对应 Rust check 与构建流程，不能据此声称具有同样的解码测试覆盖。

遇到加载失败，检查目录、操作系统/架构、版本、swresample 依赖和库签名，并查 `audio-decoder` 日志。修改后以新进程重跑。验证交付包时还需检查解压后的资源、实际启动和播放/seek/切歌；仓库缓存中的解码测试不能代替安装包运行验证。

## ⚖️ 许可与升级

共享库为未启用 GPL/nonfree 组件的 **LGPL-2.1-or-later** 构建；保留许可证、构建信息和完整对应源码。Release 工作流附源码包和构建脚本，不能仅分发 DLL/dylib/so 而丢弃这些资料。

`include/` 保存垫片使用的 32 个公共头文件，现有来源记录为 FFmpeg `n9.0.2-22-g46d8f462ee`；每个头文件遵循自身 LGPL 声明。更新时：

1. 从确定版本获取完整头文件，保留许可与来源。
2. 用 MSVC `/showIncludes` 或对应编译器依赖输出确认垫片实际使用的头文件，再替换所需集合。
3. 同步检查结构体布局、函数签名与 Rust C 接口；主版本变化同步库名、版本说明、测试、构建脚本版本与源码哈希。
4. 重新核查解码器/容器/parser 清单，执行随包库解码测试、配置检查和平台安装包验证。

不要单独替换版本宏或仅为了通过校验修改 `BUILDINFO.txt`。主版本不匹配的系统 FFmpeg 不能替代当前随包库；主版本一致、次版本不低于头文件的系统库可以按上面的第 5 条兜底使用。
