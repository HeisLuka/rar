#!/usr/bin/env python3
"""Mine public GitHub repository history/releases for Microsoft Publisher payloads.

Discovery only. Emits immutable raw.githubusercontent.com commit URLs and public
release-asset URLs into the shared Rar acquisition seed contract.
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import re
import subprocess
import tempfile
import time
from collections import defaultdict
from pathlib import Path
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

UA = "rar-pub-github-history/1.0 (public format research)"
GITHUB_API = "https://api.github.com"
PUB_RE = re.compile(r"[.]pub$", re.I)
ARCHIVE_RE = re.compile(r"[.](?:zip|7z|rar|cab|iso)$", re.I)


def api_json(path: str, token: str, timeout: float = 30.0):
    req = Request(
        GITHUB_API + path,
        headers={
            "User-Agent": UA,
            "Accept": "application/vnd.github+json",
            **({"Authorization": f"Bearer {token}"} if token else {}),
        },
    )
    with urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read(16 * 1024 * 1024 + 1).decode("utf-8"))


def search_repositories(token: str, pages: int, per_page: int = 100) -> set[str]:
    repos: set[str] = set()
    if not token:
        return repos
    for page in range(1, pages + 1):
        q = urlencode({"q": "extension:pub", "per_page": per_page, "page": page})
        data = api_json("/search/code?" + q, token)
        items = data.get("items", [])
        for item in items:
            repo = item.get("repository") or {}
            full = str(repo.get("full_name") or "").strip()
            if full:
                repos.add(full)
        if len(items) < per_page:
            break
        time.sleep(1)
    return repos


def run(cmd: list[str], cwd: Path | None = None, timeout: int = 180) -> str:
    p = subprocess.run(
        cmd, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if p.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd[:4])} failed: {p.stderr[-1200:]}")
    return p.stdout


def clone_repo(repo: str, dest: Path, depth: int) -> None:
    cmd = [
        "git", "clone", "--quiet", "--filter=blob:none", "--no-checkout",
        "--depth", str(depth), f"https://github.com/{repo}.git", str(dest),
    ]
    run(cmd, timeout=300)


def history_rows(repo: str, checkout: Path, max_paths: int, max_per_path: int):
    specs = [":(icase,glob)*.pub", ":(icase,glob)**/*.pub"]
    text = run(
        ["git", "log", "--all", "--format=@@%H%x09%aI", "--name-only", "--", *specs],
        cwd=checkout, timeout=240,
    )
    current_commit = ""
    current_date = ""
    by_path: dict[str, list[tuple[str, str]]] = defaultdict(list)
    for line in text.splitlines():
        if line.startswith("@@"):
            parts = line[2:].split("\t", 1)
            current_commit = parts[0].strip()
            current_date = parts[1].strip() if len(parts) > 1 else ""
            continue
        path = line.strip()
        if current_commit and path and PUB_RE.search(path):
            bucket = by_path[path]
            if len(bucket) < max_per_path and all(c != current_commit for c, _ in bucket):
                bucket.append((current_commit, current_date))
        if len(by_path) >= max_paths:
            # Keep parsing only already-known paths; avoids unbounded path growth.
            pass

    selected_paths = sorted(by_path)[:max_paths]
    out = []
    for path in selected_paths:
        for commit, date in by_path[path][:max_per_path]:
            raw = (
                f"https://raw.githubusercontent.com/{repo}/{commit}/"
                + quote(path, safe="/")
            )
            out.append({
                "source_page": f"https://github.com/{repo}/commit/{commit}",
                "direct_url": raw,
                "candidate_filename": Path(path).name,
                "quarantine": "",
                "source_class": "github_history",
                "notes": "public Git history locator; commit time is provenance, not Publisher writer-version proof",
                "github_repository": repo,
                "github_commit": commit,
                "github_commit_date": date,
                "github_path": path,
                "github_locator_kind": "history_blob",
            })
    return out


def release_rows(repo: str, token: str, max_releases: int):
    if not token:
        return []
    out = []
    try:
        releases = api_json(
            f"/repos/{repo}/releases?per_page={min(max_releases, 100)}", token
        )
    except Exception:
        return out
    for rel in releases[:max_releases]:
        tag = str(rel.get("tag_name") or "")
        rid = str(rel.get("id") or "")
        for asset in rel.get("assets") or []:
            name = str(asset.get("name") or "")
            if not (PUB_RE.search(name) or ARCHIVE_RE.search(name)):
                continue
            url = str(asset.get("browser_download_url") or "")
            if not url:
                continue
            out.append({
                "source_page": str(rel.get("html_url") or f"https://github.com/{repo}/releases/tag/{quote(tag)}"),
                "direct_url": url,
                "candidate_filename": name,
                "quarantine": "",
                "source_class": "github_release_asset",
                "notes": "public GitHub release asset locator",
                "github_repository": repo,
                "github_release_id": rid,
                "github_release_tag": tag,
                "github_asset_id": str(asset.get("id") or ""),
                "github_asset_size": str(asset.get("size") or ""),
                "github_locator_kind": "release_asset",
            })
    return out


def write_csv(rows, path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    keys, seen = [], set()
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key)
                keys.append(key)
    with path.open("w", encoding="utf-8", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys)
        w.writeheader()
        w.writerows(rows)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", action="append", default=[])
    ap.add_argument("--repo-file", type=Path)
    ap.add_argument("--no-search", action="store_true")
    ap.add_argument("--search-pages", type=int, default=3)
    ap.add_argument("--max-repos", type=int, default=25)
    ap.add_argument("--history-depth", type=int, default=2000)
    ap.add_argument("--max-paths-per-repo", type=int, default=250)
    ap.add_argument("--max-captures-per-path", type=int, default=3)
    ap.add_argument("--max-releases", type=int, default=50)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--summary", type=Path)
    args = ap.parse_args()

    token = os.environ.get("GITHUB_TOKEN", "")
    repos = {x.strip() for x in args.repo if x.strip()}
    if args.repo_file and args.repo_file.exists():
        repos |= {
            x.strip() for x in args.repo_file.read_text(encoding="utf-8").splitlines()
            if x.strip() and not x.lstrip().startswith("#")
        }
    if not args.no_search:
        try:
            repos |= search_repositories(token, args.search_pages)
        except Exception as exc:
            print(f"github code search degraded: {type(exc).__name__}: {exc}", file=sys.stderr)

    repos = set(sorted(repos)[: args.max_repos])
    rows = []
    errors = []
    with tempfile.TemporaryDirectory(prefix="pub-github-history-") as td:
        root = Path(td)
        for i, repo in enumerate(sorted(repos)):
            checkout = root / f"r{i}"
            try:
                clone_repo(repo, checkout, args.history_depth)
                rows.extend(history_rows(
                    repo, checkout, args.max_paths_per_repo, args.max_captures_per_path
                ))
                rows.extend(release_rows(repo, token, args.max_releases))
            except Exception as exc:
                errors.append({"repository": repo, "error": f"{type(exc).__name__}: {exc}"})

    dedup = {}
    for row in rows:
        dedup.setdefault(row["direct_url"], row)
    final = sorted(dedup.values(), key=lambda r: (r["github_repository"], r["direct_url"]))
    write_csv(final, args.out)
    summary = {
        "schema": "rar-github-history-v1",
        "repositories_attempted": len(repos),
        "locator_rows": len(final),
        "history_rows": sum(r.get("github_locator_kind") == "history_blob" for r in final),
        "release_rows": sum(r.get("github_locator_kind") == "release_asset" for r in final),
        "errors": errors,
    }
    sp = args.summary or args.out.with_suffix(".summary.json")
    sp.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    import sys
    raise SystemExit(main())
