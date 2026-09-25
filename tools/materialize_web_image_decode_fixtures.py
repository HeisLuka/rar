#!/usr/bin/env python3
import argparse
import base64
import hashlib
import json
from pathlib import Path

FIXTURES = {
    "orientation6.jpg": {
        "mime_type": "image/jpeg",
        "sha256": "160eea8797ca96dde96daa896e243d972869edf3b9f5459a54c618ce10c6df38",
        "base64": "/9j/4AAQSkZJRgABAQAAAQABAAD/4QAiRXhpZgAATU0AKgAAAAgAAQESAAMAAAABAAYAAAAAAAD/2wBDAAIBAQEBAQIBAQECAgICAgQDAgICAgUEBAMEBgUGBgYFBgYGBwkIBgcJBwYGCAsICQoKCgoKBggLDAsKDAkKCgr/2wBDAQICAgICAgUDAwUKBwYHCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgr/wAARCAACAAMDAREAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD6An/ZR/Zb8UeI/EepeJv2a/AGo3MXjDWrSO4vvB1jM6W9vqVzBBCGeIkJHDHHEi9ESNVUAKAP6F8DuJeIsl8EuFsPl+Mq0acstwFRxp1Jwi6lbC0q1ao1FpOdWrOdWpL4p1JynJuUm3/nB9Nri7izh76S2cYXKswr4elKlgKzhSq1KcXVxGXYSvXquMJJOpXr1KlatNrmqVak6k3KcpN//9k=",
    },
    "alpha.png": {
        "mime_type": "image/png",
        "sha256": "35a0f0f110046b955a8bdcd4c1a4fd2972ac20fb4081ae284bf008166784635f",
        "base64": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mM4kWLUAAAFNQHfrDzxywAAAABJRU5ErkJggg==",
    },
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    manifest = {}
    for name, spec in FIXTURES.items():
        payload = base64.b64decode(spec["base64"], validate=True)
        digest = hashlib.sha256(payload).hexdigest()
        if digest != spec["sha256"]:
            raise AssertionError((name, digest, spec["sha256"]))
        path = args.output / name
        path.write_bytes(payload)
        manifest[name] = {
            "mime_type": spec["mime_type"],
            "sha256": digest,
            "byte_len": len(payload),
            "path": str(path),
        }

    (args.output / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(manifest, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
