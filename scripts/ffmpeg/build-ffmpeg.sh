#!/usr/bin/env bash
# 构建播放器随包分发的精简 FFmpeg：只有解码与解封装，四个共享库（avutil、swresample、avcodec、avformat）
# 播放器只直接加载其中三个；swresample 是 Opus 解码器的依赖，随 avcodec 一起按同目录加载
#
# 用法：scripts/ffmpeg/build-ffmpeg.sh <target> [输出目录]
#   target：linux-x86_64、windows-x86_64、macos-aarch64、macos-x86_64
#   输出：<输出目录>/<target>/ 下的动态库、LICENSE.txt 与 BUILDINFO.txt，默认输出目录 .cache/ffmpeg-build
#
# 环境变量：
#   CROSS_PREFIX       windows-x86_64 交叉编译的工具链前缀，默认 x86_64-w64-mingw32-
#   CROSS_CC           交叉编译器（用 llvm-mingw 时设为 x86_64-w64-mingw32-clang）
#   FFMPEG_DISABLE_ASM 设为 1 时不用 nasm（只适合本地验证：没有汇编优化，解码更慢）
#   FFMPEG_WORK_DIR    源码与中间产物目录，默认 <输出目录>/work
#   JOBS               并行编译数
#
# 组件清单要覆盖 src-tauri/src/audio/decoder.rs 里 FFMPEG_CODECS 与嗅探规则用到的编码和容器。
# 不加 --enable-gpl / --enable-nonfree：这些组件都是 FFmpeg 自带的 LGPL 实现。
set -euo pipefail

readonly FFMPEG_VERSION=9.0.2
# 与 ffmpeg-9.0.2.tar.xz.asc 的发布签名核对过（FFmpeg release signing key FCF986EA15E6E293A5644F10B4322F04D67658D8）
readonly FFMPEG_SHA256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
readonly FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"

readonly DECODERS=(
  aac aac_latm mp3float mp3 flac vorbis opus ac3 eac3 dca alac ape wavpack
  dsd_lsbf dsd_msbf dsd_lsbf_planar dsd_msbf_planar
  pcm_s16le pcm_s16be pcm_s24le pcm_s24be pcm_s32le pcm_s32be
  pcm_f32le pcm_f32be pcm_f64le pcm_f64be pcm_u8 pcm_alaw pcm_mulaw
)
readonly DEMUXERS=(mov matroska ogg flac mp3 aac loas wav w64 aiff caf ape wv dsf iff ac3 eac3 dts)
readonly PARSERS=(aac aac_latm ac3 mpegaudio flac opus vorbis dca)

target="${1:?usage: build-ffmpeg.sh <target> [output-dir]}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
output_root="$(mkdir -p "${2:-$root/.cache/ffmpeg-build}" && cd "${2:-$root/.cache/ffmpeg-build}" && pwd)"
work="${FFMPEG_WORK_DIR:-$output_root/work}"
mkdir -p "$work"
jobs="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"

join() { local IFS=,; echo "$*"; }
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

configure_flags=(
  --disable-everything
  --disable-autodetect
  --disable-programs --disable-doc --disable-network
  --disable-avdevice --disable-avfilter --disable-swscale
  --disable-static --enable-shared --enable-pic
  --disable-debug
  --enable-decoder="$(join "${DECODERS[@]}")"
  --enable-demuxer="$(join "${DEMUXERS[@]}")"
  --enable-parser="$(join "${PARSERS[@]}")"
  --extra-version=neri
)
if [[ "${FFMPEG_DISABLE_ASM:-0}" == 1 ]]; then
  configure_flags+=(--disable-x86asm)
fi

strip_tool=strip
strip_flags=(--strip-unneeded)
case "$target" in
  linux-x86_64)
    # 几个库放在同一目录，依赖按库所在目录查找。$ORIGIN 写在命令行上要穿过 configure、
    # make 与 shell 好几层展开；放进 gcc 的响应文件就原样交给链接器
    printf '%s\n' '-Wl,-rpath,$ORIGIN' > "$work/rpath.rsp"
    configure_flags+=("--extra-ldflags=@$work/rpath.rsp")
    library_name() { echo "lib$1.so.$2"; }
    ;;
  windows-x86_64)
    cross_prefix="${CROSS_PREFIX:-x86_64-w64-mingw32-}"
    configure_flags+=(--enable-cross-compile --target-os=mingw32 --arch=x86_64 --cross-prefix="$cross_prefix")
    if [[ -n "${CROSS_CC:-}" ]]; then
      configure_flags+=(--cc="$CROSS_CC")
    fi
    # 只用 Windows 原生线程，libgcc 静态链接：DLL 不依赖 winpthread 等工具链运行库
    configure_flags+=(--enable-w32threads --extra-ldflags=-static-libgcc)
    strip_tool="${cross_prefix}strip"
    library_name() { echo "$1-$2.dll"; }
    ;;
  macos-aarch64 | macos-x86_64)
    arch="${target#macos-}"
    [[ "$arch" == aarch64 ]] && arch=arm64
    if [[ "$arch" == arm64 ]]; then default_minimum=11.0; else default_minimum=10.15; fi
    minimum="${MACOSX_DEPLOYMENT_TARGET:-$default_minimum}"
    # 安装名用 @rpath、运行路径指向库自己所在目录，放进 .app 的 Frameworks 后互相能找到
    configure_flags+=(
      --arch="$arch" --cc="clang -arch $arch" --install-name-dir=@rpath
      --extra-cflags="-mmacosx-version-min=$minimum"
      --extra-ldflags="-mmacosx-version-min=$minimum -Wl,-rpath,@loader_path"
    )
    if [[ "$(uname -m)" != "$arch" ]]; then
      configure_flags+=(--enable-cross-compile)
    fi
    strip_flags=(-x)
    library_name() { echo "lib$1.$2.dylib"; }
    ;;
  *)
    echo "unknown target: $target" >&2
    exit 2
    ;;
esac

tarball="$work/ffmpeg-${FFMPEG_VERSION}.tar.xz"
if [[ ! -f "$tarball" ]]; then
  curl -fsSL --retry 3 -o "$tarball.part" "$FFMPEG_URL"
  mv "$tarball.part" "$tarball"
fi
actual_sha256="$(sha256 "$tarball")"
if [[ "$actual_sha256" != "$FFMPEG_SHA256" ]]; then
  echo "ffmpeg-${FFMPEG_VERSION}.tar.xz sha256 mismatch: $actual_sha256" >&2
  exit 1
fi

source_dir="$work/ffmpeg-${FFMPEG_VERSION}"
build_dir="$work/build-$target"
stage="$work/stage-$target"
rm -rf "$source_dir" "$build_dir" "$stage"
tar -xJf "$tarball" -C "$work"
mkdir -p "$build_dir"
(cd "$build_dir" && "$source_dir/configure" --prefix="$stage" "${configure_flags[@]}")

# configure 会悄悄丢掉缺依赖的组件（Opus 解码器就依赖 swresample），这里逐个核对
upper() { echo "$1" | tr '[:lower:]' '[:upper:]'; }
missing=()
for decoder in "${DECODERS[@]}"; do
  grep -q "^#define CONFIG_$(upper "$decoder")_DECODER 1" "$build_dir/config_components.h" || missing+=("decoder:$decoder")
done
for demuxer in "${DEMUXERS[@]}"; do
  grep -q "^#define CONFIG_$(upper "$demuxer")_DEMUXER 1" "$build_dir/config_components.h" || missing+=("demuxer:$demuxer")
done
for parser in "${PARSERS[@]}"; do
  grep -q "^#define CONFIG_$(upper "$parser")_PARSER 1" "$build_dir/config_components.h" || missing+=("parser:$parser")
done
if (( ${#missing[@]} > 0 )); then
  echo "configure left out: ${missing[*]}" >&2
  exit 1
fi

make -C "$build_dir" -j"$jobs"
make -C "$build_dir" install

# 文件名里的主版本号以源码为准（avutil 的在 version.h，其余在 version_major.h）
major() {
  local value
  value="$(cat "$source_dir/lib$1/version_major.h" "$source_dir/lib$1/version.h" 2>/dev/null \
    | sed -n "s/^#define LIB$(upper "$1")_VERSION_MAJOR *\([0-9][0-9]*\).*/\1/p" | head -n 1)"
  if [[ -z "$value" ]]; then
    echo "cannot find the major version of lib$1" >&2
    exit 1
  fi
  echo "$value"
}
libraries=()
for library in avutil swresample avcodec avformat; do
  version="$(major "$library")"
  libraries+=("$(library_name "$library" "$version")")
done

destination="$output_root/$target"
rm -rf "$destination"
mkdir -p "$destination"
for library in "${libraries[@]}"; do
  found="$(find "$stage" -name "$library" -print -quit)"
  if [[ -z "$found" ]]; then
    echo "build did not produce $library" >&2
    exit 1
  fi
  cp -L "$found" "$destination/$library"
  "$strip_tool" "${strip_flags[@]}" "$destination/$library"
  if [[ "$target" == macos-* ]]; then
    # strip 会破坏链接时自动打上的签名，Apple Silicon 上没有有效签名的代码加载不了
    codesign --force --sign - "$destination/$library"
  fi
  if [[ "$target" == linux-x86_64 ]] && ! readelf -d "$destination/$library" | grep -q 'RUNPATH.*\[\$ORIGIN\]'; then
    echo "$library does not look up its dependencies next to itself (RUNPATH is not \$ORIGIN)" >&2
    exit 1
  fi
done
cp "$source_dir/COPYING.LGPLv2.1" "$destination/LICENSE.txt"

{
  echo "FFmpeg ${FFMPEG_VERSION} for NeriPlayer ($target)"
  echo "source: $FFMPEG_URL"
  echo "source sha256: $FFMPEG_SHA256"
  echo "license: LGPL-2.1-or-later (built without --enable-gpl and --enable-nonfree)"
  echo "configure: ${configure_flags[*]}"
  echo "libraries:"
  for library in "${libraries[@]}"; do
    echo "  $(sha256 "$destination/$library")  $library  $(wc -c < "$destination/$library" | tr -d ' ') bytes"
  done
} > "$destination/BUILDINFO.txt"
cat "$destination/BUILDINFO.txt"
