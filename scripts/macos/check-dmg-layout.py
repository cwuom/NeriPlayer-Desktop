"""核对已挂载 DMG 的 Finder 布局：背景图、图标位置和隐藏的窗口装饰都写进了 .DS_Store。

CI 上 Tauri 会跳过设置 Finder 外观的步骤，打出来的 DMG 看起来正常却没有背景；
发布前用它确认 build-dmg.sh 生成的 DMG 真的带着布局。

用法：python check-dmg-layout.py <挂载目录> <tauri.conf.json>
需要 ds_store 与 mac_alias（packaging/macos/requirements-dmg.txt）。
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from ds_store import DSStore
from mac_alias import Alias


def read(store: DSStore, name: str, code: str, errors: list[str]):
    """读一条 .DS_Store 记录；缺失或解不开都算布局不对，不抛异常"""
    try:
        return store[name][code]
    except (KeyError, ValueError) as error:
        errors.append(f"{name}: no readable {code} record ({error})")
        return None


def main() -> int:
    mount, config = Path(sys.argv[1]), Path(sys.argv[2])
    dmg = json.loads(config.read_text(encoding="utf-8"))["bundle"]["macOS"]["dmg"]
    apps = sorted(mount.glob("*.app"))
    if len(apps) != 1:
        print(f"expected one .app in {mount}, found {[a.name for a in apps]}", file=sys.stderr)
        return 1
    expected = {
        apps[0].name: (dmg["appPosition"]["x"], dmg["appPosition"]["y"]),
        "Applications": (dmg["applicationFolderPosition"]["x"], dmg["applicationFolderPosition"]["y"]),
    }

    errors = []
    if not (mount / "Applications").is_symlink():
        errors.append("missing Applications link")
    with DSStore.open(str(mount / ".DS_Store"), "r") as store:
        view = read(store, ".", "icvp", errors) or {}
        # backgroundType 2 = 用图片作背景
        if view.get("backgroundType") != 2 or not view.get("backgroundImageAlias"):
            errors.append(f"background image is not set: {view.get('backgroundType')!r}")
        else:
            # 别名里记的是卷内路径，确认它指向的图片真的打进了 DMG
            target = Alias.from_bytes(view["backgroundImageAlias"]).target.posix_path
            if not (mount / target.lstrip("/")).is_file():
                errors.append(f"background alias points to missing {target}")
        for name, position in expected.items():
            location = read(store, name, "Iloc", errors)
            if location is not None and tuple(location) != position:
                errors.append(f"{name} at {tuple(location)}, expected {position}")
        window = read(store, ".", "bwsp", errors) or {}
        if window.get("ShowToolbar", True) or window.get("ShowSidebar", True):
            errors.append("toolbar or sidebar is visible")

    for error in errors:
        print(f"DMG layout: {error}", file=sys.stderr)
    if not errors:
        print(f"DMG layout ok: background, {len(expected)} icon positions, plain window")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
