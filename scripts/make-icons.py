#!/usr/bin/env python3
"""Render the MoxRay icon (crimson rounded square, white M) at the sizes the bundler wants."""
import struct
import zlib
from pathlib import Path

GLYPH = [(64, 196), (64, 62), (98, 62), (128, 110), (158, 62), (192, 62),
         (192, 196), (160, 196), (160, 116), (128, 164), (96, 116), (96, 196)]
SAMPLES = 3


def inside(x, y, poly):
    result = False
    for i, (x1, y1) in enumerate(poly):
        x2, y2 = poly[(i + 1) % len(poly)]
        if (y1 > y) != (y2 > y) and x < (x2 - x1) * (y - y1) / (y2 - y1) + x1:
            result = not result
    return result


def in_rounded_square(x, y, radius=58, lo=8, hi=248):
    if not (lo <= x <= hi and lo <= y <= hi):
        return False
    cx = min(max(x, lo + radius), hi - radius)
    cy = min(max(y, lo + radius), hi - radius)
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius


def chunk(tag, data):
    body = struct.pack(">I", len(data)) + tag + data
    return body + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


def render(size):
    scale = 256 / size
    rows = []
    for py in range(size):
        row = bytearray([0])
        for px in range(size):
            covered = glyph = 0
            for sy in range(SAMPLES):
                for sx in range(SAMPLES):
                    x = (px + (sx + 0.5) / SAMPLES) * scale
                    y = (py + (sy + 0.5) / SAMPLES) * scale
                    if in_rounded_square(x, y):
                        covered += 1
                        glyph += inside(x, y, GLYPH)
            alpha = covered / SAMPLES**2
            white = glyph / max(covered, 1)
            t = (px + py) / (2 * size)
            base = (229 - (229 - 143) * t, 72 - (72 - 31) * t, 82 - (82 - 42) * t)
            rgb = tuple(int(c * (1 - white) + 255 * white) for c in base)
            row += bytes(rgb) + bytes([int(255 * alpha)])
        rows.append(bytes(row))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(b"".join(rows), 9))
    return png + chunk(b"IEND", b"")


if __name__ == "__main__":
    out = Path(__file__).resolve().parent.parent / "icons"
    for name, size in {"32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512}.items():
        (out / name).write_bytes(render(size))
        print("wrote", name)
