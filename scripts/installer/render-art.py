"""生成 macOS DMG 安装窗口的背景图。

图上不放文字：拖拽箭头本身就能看懂，也避免为每种语言各出一套图。
Windows 安装界面由 packaging/windows/nsis 的脚本绘制，不需要位图。
只依赖 Pillow 与 macOS 自带的 tiffutil（把 1x/2x 合成 Retina 背景）。

用法：python3 scripts/installer/render-art.py
"""

from __future__ import annotations

import math
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
ICON = ROOT / "app-icon.png"
OUT = ROOT / "packaging"

NAVY = (31, 42, 60)
PAPER_TOP = (248, 248, 250)
PAPER_BOTTOM = (237, 239, 244)

# DMG 窗口与图标中心，需与 tauri.conf.json 的 bundle.macOS.dmg 保持一致
DMG_SIZE = (660, 400)
DMG_APP = (180, 190)
DMG_APPLICATIONS = (480, 190)


def vertical_gradient(size: tuple[int, int], top: tuple[int, int, int], bottom: tuple[int, int, int]) -> Image.Image:
    width, height = size
    image = Image.new("RGB", size)
    draw = ImageDraw.Draw(image)
    for y in range(height):
        t = y / max(1, height - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        draw.line([(0, y), (width, y)], fill=color)
    return image


def soft_pad(canvas: Image.Image, center: tuple[float, float], radius: float, scale: int) -> None:
    """图标下方的柔和底座，让两个落点更醒目"""
    layer = Image.new("L", canvas.size, 0)
    ImageDraw.Draw(layer).ellipse(
        [center[0] - radius, center[1] - radius, center[0] + radius, center[1] + radius], fill=255
    )
    layer = layer.filter(ImageFilter.GaussianBlur(18 * scale))
    tint = Image.new("RGB", canvas.size, (226, 230, 238))
    canvas.paste(tint, mask=layer.point(lambda v: v * 0.55))


def arrow(canvas: Image.Image, start: tuple[float, float], end: tuple[float, float], scale: int) -> None:
    """略向上拱的细箭头；先按 4 倍尺寸画再缩小，得到平滑边缘"""
    ss = 4
    big = Image.new("RGBA", (canvas.width * ss, canvas.height * ss), (0, 0, 0, 0))
    draw = ImageDraw.Draw(big)
    width = round(3.5 * scale * ss)
    color = (*NAVY, 135)
    (x0, y0), (x1, y1) = start, end
    lift = 22 * scale
    steps = 80
    points = []
    for i in range(steps + 1):
        t = i / steps
        x = x0 + (x1 - x0) * t
        y = y0 + (y1 - y0) * t - math.sin(math.pi * t) * lift
        points.append((x * ss, y * ss))

    def dot(point: tuple[float, float]) -> None:
        r = width / 2
        draw.ellipse([point[0] - r, point[1] - r, point[0] + r, point[1] + r], fill=color)

    draw.line(points, fill=color, width=width, joint="curve")
    dot(points[0])
    # 箭头：沿曲线末端切线方向张开的两笔
    tip = points[-1]
    before = points[-4]
    angle = math.atan2(tip[1] - before[1], tip[0] - before[0])
    head = 13 * scale * ss
    for side in (-1, 1):
        a = angle + math.pi + side * math.radians(40)
        wing = (tip[0] + head * math.cos(a), tip[1] + head * math.sin(a))
        draw.line([tip, wing], fill=color, width=width)
        dot(wing)
    dot(tip)
    small = big.resize(canvas.size, Image.LANCZOS)
    canvas.paste(small, mask=small)


def dmg_background(scale: int) -> Image.Image:
    size = (DMG_SIZE[0] * scale, DMG_SIZE[1] * scale)
    canvas = vertical_gradient(size, PAPER_TOP, PAPER_BOTTOM)
    for center in (DMG_APP, DMG_APPLICATIONS):
        soft_pad(canvas, (center[0] * scale, center[1] * scale), 78 * scale, scale)
    arrow(
        canvas,
        ((DMG_APP[0] + 92) * scale, DMG_APP[1] * scale),
        ((DMG_APPLICATIONS[0] - 92) * scale, DMG_APPLICATIONS[1] * scale),
        scale,
    )
    return canvas


def save_dmg_background() -> None:
    target = OUT / "macos" / "dmg-background.tiff"
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as work:
        one = Path(work) / "bg.png"
        two = Path(work) / "bg@2x.png"
        dmg_background(1).save(one, dpi=(72, 72))
        dmg_background(2).save(two, dpi=(144, 144))
        if shutil.which("tiffutil"):
            # 1x 与 2x 合成一个多分辨率 TIFF，Finder 在 Retina 屏上自动取 2x
            subprocess.run(["tiffutil", "-cathidpicheck", str(one), str(two), "-out", str(target)], check=True)
        else:
            Image.open(one).save(target, dpi=(72, 72))


if __name__ == "__main__":
    save_dmg_background()
    print("installer art written to", OUT)
