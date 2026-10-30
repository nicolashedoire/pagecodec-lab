"""Generate alpha glyph cells from the bundled unmodified Inter font.

Requires Pillow. Run when updating assets; the guest does not need Python.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

root = Path(__file__).resolve().parent
for name, size, width, height, weight in [
    ("inter_small.bin", 11, 12, 14, "Regular"),
    ("inter_large.bin", 17, 20, 22, "SemiBold"),
]:
    font = ImageFont.truetype(str(root / "Inter-variable.ttf"), size)
    font.set_variation_by_name(weight)
    output = bytearray()
    for code in range(32, 91):
        image = Image.new("L", (width, height))
        draw = ImageDraw.Draw(image)
        draw.text((0, 0), chr(code), font=font, fill=255, anchor="lt")
        output.append(round(font.getlength(chr(code))))
        output.extend(image.tobytes())
    (root / name).write_bytes(output)
