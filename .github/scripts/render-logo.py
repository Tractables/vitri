#!/usr/bin/env python3
"""Render docs/logo.svg into a checkout of the published assets branch.

Usage: python3 .github/scripts/render-logo.py <assets-checkout>
Requires CairoSVG and Pillow.
"""

import argparse
from io import BytesIO
from pathlib import Path
import shutil

import cairosvg
from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets_checkout", type=Path)
    args = parser.parse_args()
    source = Path(__file__).resolve().parents[2] / "docs/logo.svg"
    destination = args.assets_checkout / "logo"
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination / "vitri-logo.svg")
    for name, width, height, logo_width, white in [
        ("vitri-logo.png", 948, 624, 948, False),
        ("vitri-mark.png", 1024, 1024, 900, False),
        ("vitri-social-preview.png", 1280, 640, 760, True),
    ]:
        png = cairosvg.svg2png(url=str(source), output_width=logo_width)
        with Image.open(BytesIO(png)) as logo:
            background = "white" if white else (0, 0, 0, 0)
            canvas = Image.new("RGBA", (width, height), background)
            canvas.alpha_composite(logo, ((width - logo.width) // 2, (height - logo.height) // 2))
            canvas.save(destination / name)
    shutil.copyfile(destination / "vitri-logo.png", destination / "vitri-logo-horizontal.png")


if __name__ == "__main__":
    main()
