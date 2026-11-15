"""Lay the pills rendered by the `readme_banner` test out as docs/banner.png.

    cd widgets
    set BTW_BANNER_DIR=%TEMP%\\btw-banner     (PowerShell: $env:BTW_BANNER_DIR=...)
    cargo test readme_banner
    python ..\\docs\\make_banner.py %TEMP%\\btw-banner

The pills come from the app's own drawing code with fixed sample data, at
twice the taskbar's size. Needs Pillow.
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ORDER = [
    "weather",
    "calendar",
    "mail",
    "departures",
    "news",
    "now-playing",
    "claude",
    "codex",
]
GAP = 4  # 2 px between pills on the bar, at 2x
BAR_PAD_X = 28
BAR_HEIGHT = 96  # the 48 px taskbar, at 2x
MARGIN_X = 72
MARGIN_Y = 64


def gradient(width: int, height: int) -> Image.Image:
    """A soft diagonal blend, dark enough for the pills' light text."""
    stops = [(0.0, (24, 28, 52)), (0.55, (52, 38, 92)), (1.0, (18, 64, 96))]

    def colour(t: float):
        for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
            if t <= t1:
                f = (t - t0) / (t1 - t0)
                return tuple(round(a + (b - a) * f) for a, b in zip(c0, c1))
        return stops[-1][1]

    image = Image.new("RGB", (width, height))
    pixels = image.load()
    for y in range(height):
        for x in range(width):
            pixels[x, y] = colour((x / width) * 0.8 + (y / height) * 0.2)
    # Two blurred glows for depth.
    glow = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    draw = ImageDraw.Draw(glow)
    draw.ellipse((-width * 0.1, -height * 1.2, width * 0.45, height * 1.1), fill=(120, 90, 255, 70))
    draw.ellipse((width * 0.6, -height * 0.4, width * 1.15, height * 1.6), fill=(40, 170, 220, 60))
    glow = glow.filter(ImageFilter.GaussianBlur(height / 2.5))
    return Image.alpha_composite(image.convert("RGBA"), glow)


def main(folder: str) -> None:
    pills = [Image.open(Path(folder) / f"{name}.png").convert("RGBA") for name in ORDER]
    bar_width = sum(p.width for p in pills) + GAP * (len(pills) - 1) + 2 * BAR_PAD_X
    width = bar_width + 2 * MARGIN_X
    height = BAR_HEIGHT + 2 * MARGIN_Y

    banner = gradient(width, height)

    # The taskbar: a dark glass strip with a soft shadow.
    box = (MARGIN_X, MARGIN_Y, MARGIN_X + bar_width, MARGIN_Y + BAR_HEIGHT)
    shadow = Image.new("RGBA", banner.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (box[0], box[1] + 10, box[2], box[3] + 10), radius=24, fill=(0, 0, 0, 120)
    )
    banner = Image.alpha_composite(banner, shadow.filter(ImageFilter.GaussianBlur(18)))
    bar = Image.new("RGBA", banner.size, (0, 0, 0, 0))
    ImageDraw.Draw(bar).rounded_rectangle(
        box, radius=24, fill=(30, 33, 40, 235), outline=(255, 255, 255, 28), width=2
    )
    banner = Image.alpha_composite(banner, bar)

    x = MARGIN_X + BAR_PAD_X
    for pill in pills:
        y = MARGIN_Y + (BAR_HEIGHT - pill.height) // 2
        layer = Image.new("RGBA", banner.size, (0, 0, 0, 0))
        layer.paste(pill, (x, y))
        banner = Image.alpha_composite(banner, layer)
        x += pill.width + GAP

    out = Path(__file__).with_name("banner.png")
    banner.convert("RGB").save(out, optimize=True)
    print(out, banner.size)


if __name__ == "__main__":
    main(sys.argv[1])
