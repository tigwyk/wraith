# Rebuild assets/icon/wraith.ico, wraith-badge.ico and wraith-256.png from assets/icon/wraith.svg.
# Each frame is rendered at its own size with headless Edge; the SVG sits on a 16px grid, so
# multiples of 16 come out pixel-sharp. The badge (taskbar overlay for "needs attention") is the
# same ghost on Windows orange.
#   python scripts/make-icon.py
import pathlib, subprocess, tempfile
from PIL import Image

root = pathlib.Path(__file__).resolve().parent.parent
icon = root / "assets" / "icon"
edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
svg = (icon / "wraith.svg").read_text()
blue, orange = "#0078D4", "#F7630C"
assert blue in svg


def render(svg_text, px):
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "icon.svg").write_text(svg_text)
        page = tmp / "icon.html"
        page.write_text(f'<body style="margin:0"><img src="icon.svg" width="{px}" height="{px}">')
        shot = tmp / "icon.png"
        # Edge won't go below a minimum window size, so render into at least 256px and crop.
        win = max(px, 256)
        subprocess.run([edge, "--headless", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                        "--default-background-color=00000000", f"--window-size={win},{win}",
                        f"--screenshot={shot}", page.as_uri()], check=True, capture_output=True)
        img = Image.open(shot).convert("RGBA")
    assert img.getpixel((win - 1, win - 1))[3] == 0, "background should render transparent"
    return img.crop((0, 0, px, px))


def ico(svg_text, sizes, out):
    frames = [render(svg_text, s) for s in sizes]
    frames[-1].save(out, sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    return frames[-1]


ico(svg, [16, 20, 24, 32, 40, 48, 64, 256], icon / "wraith.ico").save(icon / "wraith-256.png")
ico(svg.replace(blue, orange), [16, 20, 24, 32], icon / "wraith-badge.ico")
print(icon / "wraith.ico")
