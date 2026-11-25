#!/usr/bin/env bash
# 安装 Ubuntu 22.04 上构建 Linux 安装包所需的系统依赖。CI 容器、本地容器脚本和直接在
# Ubuntu 22.04 主机上构建都用这一份清单，避免三处各维护一份。
set -euo pipefail

packages=(
  build-essential
  ca-certificates
  curl
  file
  git
  pkg-config
  xz-utils
  # Tauri / WebKitGTK 4.1（libsoup3）
  libwebkit2gtk-4.1-dev
  librsvg2-dev
  # 托盘：运行时 dlopen，AppImage 打包时要把它带进去
  libappindicator3-dev
  # cpal、alsa crate
  libasound2-dev
  # souvlaki MPRIS、keyring Secret Service 都走 libdbus
  libdbus-1-dev
  # tokio-tungstenite native-tls
  libssl-dev
  # 打包：AppImage 需要 patchelf，可能把 xdg-open 带进包；RPM 需要 rpmbuild；校验 glibc 下限用 objdump
  patchelf
  xdg-utils
  wget
  rpm
  binutils
)

sudo_cmd=()
if [[ "$(id -u)" -ne 0 ]]; then
  sudo_cmd=(sudo)
fi

export DEBIAN_FRONTEND=noninteractive
"${sudo_cmd[@]}" apt-get update
"${sudo_cmd[@]}" apt-get install -y --no-install-recommends "${packages[@]}"
