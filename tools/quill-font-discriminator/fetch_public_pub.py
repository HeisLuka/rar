#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
from urllib.parse import urljoin

import requests
from bs4 import BeautifulSoup

OLE_MAGIC = bytes.fromhex("d0cf11e0a1b11ae1")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--landing", required=True)
    ap.add_argument("--title-needle", required=True)
    ap.add_argument("--output", type=pathlib.Path, required=True)
    ap.add_argument("--receipt", type=pathlib.Path, required=True)
    ap.add_argument("--direct-candidate")
    ap.add_argument("--expected-sha256")
    args = ap.parse_args()

    headers = {
        "User-Agent": (
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
            "(KHTML, like Gecko) Chrome/140.0 Safari/537.36"
        ),
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        "Accept-Language": "en-US,en;q=0.9",
    }
    session = requests.Session()
    landing = None
    landing_error = None
    link_text = None
    href = None
    response = None
    payload = None
    acquisition_route = None

    try:
        landing = session.get(args.landing, headers=headers, timeout=45)
        landing.raise_for_status()

        soup = BeautifulSoup(landing.text, "html.parser")
        candidates = []
        for anchor in soup.find_all("a", href=True):
            text = " ".join(anchor.get_text(" ", strip=True).split())
            if args.title_needle.casefold() in text.casefold():
                candidates.append((text, urljoin(landing.url, anchor["href"])))

        if len(candidates) != 1:
            raise RuntimeError(
                f"expected exactly one matching public PUB link, found {len(candidates)}: {candidates!r}"
            )

        link_text, href = candidates[0]
        response = session.get(
            href,
            headers={
                **headers,
                "Referer": landing.url,
                "Accept": "application/octet-stream,*/*",
            },
            timeout=90,
            allow_redirects=True,
        )
        response.raise_for_status()
        payload = response.content
        acquisition_route = "landing_exact_title_link"
    except (requests.RequestException, RuntimeError) as exc:
        landing_error = f"{type(exc).__name__}: {exc}"
        if not args.direct_candidate:
            raise SystemExit(f"landing acquisition failed and no direct candidate is pinned: {landing_error}")
        href = args.direct_candidate
        response = session.get(
            href,
            headers={
                **headers,
                "Referer": args.landing,
                "Accept": "application/octet-stream,*/*",
            },
            timeout=90,
            allow_redirects=True,
        )
        response.raise_for_status()
        payload = response.content
        acquisition_route = "direct_candidate_sha_pinned"

    if not payload.startswith(OLE_MAGIC):
        head = payload[:80]
        raise SystemExit(
            f"resolved file is not OLE/CFB (status={response.status_code}, "
            f"content-type={response.headers.get('content-type')!r}, head={head!r})"
        )

    sha256 = hashlib.sha256(payload).hexdigest()
    if args.expected_sha256 and sha256.casefold() != args.expected_sha256.casefold():
        raise SystemExit(
            f"resolved PUB SHA-256 mismatch: expected {args.expected_sha256.casefold()}, got {sha256}"
        )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(payload)

    receipt = {
        "schema": "chaptera.public-pub-acquisition.v1",
        "landing_url": args.landing,
        "landing_final_url": landing.url if landing is not None else None,
        "landing_error": landing_error,
        "matched_link_text": link_text,
        "resolved_href": href,
        "acquisition_route": acquisition_route,
        "download_final_url": response.url,
        "content_type": response.headers.get("content-type"),
        "byte_len": len(payload),
        "sha256": sha256,
        "expected_sha256": args.expected_sha256,
        "ole_magic_ok": True,
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
