#!/usr/bin/env bash
# 把 `tauri build --bundles app` 产出的 .app 打成带安装引导布局的 DMG，并核对布局。
#
# CI 环境里 Tauri 给 DMG 脚本加 --skip-jenkins，跳过驱动 Finder 设置背景、图标位置的
# AppleScript（让 CI 去控制 Finder 也不可靠），打出来的 DMG 没有任何布局。这里改用 dmgbuild
# 直接写 .DS_Store，不依赖 Finder。
#
# 用法：bash scripts/macos/build-dmg.sh <Rust 目标>，例如 aarch64-apple-darwin
# 输出：src-tauri/target/<目标>/release/bundle/dmg/<名称>_<版本>_<aarch64|x64>.dmg（与 Tauri 命名一致）
set -euo pipefail

target="${1:?usage: build-dmg.sh <rust-target>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bundle="$root/src-tauri/target/$target/release/bundle"

shopt -s nullglob
apps=("$bundle"/macos/*.app)
if (( ${#apps[@]} != 1 )); then
  echo "expected exactly one .app in $bundle/macos" >&2
  exit 1
fi
app="${apps[0]}"

case "$target" in
  aarch64-*) arch=aarch64 ;;
  x86_64-*) arch=x64 ;;
  *) echo "unsupported target $target" >&2; exit 1 ;;
esac
name="$(basename "$app" .app)"
version="$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")"

work="$(mktemp -d)"
mount_dir="$work/mount"
cleanup() {
  hdiutil detach "$mount_dir" -quiet 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet --disable-pip-version-check --require-hashes \
  -r "$root/packaging/macos/requirements-dmg.txt"

mkdir -p "$bundle/dmg"
rm -f "$bundle"/dmg/*.dmg
output="$bundle/dmg/${name}_${version}_${arch}.dmg"
"$work/venv/bin/dmgbuild" -s "$root/packaging/macos/dmg-settings.py" \
  -D app="$app" -D root="$root" "$name" "$output"

mkdir -p "$mount_dir"
hdiutil attach "$output" -mountpoint "$mount_dir" -nobrowse -readonly -quiet
"$work/venv/bin/python" "$root/scripts/macos/check-dmg-layout.py" "$mount_dir" "$root/src-tauri/tauri.conf.json"
# 复制进 DMG 的 .app 不能带额外的扩展属性，否则严格校验会判 ad-hoc 签名无效
codesign --verify --deep --strict "$mount_dir/$name.app"
echo "$output"
