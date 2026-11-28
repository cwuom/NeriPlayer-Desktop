# dmgbuild 的设置文件，由 scripts/macos/build-dmg.sh 以 `-D app=… -D root=…` 调用。
#
# dmgbuild 直接写 .DS_Store，不经过 Finder，所以在 CI 上背景、图标位置和窗口大小也能生效。
# 窗口尺寸、图标位置与背景图沿用 tauri.conf.json 的 bundle.macOS.dmg，本地 `tauri build`
# 打出来的 DMG 与 CI 一致。`defines` 由 dmgbuild 注入。
import json
import os
import plistlib

app = defines["app"]  # noqa: F821
root = defines["root"]  # noqa: F821

with open(os.path.join(root, "src-tauri", "tauri.conf.json"), encoding="utf-8") as config_file:
    dmg = json.load(config_file)["bundle"]["macOS"]["dmg"]

app_name = os.path.basename(app)
with open(os.path.join(app, "Contents", "Info.plist"), "rb") as info_file:
    icon_file = plistlib.load(info_file)["CFBundleIconFile"]
if not icon_file.endswith(".icns"):
    icon_file += ".icns"

# 与 Tauri 自带的 DMG 一致：zlib 压缩，macOS 10.15 也能打开
format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}
# 不隐藏 .app 后缀：隐藏要在包上写 com.apple.FinderInfo，codesign --strict 会判签名无效
icon = os.path.join(app, "Contents", "Resources", icon_file)
background = os.path.normpath(os.path.join(root, "src-tauri", dmg["background"]))

window_rect = ((200, 120), (dmg["windowSize"]["width"], dmg["windowSize"]["height"]))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False
icon_size = 128
text_size = 13
icon_locations = {
    app_name: (dmg["appPosition"]["x"], dmg["appPosition"]["y"]),
    "Applications": (dmg["applicationFolderPosition"]["x"], dmg["applicationFolderPosition"]["y"]),
}
