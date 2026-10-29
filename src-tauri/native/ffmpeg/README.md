# FFmpeg 构建与运行时加载 / FFmpeg Runtime

本目录保存 FFmpeg C 接口垫片和编译所需的固定版本头文件。共享库由运行时加载，随包构建和资源合并由仓库脚本处理。

## 集成边界 / Integration

[neri_ffmpeg.cpp](./neri_ffmpeg.cpp) 由 [src-tauri/build.rs](../../build.rs) 通过 `cc` 编入程序。编译垫片只需要仓库头文件和 C++17 编译器，**不要求本机安装 FFmpeg 开发包**。运行时加载的是共享库，不调用 `ffmpeg` 命令行，也不从系统 PATH 查找该命令。

Rust 加载入口见 [audio/ffmpeg.rs](../../src/audio/ffmpeg.rs)，解码后端选择见 [audio/decoder.rs](../../src/audio/decoder.rs)。缺库或版本不兼容时会记录原因并尝试内置 Symphonia 后端，后备路径不保证覆盖所有格式。

普通 `pnpm tauri build` 不会自动下载或加入 FFmpeg 共享库。交付安装包时，必须先准备匹配目标的库，再合并本文的资源配置。

## 共享库与搜索顺序 / Libraries and Loading

| 平台 | 需要同目录放置的共享库 |
| --- | --- |
| Windows | `avutil-61.dll`、`swresample-7.dll`、`avcodec-63.dll`、`avformat-63.dll` |
| Linux | `libavutil.so.61`、`libswresample.so.7`、`libavcodec.so.63`、`libavformat.so.63` |
| macOS | `libavutil.61.dylib`、`libswresample.7.dylib`、`libavcodec.63.dylib`、`libavformat.63.dylib` |

垫片直接加载 avutil、avcodec、avformat；swresample 是 avcodec / Opus 解码依赖，分发时也需保留。库文件必须匹配应用的操作系统和架构。当前头文件版本为 avutil 61.1.102、avcodec 63.1.102、avformat 63.1.102；运行时要求主版本相同、次版本不低于头文件版本。文件名相同仍需满足 ABI、组件和依赖要求。

依次尝试以下目录：

1. `NERI_FFMPEG_DIR` 环境变量指定的目录。
2. 应用启动时由 Tauri `resource_dir()` 设置的资源目录下 `ffmpeg/`。
3. 可执行文件旁的 `ffmpeg/`，以及可执行文件所在目录；macOS 另查 `../Frameworks`。
4. 仅 Debug：仓库 `.cache/ffmpeg-build/<os>-<arch>/`，再查 `.cache/ffmpeg/<os>-<arch>/bin`（Windows）或 `lib`（其它平台）。
5. 仅 Linux，最后兜底：发行版的系统库目录 `/usr/lib/<多架构三元组>`、`/usr/local/lib`、`/usr/lib64`、`/usr/lib`、`/lib64`、`/lib`。

采用第一个成功加载的目录，结果在进程内缓存。调整目录或替换库后应重启应用或测试进程。Release 不自动搜索开发缓存；开发缓存中的库不会因此进入安装包。

Linux 系统目录排在随包目录之后，仅作为兜底。加载器对每个候选目录中的三个直接加载库执行版本检查，拒绝主版本不同或次版本不足的库，并记录失败原因。系统库是否可用取决于该机器实际安装的版本、组件和依赖，需通过原生解码测试确认。

## 准备共享库 / Building Libraries

从仓库根目录运行 [build-ffmpeg.sh](../../../scripts/ffmpeg/build-ffmpeg.sh)：

```bash
bash scripts/ffmpeg/build-ffmpeg.sh <FFmpeg目标>
```

脚本固定下载 FFmpeg **9.0.2**，按脚本中的 SHA-256 校验源码。脚本注释保留源码签名的历史核对记录；每次构建实际执行哈希校验，不会重新在线验签。组件清单仅启用所列音频解码器、解封装器和 parser，关闭 CLI、网络、编码器与滤镜，不加入 `--enable-gpl` / `--enable-nonfree`。

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

脚本会核对指定组件，缺组件即失败；Linux 检查 `$ORIGIN` RUNPATH，macOS 对 strip 后的 dylib 重新 ad-hoc 签名。重建会替换脚本管理的目标输出与中间目录，用户文件应放在这些目录之外。

可选配置：第二个参数改变输出根目录，`FFMPEG_WORK_DIR` 改工作目录，`JOBS` 控制并发，`CROSS_PREFIX` / `CROSS_CC` 改 Windows 工具链，`MACOSX_DEPLOYMENT_TARGET` 改 macOS 下限。`FFMPEG_DISABLE_ASM=1` 可用于无 nasm 的本地检查，但会降低性能；分发构建应与工作流保持一致。多个目标同时构建时分配不同 `FFMPEG_WORK_DIR`，避免共享源码目录被相互删除。

## 合并安装包配置 / Packaging

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

`NERI_FFMPEG_DIR` 只控制运行时查找，不负责打包资源。macOS 当前采用 ad-hoc 签名；Release 工作流检查随包 dylib 的签名和架构，这不代表 Apple 公证。修改签名方式时，应同时核查应用与共享库的签名及加载约束。

## 验证与诊断 / Verification

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

未设 `NERI_REQUIRE_FFMPEG=1` 时，部分 FFmpeg 测试遇到缺库会跳过。[build.yml](../../../.github/workflows/build.yml) 的解码 job 使用随包构建产物和该变量，矩阵包括 Windows x64、macOS arm64、Linux x64。macOS Intel 有 Rust check 与构建流程，尚未列入同一解码测试矩阵。

加载失败时，检查搜索目录、操作系统与架构、库版本、swresample 依赖和签名，再查看 `audio-decoder` 日志。替换后以新进程重跑。交付包还需检查解压后的资源、应用启动和播放、seek、切歌行为；仓库缓存中的解码测试不能代替安装包运行验证。

## 来源、许可与升级 / Provenance and Updates

共享库为未启用 GPL/nonfree 组件的 **LGPL-2.1-or-later** 构建；保留许可证、构建信息和完整对应源码。Release 工作流附源码包和构建脚本，不能仅分发 DLL/dylib/so 而丢弃这些资料。

`include/` 保存垫片使用的 32 个公共头文件，现有来源记录为 FFmpeg `n9.0.2-22-g46d8f462ee`；每个头文件遵循自身 LGPL 声明。更新时：

1. 从确定版本获取完整头文件，保留许可与来源。
2. 用 MSVC `/showIncludes` 或对应编译器依赖输出确认垫片实际使用的头文件，再替换所需集合。
3. 同步检查结构体布局、函数签名与 Rust C 接口；主版本变化同步库名、版本说明、测试、构建脚本版本与源码哈希。
4. 重新核查解码器/容器/parser 清单，执行随包库解码测试、配置检查和平台安装包验证。

版本宏应与实际头文件一致，`BUILDINFO.txt` 应准确记录分发库的来源和哈希，不应单独改写它们以绕过校验。主版本不匹配的系统 FFmpeg 不能替代当前随包库；主版本一致、次版本满足要求的系统库可进入 Linux 兜底加载流程，仍需验证实际解码。通用构建要求见根目录 [贡献指南](../../../CONTRIBUTING.md)。
