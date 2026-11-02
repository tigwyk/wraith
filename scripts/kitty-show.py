"""Show a PNG inline with the Kitty graphics protocol (a minimal `kitten icat` for acceptance runs).

    python scripts/kitty-show.py photo.png      # show a file
    python scripts/kitty-show.py --gradient     # show a generated 320x160 test image
"""
import base64
import struct
import sys
import zlib


def gradient_png(w=320, h=160):
    rows = bytearray()
    for y in range(h):
        rows.append(0)  # filter: none
        for x in range(w):
            rows += bytes((x * 255 // w, y * 255 // h, 160, 255))

    def chunk(kind, data):
        c = struct.pack(">I", len(data)) + kind + data
        return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(rows))) + chunk(b"IEND", b"")


def main():
    arg = sys.argv[1] if len(sys.argv) > 1 else "--gradient"
    data = gradient_png() if arg == "--gradient" else open(arg, "rb").read()
    b64 = base64.standard_b64encode(data)
    out = sys.stdout.buffer
    chunks = [b64[i:i + 4096] for i in range(0, len(b64), 4096)] or [b""]
    for i, c in enumerate(chunks):
        more = 1 if i < len(chunks) - 1 else 0
        head = b"a=T,f=100,q=2," if i == 0 else b""
        out.write(b"\x1b_G" + head + b"m=%d;" % more + c + b"\x1b\\")
    out.write(b"\n")
    out.flush()


if __name__ == "__main__":
    main()
