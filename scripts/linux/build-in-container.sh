#!/usr/bin/env bash
# 在 ubuntu:22.04 容器里构建 Linux 安装包，产物的 glibc 下限固定为 2.35，
# 可以直接发给 Debian 12 / Ubuntu 22.04 / Fedora 36+ 的测试者。
#
# 用法：scripts/linux/build-in-container.sh [额外的 tauri build 参数...]
# 环境变量：
#   CONTAINER_ENGINE  docker 或 podman（默认自动探测，优先 podman）
#   NERI_BUNDLES      打包格式，默认 deb,rpm,appimage
#   NERI_GLIBC_MAX    允许的最高 GLIBC 符号版本，默认 2.35
# 产物：src-tauri/target/linux-glibc-2.35/x86_64-unknown-linux-gnu/release/bundle/
# 若 .cache/ffmpeg-build/linux-x86_64/ 已放好 FFmpeg（见 CONTRIBUTING），会一并打包。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script_dir="$repo_root/scripts/linux"
image="neri-player-linux-build:ubuntu-22.04"
target="x86_64-unknown-linux-gnu"
bundles="${NERI_BUNDLES:-deb,rpm,appimage}"
glibc_max="${NERI_GLIBC_MAX:-2.35}"

engine="${CONTAINER_ENGINE:-}"
if [[ -z "$engine" ]]; then
  if command -v podman >/dev/null; then engine=podman
  elif command -v docker >/dev/null; then engine=docker
  else echo "need podman or docker" >&2; exit 1
  fi
fi

rust_toolchain="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$repo_root/rust-toolchain.toml")"
pnpm_version="$(sed -n 's/.*"packageManager": *"pnpm@\([^"]*\)".*/\1/p' "$repo_root/package.json")"
[[ -n "$rust_toolchain" && -n "$pnpm_version" ]] || { echo "cannot read toolchain versions" >&2; exit 1; }

# 产物只面向 x86_64；Apple Silicon 等主机上走模拟
"$engine" build --platform linux/amd64 \
  --build-arg "RUST_TOOLCHAIN=$rust_toolchain" \
  --build-arg "PNPM_VERSION=$pnpm_version" \
  -t "$image" -f "$script_dir/Containerfile" "$script_dir"

# 以宿主用户身份运行，避免在仓库里留下 root 所有的文件
user_args=(--user "$(id -u):$(id -g)")
if [[ "$engine" == podman ]]; then
  user_args=(--userns=keep-id)
fi

cache="/work/.cache/linux-container"
# 交互终端下才分配 TTY；空数组写法兼容 macOS 自带的 bash 3.2
tty_args=()
[[ -t 0 && -t 1 ]] && tty_args=(-it)

"$engine" run --rm ${tty_args[@]+"${tty_args[@]}"} --platform linux/amd64 "${user_args[@]}" \
  -v "$repo_root:/work" \
  -v neri-player-linux-node-modules:/work/node_modules \
  -e HOME=/tmp/home \
  -e CARGO_HOME="$cache/cargo" \
  -e XDG_CACHE_HOME="$cache/xdg" \
  -e npm_config_store_dir="$cache/pnpm-store" \
  -e CARGO_TARGET_DIR=/work/src-tauri/target/linux-glibc-2.35 \
  -e APPIMAGE_EXTRACT_AND_RUN=1 \
  -e CI=true \
  -e NERI_GLIBC_MAX="$glibc_max" \
  -e NERI_BUNDLES="$bundles" \
  -e NERI_TARGET="$target" \
  "$image" bash -euo pipefail -c '
    mkdir -p "$HOME"
    # cargo 的 bin 目录随 CARGO_HOME 变了，rustup 代理仍在 /opt/cargo/bin
    export PATH="/opt/cargo/bin:$PATH"
    pnpm install --frozen-lockfile

    config_args=()
    ffmpeg_dir=".cache/ffmpeg-build/linux-x86_64"
    if compgen -G "$ffmpeg_dir/*.so.*" >/dev/null; then
      node scripts/ffmpeg/bundle-config.mjs linux-x86_64
      config_args+=(--config "$ffmpeg_dir/tauri.bundle.json")
    else
      echo "warning: $ffmpeg_dir not found, building without bundled FFmpeg" >&2
    fi

    pnpm tauri build --ci --target "$NERI_TARGET" --bundles "$NERI_BUNDLES" "${config_args[@]}" "$@" -- --locked

    check_paths=("$CARGO_TARGET_DIR/$NERI_TARGET/release/neri-player-desktop")
    compgen -G "$ffmpeg_dir/*.so.*" >/dev/null && check_paths+=("$ffmpeg_dir")
    bash scripts/linux/check-glibc.sh --max "$NERI_GLIBC_MAX" "${check_paths[@]}"
    echo "bundles: $CARGO_TARGET_DIR/$NERI_TARGET/release/bundle"
  ' neri-build "$@"
