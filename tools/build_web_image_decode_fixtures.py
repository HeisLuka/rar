from pathlib import Path
from PIL import Image

root = Path("target/web-image-decode")
root.mkdir(parents=True, exist_ok=True)

alpha = Image.new("RGBA", (3, 2))
alpha.putdata([
    (200, 40, 120, 64),
    (20, 220, 80, 128),
    (255, 10, 20, 255),
    (17, 93, 211, 0),
    (80, 10, 220, 192),
    (5, 6, 7, 255),
])
alpha.save(root / "alpha.png", format="PNG", optimize=False)

orientation = Image.new("RGB", (3, 2))
orientation.putdata([
    (250, 10, 10),
    (10, 250, 10),
    (10, 10, 250),
    (240, 220, 10),
    (220, 10, 220),
    (10, 220, 220),
])
exif = Image.Exif()
exif[274] = 6
orientation.save(
    root / "orientation-6.jpg",
    format="JPEG",
    quality=100,
    subsampling=0,
    exif=exif,
)
print(root)
