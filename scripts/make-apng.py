"""Builds the README's animated tours (APNG) from the demo screenshots.

APNG shows in a browser and on GitHub as a looping animation. No dependency
beyond the Python standard library: it reads the 8-bit RGBA PNGs that
`capture-screenshots.ps1` writes, centres each on one canvas, cross-fades from
one to the next and writes the frames as an animated PNG.

    python scripts/make-apng.py tour   assets/screenshots/tour.png
    python scripts/make-apng.py notices assets/screenshots/notices.png

The frames are the shots in assets/screenshots; run the capture script first
when the pages changed.
"""

import struct
import sys
import zlib
from pathlib import Path

SHOTS = Path(__file__).resolve().parent.parent / "assets" / "screenshots"

# name -> shots in order; canvas size; hold and fade in milliseconds.
TOURS = {
    "tour": {
        "files": [
            "calendar", "pomodoro", "reminders", "notes", "music-lyrics", "mixer",
            "clipboard", "shelf", "system", "cli", "shortcuts", "search",
        ],
        "canvas": (560, 298),
        "hold": 1500,
        "fade": 90,
        "steps": 4,
    },
    "notices": {
        "files": [
            "notch", "greeting", "clipboard-copied", "screenshot", "system-warning",
            "lyric-long",
        ],
        "canvas": (584, 160),
        "hold": 1400,
        "fade": 90,
        "steps": 4,
    },
}


def read_png(path):
    """(width, height, RGBA bytes) of an 8-bit RGBA, non-interlaced PNG."""
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    pos, idat, w, h = 8, b"", 0, 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            assert (depth, ctype, interlace) == (8, 6, 0), f"{path}: need 8-bit RGBA"
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = w * 4
    out = bytearray(h * stride)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        ft = raw[p]
        line = bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        if ft == 1:
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 255
        elif ft == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif ft == 3:
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 255
        elif ft == 4:
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 255
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, bytes(out)


def centred(shot, canvas):
    """The shot on a transparent canvas, centred."""
    cw, ch = canvas
    w, h, px = shot
    out = bytearray(cw * ch * 4)
    ox, oy = (cw - w) // 2, (ch - h) // 2
    for y in range(h):
        if not 0 <= y + oy < ch:
            continue
        src = px[y * w * 4:(y + 1) * w * 4]
        dst = ((y + oy) * cw + ox) * 4
        out[dst:dst + w * 4] = src
    return bytes(out)


def blend(a, b, t):
    """`a` to `b` at `t` (0..1), in premultiplied alpha so edges stay clean."""
    out = bytearray(len(a))
    for i in range(0, len(a), 4):
        aa, ba = a[i + 3], b[i + 3]
        alpha = aa + (ba - aa) * t
        if alpha < 0.5:
            continue
        for k in range(3):
            pm = a[i + k] * aa + (b[i + k] * ba - a[i + k] * aa) * t
            out[i + k] = min(255, int(pm / alpha + 0.5))
        out[i + 3] = int(alpha + 0.5)
    return bytes(out)


def chunk(kind, body):
    crc = zlib.crc32(kind + body) & 0xFFFFFFFF
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)


def write_apng(path, canvas, frames):
    """`frames`: [(RGBA bytes, delay in ms)], looping forever."""
    cw, ch = canvas
    out = [b"\x89PNG\r\n\x1a\n", chunk(b"IHDR", struct.pack(">IIBBBBB", cw, ch, 8, 6, 0, 0, 0))]
    out.append(chunk(b"acTL", struct.pack(">II", len(frames), 0)))
    seq = 0
    for n, (px, delay) in enumerate(frames):
        fctl = struct.pack(">IIIIIHHBB", seq, cw, ch, 0, 0, delay, 1000, 1, 0)
        out.append(chunk(b"fcTL", fctl))
        seq += 1
        rows = b"".join(b"\x00" + px[y * cw * 4:(y + 1) * cw * 4] for y in range(ch))
        data = zlib.compress(rows, 9)
        if n == 0:
            out.append(chunk(b"IDAT", data))
        else:
            out.append(chunk(b"fdAT", struct.pack(">I", seq) + data))
            seq += 1
    out.append(chunk(b"IEND", b""))
    Path(path).write_bytes(b"".join(out))


def main():
    name, target = sys.argv[1], sys.argv[2]
    tour = TOURS[name]
    canvas = tour["canvas"]
    images = [centred(read_png(SHOTS / f"{f}.png"), canvas) for f in tour["files"]]
    frames = []
    for i, img in enumerate(images):
        frames.append((img, tour["hold"]))
        nxt = images[(i + 1) % len(images)]
        for s in range(1, tour["steps"] + 1):
            frames.append((blend(img, nxt, s / (tour["steps"] + 1)), tour["fade"]))
    write_apng(target, canvas, frames)
    print(f"{target}: {len(frames)} frames, {Path(target).stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
