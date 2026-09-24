#!/usr/bin/env python3
"""PUB-T-603 bounded acquisition/static-boundary probe.

Public archive indexes only. Never accepts a substitute build: the target DLL
must match exact size + SHA-256. Raw binaries are never written into the repo;
the workflow uploads only JSON/text receipts.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import urllib.parse
import urllib.request

TARGET_DLL = {
    "name": "pubfilt.dll",
    "version": "11.0.5704.0",
    "size": 27200,
    "sha256": "0238a45be62da10cf98ff9b7aeac11ad55b9a1f6a69d2d2356c8fcab538105bc",
}
TARGET_CONTAINER = {
    "name": "en_office_2003_sps.iso",
    "sha1": "f5689402ede43433d9e1bde2c5f99550d6b1cc88",
}
IA_SEARCHES = [
    '"en_office_2003_sps.iso"',
    '"f5689402ede43433d9e1bde2c5f99550d6b1cc88"',
    '"Office SharePoint Portal Server 2003" AND mediatype:software',
]
CONTEXT_ITEMS = ["X10-92680"]
USER_AGENT = "HeisLuka-rar-PUB-T-603/1.0 (+public research receipt)"


def request_json(url: str):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=45) as response:
        return json.load(response)


def hash_file(path: pathlib.Path, algorithm: str) -> str:
    h = hashlib.new(algorithm)
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def ia_search(query: str):
    params = urllib.parse.urlencode(
        {
            "q": query,
            "fl[]": ["identifier", "title"],
            "rows": 100,
            "page": 1,
            "output": "json",
        },
        doseq=True,
    )
    return request_json("https://archive.org/advancedsearch.php?" + params)


def metadata(identifier: str):
    return request_json(
        "https://archive.org/metadata/" + urllib.parse.quote(identifier, safe="")
    )


def candidate_url(identifier: str, name: str) -> str:
    return (
        "https://archive.org/download/"
        + urllib.parse.quote(identifier, safe="")
        + "/"
        + urllib.parse.quote(name)
    )


def download(url: str, path: pathlib.Path, max_bytes: int = 1_200_000_000):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=90) as response, path.open("wb") as out:
        total = 0
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk:
                break
            total += len(chunk)
            if total > max_bytes:
                raise RuntimeError(f"download exceeded bounded limit {max_bytes}")
            out.write(chunk)
    return path.stat().st_size


def sevenzip_list(path: pathlib.Path) -> str:
    proc = subprocess.run(
        ["7z", "l", "-slt", str(path)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=180,
        check=False,
    )
    return proc.stdout


def sevenzip_extract(path: pathlib.Path, out_dir: pathlib.Path):
    out_dir.mkdir(parents=True, exist_ok=True)
    proc = subprocess.run(
        ["7z", "x", "-y", f"-o{out_dir}", str(path)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=900,
        check=False,
    )
    return proc.returncode, proc.stdout[-12000:]


def inspect_pe(path: pathlib.Path):
    result = {
        "size": path.stat().st_size,
        "sha1": hash_file(path, "sha1"),
        "sha256": hash_file(path, "sha256"),
    }
    try:
        import pefile
    except Exception as exc:
        result["pe_error"] = f"pefile unavailable: {exc}"
        return result

    try:
        pe = pefile.PE(str(path), fast_load=False)
        imports = []
        for entry in getattr(pe, "DIRECTORY_ENTRY_IMPORT", []) or []:
            imports.append(entry.dll.decode("ascii", errors="replace"))
        delay_imports = []
        for entry in getattr(pe, "DIRECTORY_ENTRY_DELAY_IMPORT", []) or []:
            delay_imports.append(entry.dll.decode("ascii", errors="replace"))
        exports = []
        exp = getattr(pe, "DIRECTORY_ENTRY_EXPORT", None)
        if exp:
            for sym in exp.symbols:
                if sym.name:
                    exports.append(sym.name.decode("ascii", errors="replace"))

        version_strings = {}
        for fi in getattr(pe, "FileInfo", []) or []:
            if not isinstance(fi, list):
                continue
            for block in fi:
                if getattr(block, "Key", b"") == b"StringFileInfo":
                    for table in block.StringTable:
                        for key, value in table.entries.items():
                            version_strings[
                                key.decode("utf-8", errors="replace")
                            ] = value.decode("utf-8", errors="replace")

        security_dir = pe.OPTIONAL_HEADER.DATA_DIRECTORY[
            pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_SECURITY"]
        ]
        result.update(
            {
                "machine": hex(pe.FILE_HEADER.Machine),
                "timestamp": pe.FILE_HEADER.TimeDateStamp,
                "imports": sorted(set(imports), key=str.lower),
                "delay_imports": sorted(set(delay_imports), key=str.lower),
                "exports": sorted(set(exports)),
                "version_strings": version_strings,
                "authenticode_directory": {
                    "virtual_address": security_dir.VirtualAddress,
                    "size": security_dir.Size,
                    "present": bool(security_dir.VirtualAddress and security_dir.Size),
                },
            }
        )
    except Exception as exc:
        result["pe_error"] = repr(exc)
    return result


def classify_static(pe_info: dict):
    modules = [x.lower() for x in pe_info.get("imports", []) + pe_info.get("delay_imports", [])]
    publisherish = [
        x for x in modules
        if any(token in x for token in ("mspub", "publisher", "quill", "pubconv"))
    ]
    oleish = [
        x for x in modules
        if any(token in x for token in ("ole32", "oleaut32", "storage", "propsys"))
    ]
    if publisherish:
        return {
            "class": "thin/shared-component wrapper candidate",
            "basis": "Publisher/Quill-specific imported module(s) observed",
            "publisher_specific_modules": publisherish,
        }
    if pe_info.get("imports") and oleish:
        return {
            "class": "standalone/minimal parser candidate",
            "basis": "structured-storage/common Win32 imports observed without Publisher-specific imports",
            "ole_related_modules": oleish,
            "limitation": "static imports alone do not prove parser completeness or persisted semantics",
        }
    return {
        "class": "unresolved",
        "basis": "static import surface is insufficient for bounded classification",
    }


def recursive_pubfilt_candidates(root: pathlib.Path, receipt: dict):
    found = []
    for path in root.rglob("*"):
        if path.is_file() and path.name.lower() == "pubfilt.dll":
            found.append(path)

    archive_suffixes = {".cab", ".msi", ".msp", ".exe", ".zip"}
    archives = [
        p for p in root.rglob("*")
        if p.is_file() and p.suffix.lower() in archive_suffixes and p.stat().st_size < 250_000_000
    ]
    receipt["archive_members_scanned"] = len(archives)
    nested_root = root.parent / "nested"
    for index, archive in enumerate(archives):
        listing = sevenzip_list(archive)
        if "pubfilt.dll" not in listing.lower():
            continue
        out = nested_root / f"{index:04d}"
        code, tail = sevenzip_extract(archive, out)
        receipt.setdefault("nested_archives_with_pubfilt", []).append(
            {"path": str(archive.relative_to(root)), "returncode": code, "log_tail": tail}
        )
        for candidate in out.rglob("*"):
            if candidate.is_file() and candidate.name.lower() == "pubfilt.dll":
                found.append(candidate)
    return found


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--work", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    args.work.mkdir(parents=True, exist_ok=True)

    receipt = {
        "receipt_version": "pub-t-603.acquisition.v1",
        "target_dll": TARGET_DLL,
        "target_container": TARGET_CONTAINER,
        "authority_rule": "Only exact DLL size+SHA-256 satisfies target; later builds are context only.",
        "internet_archive_queries": [],
        "candidate_items": [],
        "container_candidates": [],
        "dll_candidates": [],
        "exact_target_found": False,
        "classification": None,
        "limitations": [],
    }

    identifiers = set(CONTEXT_ITEMS)
    for query in IA_SEARCHES:
        record = {"query": query}
        try:
            data = ia_search(query)
            docs = data.get("response", {}).get("docs", [])
            record["result_count"] = data.get("response", {}).get("numFound", 0)
            record["identifiers"] = [d.get("identifier") for d in docs if d.get("identifier")]
            identifiers.update(record["identifiers"])
        except Exception as exc:
            record["error"] = repr(exc)
        receipt["internet_archive_queries"].append(record)

    exact_container = None
    for identifier in sorted(identifiers):
        item = {"identifier": identifier}
        try:
            meta = metadata(identifier)
            files = meta.get("files", [])
            item["title"] = meta.get("metadata", {}).get("title")
            item["file_count"] = len(files)
            matching = []
            for f in files:
                name = f.get("name") or ""
                sha1 = (f.get("sha1") or "").lower()
                if name.lower() == TARGET_CONTAINER["name"].lower() or sha1 == TARGET_CONTAINER["sha1"]:
                    matching.append(
                        {
                            "name": name,
                            "size": f.get("size"),
                            "sha1": sha1 or None,
                            "md5": f.get("md5"),
                            "url": candidate_url(identifier, name),
                        }
                    )
            item["matching_container_files"] = matching
            for candidate in matching:
                candidate["identifier"] = identifier
                receipt["container_candidates"].append(candidate)
                if candidate.get("sha1") == TARGET_CONTAINER["sha1"]:
                    exact_container = candidate
        except Exception as exc:
            item["error"] = repr(exc)
        receipt["candidate_items"].append(item)

    if exact_container is None:
        receipt["limitations"].append(
            "No Internet Archive metadata entry in the bounded query set exposed the exact English MSDN container SHA-1."
        )
    else:
        container_path = args.work / TARGET_CONTAINER["name"]
        exact_container["download_attempted"] = True
        try:
            exact_container["downloaded_bytes"] = download(exact_container["url"], container_path)
            exact_container["download_sha1"] = hash_file(container_path, "sha1")
            exact_container["container_identity_valid"] = (
                exact_container["download_sha1"] == TARGET_CONTAINER["sha1"]
            )
            if not exact_container["container_identity_valid"]:
                receipt["limitations"].append("Downloaded container failed exact SHA-1 identity check; extraction stopped.")
            else:
                extract_root = args.work / "iso"
                code, tail = sevenzip_extract(container_path, extract_root)
                receipt["container_extract"] = {"returncode": code, "log_tail": tail}
                if code != 0:
                    receipt["limitations"].append("Exact container was found but 7z extraction failed.")
                else:
                    candidates = recursive_pubfilt_candidates(extract_root, receipt)
                    unique = {}
                    for path in candidates:
                        info = inspect_pe(path)
                        key = info["sha256"]
                        if key in unique:
                            continue
                        info["relative_path"] = str(path.relative_to(args.work))
                        info["exact_target"] = (
                            info["size"] == TARGET_DLL["size"]
                            and info["sha256"] == TARGET_DLL["sha256"]
                        )
                        unique[key] = info
                    receipt["dll_candidates"] = list(unique.values())
                    exact = [x for x in receipt["dll_candidates"] if x["exact_target"]]
                    if exact:
                        receipt["exact_target_found"] = True
                        receipt["classification"] = classify_static(exact[0])
                    else:
                        receipt["limitations"].append(
                            "Exact container was extracted but no pubfilt.dll candidate matched target size+SHA-256."
                        )
        except Exception as exc:
            exact_container["download_error"] = repr(exc)
            receipt["limitations"].append("Exact container metadata was found but bounded download/extraction did not complete.")

    if not receipt["exact_target_found"]:
        receipt["result"] = "bounded acquisition negative"
        receipt["claim"] = (
            "The exact v11 target was not acquired from the tested public archive surfaces; "
            "no substitute build was accepted."
        )
    else:
        receipt["result"] = "exact target acquired and statically inspected"
        receipt["claim"] = (
            "Exact v11 identity acquired; classification is static/provenance only and does not establish PUB semantics."
        )

    (args.out / "acquisition-manifest.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    summary = {
        "exact_target_found": receipt["exact_target_found"],
        "result": receipt["result"],
        "container_candidate_count": len(receipt["container_candidates"]),
        "dll_candidate_count": len(receipt["dll_candidates"]),
        "classification": receipt["classification"],
        "limitations": receipt["limitations"],
    }
    (args.out / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
