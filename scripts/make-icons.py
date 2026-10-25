#!/usr/bin/env python3
"""Render icons/moxray.svg to the PNG sizes the bundler wants.

Needs Google Chrome (to rasterise the SVG with transparency) and Pillow (to scale it down).
Also refreshes ui/logo.svg, the mark shown in the app's sidebar.
"""
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SIZES = {"32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512}


def render_master(svg: str, size: int, out: Path) -> None:
    chrome = shutil.which("google-chrome") or shutil.which("chromium")
    if not chrome:
        raise SystemExit("google-chrome or chromium is required to rasterise the SVG")
    with tempfile.TemporaryDirectory() as tmp:
        page = Path(tmp) / "icon.html"
        page.write_text(
            f'<!doctype html><body style="margin:0;background:transparent">'
            f'<div style="width:{size}px;height:{size}px">'
            # Only the root <svg> element carries the pixel size; the background rect must not change.
            + svg.replace('width="200" height="200"', f'width="{size}" height="{size}"', 1)
            + "</div></body>"
        )
        subprocess.run(
            [chrome, "--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars",
             "--default-background-color=00000000", f"--window-size={size},{size}",
             f"--screenshot={out}", page.as_uri()],
            check=True, capture_output=True,
        )


if __name__ == "__main__":
    svg = (ROOT / "icons" / "moxray.svg").read_text()
    master = ROOT / "icons" / "icon.png"
    render_master(svg, 512, master)
    base = Image.open(master).convert("RGBA")
    for name, size in SIZES.items():
        if size != 512:
            base.resize((size, size), Image.LANCZOS).save(ROOT / "icons" / name)
        print("wrote", name)
    # Windows and macOS installers need their own container formats.
    base.save(ROOT / "icons" / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    base.save(ROOT / "icons" / "icon.icns")
    print("wrote icon.ico, icon.icns")
    shutil.copy(ROOT / "icons" / "moxray.svg", ROOT / "ui" / "logo.svg")
    print("wrote ui/logo.svg")
