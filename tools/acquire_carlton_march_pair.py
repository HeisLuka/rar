#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import re
import sys
import time
from pathlib import Path

import requests

PAGE = "https://www.carltonji.org.uk/parents/latest-letters"
TENANT_ID = "1fd359a4-cb4e-412d-81a1-79f36166cafc"

SPECS = {
    "March 2026.pub": {
        "artifact_name": "March-2026.pub",
        "file_id": "5a939344-76a6-4a6a-b30b-7237edac8bfa",
        "size": 9_580_032,
        "sha256": "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3",
    },
    "March 2026.pdf": {
        "artifact_name": "March-2026-reference.pdf",
        "file_id": "21dbc16e-9242-4ff1-8b3d-f7c5d0f0a50e",
        "size": 1_108_627,
        "sha256": "c1288571ee8da91afa27ca5c49191aacefe82123df294c11f58ca047d0ced769",
    },
}


def hydration_file(raw: str, public_name: str) -> tuple[str, int, str | None]:
    needle = '"name":"' + public_name + '"'
    index = raw.find(needle)
    if index < 0:
        raise RuntimeError(f"{public_name}: absent from School Jotter hydration state")
    window = raw[max(0, index - 1200) : min(len(raw), index + 18000)]

    file_match = re.search(
        r'"file":\{"id":"([^"]+)","name":"' + re.escape(public_name) + r'"',
        window,
    )
    if not file_match:
        raise RuntimeError(f"{public_name}: file id missing")
    file_id = file_match.group(1)

    size_match = re.search(
        r'"name":"' + re.escape(public_name) + r'".*?"size":(\d+)',
        window,
        flags=re.S,
    )
    if not size_match:
        raise RuntimeError(f"{public_name}: declared size missing")
    declared_size = int(size_match.group(1))

    origin_match = re.search(
        r'"url":\{"origin":"((?:\\.|[^"\\])*)"',
        window,
        flags=re.S,
    )
    origin = (
        json.loads('"' + origin_match.group(1) + '"')
        if origin_match
        else None
    )
    return file_id, declared_size, origin


def candidates(public_name: str, file_id: str, hydrated_origin: str | None) -> list[str]:
    suffix = Path(public_name).suffix
    values = [
        "https://sj3-bucket-media.s3.eu-west-1.amazonaws.com/"
        f"{file_id[:2]}/{file_id}",
        f"https://docs-cdn.schooljotter3.com/{TENANT_ID}/{file_id}/{file_id}{suffix}",
        f"https://docs-cdn.schooljotter3.com/{TENANT_ID}/{file_id}/"
        + requests.utils.quote(public_name),
    ]
    if hydrated_origin:
        values.append(hydrated_origin)
    return values


def acquire_one(
    session: requests.Session,
    raw: str,
    public_name: str,
    spec: dict,
    output_dir: Path,
) -> dict:
    file_id, declared_size, hydrated_origin = hydration_file(raw, public_name)
    if file_id != spec["file_id"]:
        raise RuntimeError(
            f"{public_name}: file id drift {file_id} != {spec['file_id']}"
        )
    if declared_size != spec["size"]:
        raise RuntimeError(
            f"{public_name}: declared size drift {declared_size} != {spec['size']}"
        )

    target = output_dir / spec["artifact_name"]
    for candidate in candidates(public_name, file_id, hydrated_origin):
        digest = hashlib.sha256()
        size = 0
        try:
            with session.get(
                candidate,
                timeout=(10, 90),
                stream=True,
                allow_redirects=True,
            ) as response:
                if response.status_code != 200:
                    continue
                with target.open("wb") as out:
                    for chunk in response.iter_content(1024 * 1024):
                        if not chunk:
                            continue
                        size += len(chunk)
                        if size > spec["size"]:
                            raise RuntimeError(
                                f"{public_name}: download exceeded exact expected size"
                            )
                        digest.update(chunk)
                        out.write(chunk)
        except requests.RequestException:
            target.unlink(missing_ok=True)
            continue

        if size != spec["size"]:
            target.unlink(missing_ok=True)
            continue
        sha256 = digest.hexdigest()
        if sha256 != spec["sha256"]:
            target.unlink(missing_ok=True)
            raise RuntimeError(
                f"{public_name}: SHA drift {sha256} != {spec['sha256']}"
            )
        return {
            "public_name": public_name,
            "artifact_name": spec["artifact_name"],
            "file_id": file_id,
            "size": size,
            "sha256": sha256,
            "source_host": requests.utils.urlparse(candidate).hostname,
        }

    raise RuntimeError(f"{public_name}: all public download candidates failed")


def main(argv: list[str]) -> None:
    if len(argv) != 2:
        raise SystemExit("usage: acquire_carlton_march_pair.py OUTPUT_DIR")
    output_dir = Path(argv[1])
    output_dir.mkdir(parents=True, exist_ok=True)

    session = requests.Session()
    session.headers.update(
        {
            "User-Agent": "Mozilla/5.0 (compatible; Chaptera-Carlton-Visual-Oracle/2.0)",
            "Accept": "*/*",
            "Cache-Control": "no-cache",
            "Pragma": "no-cache",
        }
    )
    response = session.get(
        PAGE,
        params={"chaptera_visual_oracle": str(time.time_ns())},
        timeout=30,
    )
    response.raise_for_status()

    records = {}
    for public_name, spec in SPECS.items():
        record = acquire_one(session, response.text, public_name, spec, output_dir)
        records[public_name] = record
        print(
            f"ACQUIRED {public_name}: size={record['size']} "
            f"sha256={record['sha256']} host={record['source_host']}"
        )

    manifest = {
        "schema": "chaptera.carlton-march-visual-inputs.v1",
        "source_page": PAGE,
        "files": records,
    }
    (output_dir / "acquisition.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main(sys.argv)
