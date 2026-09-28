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


def discover_api_context(
    session: requests.Session, response: requests.Response, raw: str
) -> tuple[str, str]:
    script_srcs = re.findall(r'<script[^>]+src="([^"]+)"', raw, flags=re.I)
    base_match = re.search(r'<base[^>]+href="([^"]+)"', raw, flags=re.I)
    document_base = requests.compat.urljoin(
        response.url, base_match.group(1) if base_match else "/"
    )

    asset_refs = set(script_srcs)
    asset_refs.update(
        re.findall(
            r'<link[^>]+rel="modulepreload"[^>]+href="([^"]+[.]js)"',
            raw,
            flags=re.I,
        )
    )

    main_src = next(
        (src for src in script_srcs if re.search(r'(^|/)main-[^/]+[.]js    needle = '"name":"' + public_name + '"'
    index = raw.find(needle)
    if index < 0:
        raise RuntimeError(f"{public_name}: absent from School Jotter hydration state")
    window = raw[max(0, index - 1200) : min(len(raw), index + 18000)]

    resource_match = re.search(
        r'\{"id":"([^"]+)","file":\{"id":"([^"]+)","name":"'
        + re.escape(public_name)
        + r'"',
        window,
    )
    if not resource_match:
        raise RuntimeError(f"{public_name}: resource/file ids missing")
    resource_id, file_id = resource_match.groups()

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
    return resource_id, file_id, declared_size, origin


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
    api_base: str,
    tenant_id: str,
) -> dict:
    resource_id, file_id, declared_size, hydrated_origin = hydration_file(
        raw, public_name
    )
    if file_id != spec["file_id"]:
        raise RuntimeError(
            f"{public_name}: file id drift {file_id} != {spec['file_id']}"
        )
    if declared_size != spec["size"]:
        raise RuntimeError(
            f"{public_name}: declared size drift {declared_size} != {spec['size']}"
        )

    refreshed_origin = None
    resource_url = f"{api_base}/resources/{resource_id}"
    api_response = session.get(
        resource_url,
        timeout=30,
        headers={"Accept": "application/json", "X-Tenant": tenant_id},
    )
    if api_response.ok:
        try:
            payload = api_response.json()
        except ValueError:
            payload = None

        def find_file(node):
            if isinstance(node, dict):
                if (
                    node.get("id") == file_id
                    and node.get("name") == public_name
                    and isinstance(node.get("url"), dict)
                    and isinstance(node["url"].get("origin"), str)
                ):
                    return node
                for value in node.values():
                    found = find_file(value)
                    if found is not None:
                        return found
            elif isinstance(node, list):
                for value in node:
                    found = find_file(value)
                    if found is not None:
                        return found
            return None

        refreshed = find_file(payload)
        if refreshed is not None:
            if refreshed.get("size") != spec["size"]:
                raise RuntimeError(
                    f"{public_name}: refreshed size drift "
                    f"{refreshed.get('size')} != {spec['size']}"
                )
            refreshed_origin = refreshed["url"]["origin"]

    target = output_dir / spec["artifact_name"]
    download_candidates = []
    if refreshed_origin:
        download_candidates.append(refreshed_origin)
    download_candidates.extend(candidates(public_name, file_id, hydrated_origin))

    for candidate in download_candidates:
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
            "resource_id": resource_id,
            "file_id": file_id,
            "resource_api_status": api_response.status_code,
            "resource_api_refreshed_origin": refreshed_origin is not None,
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
    api_base, tenant_id = discover_api_context(session, response, response.text)

    records = {}
    for public_name, spec in SPECS.items():
        record = acquire_one(
            session,
            response.text,
            public_name,
            spec,
            output_dir,
            api_base,
            tenant_id,
        )
        records[public_name] = record
        print(
            f"ACQUIRED {public_name}: size={record['size']} "
            f"sha256={record['sha256']} host={record['source_host']}"
        )

    manifest = {
        "schema": "chaptera.carlton-march-visual-inputs.v1",
        "source_page": PAGE,
        "api_base": api_base,
        "tenant_id": tenant_id,
        "files": records,
    }
    (output_dir / "acquisition.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main(sys.argv)
, src)),
        None,
    )
    if main_src:
        main_response = session.get(
            requests.compat.urljoin(document_base, main_src), timeout=30
        )
        main_response.raise_for_status()
        asset_refs.update(re.findall(r'from"[.]/([^"]+[.]js)"', main_response.text))
        asset_refs.update(
            re.findall(r'import\("[.]/([^"]+[.]js)"\)', main_response.text)
        )

    api_base_candidates = set()
    total_js_bytes = 0
    for asset_ref in sorted(asset_refs):
        asset_name = asset_ref.rsplit("/", 1)[-1]
        if not re.fullmatch(r"[A-Za-z0-9._-]+[.]js", asset_name):
            continue
        asset_response = session.get(
            requests.compat.urljoin(document_base, asset_ref), timeout=30
        )
        if not asset_response.ok:
            continue
        total_js_bytes += len(asset_response.content)
        if total_js_bytes > 10_000_000:
            raise RuntimeError("frontend API discovery exceeded 10 MB JS ceiling")
        api_base_candidates.update(
            re.findall(
                r'ApiConnectorConfig:\{DB_URL:"(https://[^"]+)"',
                asset_response.text,
            )
        )

    if len(api_base_candidates) != 1:
        raise RuntimeError(
            f"public API base is not uniquely grounded: {sorted(api_base_candidates)}"
        )
    api_base = next(iter(api_base_candidates)).rstrip("/")

    tenant_match = re.search(r'"TENANT_DOMAIN":\{"id":"([^"]+)"', raw)
    if not tenant_match:
        raise RuntimeError("TENANT_DOMAIN.id missing from public hydration state")
    return api_base, tenant_match.group(1)


def hydration_file(
    raw: str, public_name: str
) -> tuple[str, str, int, str | None]:
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
