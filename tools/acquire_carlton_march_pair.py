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
    base_href = base_match.group(1) if base_match else "/"
    document_base = requests.compat.urljoin(response.url, base_href)

    asset_refs = set(script_srcs)
    asset_refs.update(
        re.findall(
            r'<link[^>]+rel="modulepreload"[^>]+href="([^"]+[.]js)"',
            raw,
            flags=re.I,
        )
    )

    main_src = next(
        (src for src in script_srcs if re.search(r"(^|/)main-[^/]+[.]js$", src)),
        None,
    )
    if main_src:
        bundle = session.get(
            requests.compat.urljoin(document_base, main_src),
            timeout=30,
        )
        bundle.raise_for_status()
        asset_refs.update(re.findall(r'from"[.]/([^"]+[.]js)"', bundle.text))
        asset_refs.update(
            re.findall(r'import\("[.]/([^"]+[.]js)"\)', bundle.text)
        )

    api_base_candidates: set[str] = set()
    total_js_bytes = 0
    for asset_ref in sorted(asset_refs):
        asset_name = asset_ref.rsplit("/", 1)[-1]
        if not re.fullmatch(r"[A-Za-z0-9._-]+[.]js", asset_name):
            continue
        asset_response = session.get(
            requests.compat.urljoin(document_base, asset_ref),
            timeout=30,
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
            "public API base is not uniquely grounded: "
            + repr(sorted(api_base_candidates))
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
        raise RuntimeError(
            f"{public_name}: absent from School Jotter hydration state"
        )
    window = raw[max(0, index - 600) : min(len(raw), index + 14_000)]

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
    hydration_origin = (
        json.loads('"' + origin_match.group(1) + '"')
        if origin_match
        else None
    )
    return resource_id, file_id, declared_size, hydration_origin


def find_exact_file(node, file_id: str, public_name: str):
    if isinstance(node, dict):
        if (
            node.get("id") == file_id
            and node.get("name") == public_name
            and isinstance(node.get("url"), dict)
            and isinstance(node["url"].get("origin"), str)
        ):
            return node
        for value in node.values():
            found = find_exact_file(value, file_id, public_name)
            if found is not None:
                return found
    elif isinstance(node, list):
        for value in node:
            found = find_exact_file(value, file_id, public_name)
            if found is not None:
                return found
    return None


def static_candidates(
    public_name: str, file_id: str, tenant_id: str, hydration_origin: str | None
) -> list[str]:
    suffix = Path(public_name).suffix
    values = [
        "https://sj3-bucket-media.s3.eu-west-1.amazonaws.com/"
        f"{file_id[:2]}/{file_id}",
        f"https://docs-cdn.schooljotter3.com/{tenant_id}/{file_id}/{file_id}{suffix}",
        f"https://docs-cdn.schooljotter3.com/{tenant_id}/{file_id}/"
        + requests.utils.quote(public_name),
    ]
    if hydration_origin:
        values.append(hydration_origin)
    return values


def download_exact(
    session: requests.Session,
    public_name: str,
    spec: dict,
    target: Path,
    candidates: list[str],
) -> tuple[int, str, str]:
    for candidate in candidates:
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
                                f"{public_name}: download exceeded expected size"
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
        return size, sha256, requests.utils.urlparse(candidate).hostname or ""

    raise RuntimeError(f"{public_name}: all public download candidates failed")


def acquire_one(
    session: requests.Session,
    raw: str,
    public_name: str,
    spec: dict,
    output_dir: Path,
    api_base: str,
    tenant_id: str,
) -> dict:
    resource_id, file_id, declared_size, hydration_origin = hydration_file(
        raw, public_name
    )
    if file_id != spec["file_id"]:
        raise RuntimeError(
            f"{public_name}: file id drift {file_id} != {spec['file_id']}"
        )
    if declared_size != spec["size"]:
        raise RuntimeError(
            f"{public_name}: declared size drift "
            f"{declared_size} != {spec['size']}"
        )

    resource_url = f"{api_base}/resources/{resource_id}"
    api_response = session.get(
        resource_url,
        timeout=30,
        headers={"Accept": "application/json", "X-Tenant": tenant_id},
    )

    refreshed_origin = None
    if api_response.ok:
        try:
            api_payload = api_response.json()
        except ValueError:
            api_payload = None

        refreshed_file = find_exact_file(api_payload, file_id, public_name)
        if refreshed_file is not None:
            refreshed_size = refreshed_file.get("size")
            if refreshed_size != spec["size"]:
                raise RuntimeError(
                    f"{public_name}: refreshed size drift "
                    f"{refreshed_size} != {spec['size']}"
                )
            refreshed_origin = refreshed_file["url"]["origin"]

    candidates: list[str] = []
    if refreshed_origin:
        candidates.append(refreshed_origin)
    candidates.extend(
        static_candidates(
            public_name,
            file_id,
            tenant_id,
            hydration_origin,
        )
    )

    target = output_dir / spec["artifact_name"]
    size, sha256, source_host = download_exact(
        session,
        public_name,
        spec,
        target,
        candidates,
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
        "source_host": source_host,
    }


def main(argv: list[str]) -> None:
    if len(argv) != 2:
        raise SystemExit("usage: acquire_carlton_march_pair.py OUTPUT_DIR")

    output_dir = Path(argv[1])
    output_dir.mkdir(parents=True, exist_ok=True)

    session = requests.Session()
    session.headers.update(
        {
            "User-Agent": (
                "Mozilla/5.0 "
                "(compatible; Chaptera-Carlton-Visual-Oracle/2.0)"
            ),
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

    api_base, tenant_id = discover_api_context(
        session, response, response.text
    )

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
            f"ACQUIRED {public_name}: "
            f"resource={record['resource_id']} "
            f"id={record['file_id']} "
            f"api_status={record['resource_api_status']} "
            f"refreshed={record['resource_api_refreshed_origin']} "
            f"size={record['size']} sha256={record['sha256']} "
            f"host={record['source_host']}"
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
