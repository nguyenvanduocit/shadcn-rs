"""Each demo's interaction as an animated WebP, cut from the frames its page's script takes.

`docs --shots <dir>` (and `--dark`) runs every gallery page's `script()`: it hovers, clicks, types and lets motion play,
writing each step as `NN-step.png` (2x) beside `NN-step.txt`, whose first line holds the scenario's clock and whose
layout tree names each demo (`DialogDemo.vue: Div 768x36 at (32,88)`) and the layers it opens (`dialog-portal: ...
layer 1`). Per demo this keeps the frames where it shows whole, crops each to the demo and the layers it opened, drops
a frame that shows nothing new, and times each frame by the clock: a motion step for as long as it took, a state for
STATE_MS. A demo whose frames all look alike gets nothing: its page shows the still.

    uv run --with pillow python motion.py <shots dir> <out dir> <light|dark>

Writes <out dir>/<page>/<Demo>.webp and <out dir>/motion.json ({page: {demo: {width, height, frames}}}, 1x sizes).
"""

import json
import re
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFont

SCALE = 2
VIEW = (960, 640)
STATE_MS = 1000
LAST_MS = 1800
MOTION_MAX_S = 0.6
MAX_FRAMES = 60

SIZE = re.compile(r"(\d+(?:\.\d+)?)x(\d+(?:\.\d+)?) at \((-?\d+(?:\.\d+)?),(-?\d+(?:\.\d+)?)\)")
CLOCK = re.compile(r"clock (\d+(?:\.\d+)?)s")
CLICK = re.compile(r"(?<!outside at )click \((-?\d+(?:\.\d+)?),(-?\d+(?:\.\d+)?)\)")
KEY = re.compile(r"key Named\((\w+)\)")
KEYS = {"ArrowDown": "↓", "ArrowUp": "↑", "ArrowLeft": "←", "ArrowRight": "→", "Enter": "↵ Enter", "Escape": "Esc",
        "Backspace": "⌫", "Space": "Space", "Home": "Home", "End": "End"}
TYPE_MS = 110
FONT = str(Path(__file__).parent / "public/fonts/Geist.woff2")


def pointer(canvas, x, y, dark):
    """An arrow cursor with its tip at (x, y), where the script clicked."""
    if not (0 <= x < canvas.width and 0 <= y < canvas.height):
        return
    s = SCALE
    shape = [(0, 0), (0, 16), (4, 12.5), (7, 18.5), (9.5, 17.5), (6.5, 11.5), (11.5, 11.5)]
    pts = [(x + px * s, y + py * s) for px, py in shape]
    d = ImageDraw.Draw(canvas)
    d.polygon(pts, fill=(255, 255, 255) if dark else (10, 10, 10), outline=(10, 10, 10) if dark else (255, 255, 255), width=round(1.5 * s))


def keycap(canvas, label, dark):
    """The key the script pressed, as a kbd in the corner."""
    d = ImageDraw.Draw(canvas)
    try:
        font = ImageFont.truetype(FONT, 13 * SCALE)
    except OSError:
        font = ImageFont.load_default()
    pad = 6 * SCALE
    l, t, r, b = d.textbbox((0, 0), label, font=font)
    w, h = r - l + 2 * pad, 24 * SCALE
    x0, y0 = canvas.width - w - 8 * SCALE, canvas.height - h - 8 * SCALE
    fg, bg, border = ((250, 250, 250), (38, 38, 38), (64, 64, 64)) if dark else ((10, 10, 10), (245, 245, 245), (229, 229, 229))
    d.rounded_rectangle((x0, y0, x0 + w, y0 + h), radius=5 * SCALE, fill=bg, outline=border, width=SCALE)
    d.text((x0 + pad - l, y0 + (h - (b - t)) / 2 - t), label, font=font, fill=fg)


def parse(txt: Path):
    """The frame's clock, what caused it, and every node of its layout tree: (path, name, layer, (x, y, w, h))."""
    lines = txt.read_text().splitlines()
    clock = float(m.group(1)) if lines and (m := CLOCK.search(lines[0])) else 0.0
    cause = lines[0].split(": ", 1)[1] if lines and ": " in lines[0] else ""
    nodes = []
    try:
        start = lines.index("layout tree") + 1
    except ValueError:
        return clock, cause, nodes
    for line in lines[start:]:
        parts = line.split()
        sizes = SIZE.findall(line)
        if not parts or not sizes:
            continue
        w, h, x, y = (float(v) for v in sizes[-1])
        name = parts[1][:-1] if len(parts) > 1 and parts[1].endswith(":") else None
        nodes.append((parts[0], name, " layer " in line, (x, y, w, h)))
    return clock, cause, nodes


def union(a, b):
    if a is None:
        return b
    x0, y0 = min(a[0], b[0]), min(a[1], b[1])
    x1, y1 = max(a[0] + a[2], b[0] + b[2]), max(a[1] + a[3], b[1] + b[3])
    return (x0, y0, x1 - x0, y1 - y0)


def demo_frames(page_dir: Path):
    """Per demo: [(png, clock, demo box, box of the demo and the layers under it)] for frames showing it whole."""
    out = {}
    for txt in sorted(page_dir.glob("[0-9][0-9]-*.txt")):
        png = txt.with_suffix(".png")
        if not png.exists():
            continue
        clock, cause, nodes = parse(txt)
        demos = [(path, name, box) for path, name, _, box in nodes if name and name.endswith(".vue")]
        layers = {
            name: [b for p, _, layer, b in nodes if layer and p.startswith(path + "/") and b[2] > 0 and b[3] > 0]
            for path, name, _ in demos
        }
        for path, name, box in demos:
            x, y, w, h = box
            if w <= 0 or h <= 0 or x < 0 or y < 0 or x + w > VIEW[0] or y + h > VIEW[1]:
                continue
            # Another demo's dialog or menu over this one is that demo's frame, not this one's.
            if any(layers[other] for _, other, _ in demos if other != name):
                continue
            ext = box
            for b in layers[name]:
                ext = union(ext, b)
            out.setdefault(name, []).append((png, clock, cause, box, ext))
    return out


def clamp(box):
    x, y, w, h = box
    x0, y0 = max(0.0, x), max(0.0, y)
    x1, y1 = min(VIEW[0], x + w), min(VIEW[1], y + h)
    return (x0, y0, x1 - x0, y1 - y0)


def render(frames):
    """The demo's distinct frames on one canvas, each placed as the demo sits in it; with their durations."""
    # The canvas holds the demo and everything it opened, relative to the demo's own corner in each frame.
    rel = None
    for _, _, _, (dx, dy, _, _), ext in frames:
        e = clamp(ext)
        rel = union(rel, (e[0] - dx, e[1] - dy, e[2], e[3]))
    rx, ry, rw, rh = rel
    size = (round(rw * SCALE), round(rh * SCALE))
    kept, steps = [], []
    for png, clock, cause, (dx, dy, _, _), _ in frames:
        img = Image.open(png).convert("RGB")
        background = img.getpixel((2, 2))
        canvas = Image.new("RGB", size, background)
        want = (dx + rx, dy + ry, rw, rh)
        have = clamp(want)
        crop = img.crop(tuple(round(v * SCALE) for v in (have[0], have[1], have[0] + have[2], have[1] + have[3])))
        canvas.paste(crop, (round((have[0] - want[0]) * SCALE), round((have[1] - want[1]) * SCALE)))
        if kept and ImageChops.difference(canvas, kept[-1]).getbbox() is None:
            continue
        dark = sum(background) < 384
        for cx, cy in CLICK.findall(cause):
            pointer(canvas, (float(cx) - want[0]) * SCALE, (float(cy) - want[1]) * SCALE, dark)
        if key := KEY.search(cause):
            keycap(canvas, KEYS.get(key.group(1), key.group(1)), dark)
        elif "tab -> focus" in cause:
            keycap(canvas, "Tab", dark)
        kept.append(canvas)
        steps.append((clock, cause))
    durations = []
    for i, (c, _) in enumerate(steps):
        if i + 1 == len(steps):
            durations.append(LAST_MS)
            continue
        nc, ncause = steps[i + 1]
        d = nc - c
        if 0 < d <= MOTION_MAX_S:
            durations.append(max(40, round(d * 1000)))
        elif ncause.startswith("key Character"):
            durations.append(TYPE_MS)
        else:
            durations.append(STATE_MS)
    return kept[:MAX_FRAMES], durations[:MAX_FRAMES]


def main():
    shots, out, mode = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    manifest = {}
    for page_dir in sorted(p for p in shots.iterdir() if p.is_dir()):
        for demo, frames in demo_frames(page_dir).items():
            images, durations = render(frames)
            if len(images) < 2:
                continue
            stem = demo.removesuffix(".vue")
            target = out / page_dir.name / f"{stem}.webp"
            target.parent.mkdir(parents=True, exist_ok=True)
            images[0].save(
                target, save_all=True, append_images=images[1:], duration=durations, loop=0, quality=80, method=4
            )
            w, h = images[0].size
            manifest.setdefault(page_dir.name, {})[demo] = {
                "width": round(w / SCALE),
                "height": round(h / SCALE),
                "frames": len(images),
            }
            print(f"{mode} {page_dir.name} {demo}: {len(images)} frames, {target.stat().st_size // 1024} KB")
    (out / "motion.json").write_text(json.dumps(manifest, indent=1))


if __name__ == "__main__":
    main()
