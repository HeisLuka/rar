#!/usr/bin/env python3
"""Role-aware GitHub execution-owner uniqueness guard.

Public-safe invariant:
  one exact Notion task authority -> at most one live task issue
                                + at most one explicitly paired implementation PR.

The enforcer reads only public GitHub issue/PR metadata. It never calls Notion.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Iterable

AUTHORITY_LINE_RE = re.compile(
    r"(?im)^\s*Notion(?:\s+(?:authority|owner))?\s*:\s*"
    r"(https://app\.notion\.com/p/[0-9a-fA-F-]{32,36}(?:\?[^\s]*)?)\s*$"
)
NOTION_PAGE_RE = re.compile(r"/p/([0-9a-fA-F-]{32,36})")
OWNER_RE = re.compile(r"(?im)^\s*Owner\s*:\s*#(\d+)\s*$")
SUPERSEDES_RE = re.compile(r"(?im)^\s*Supersedes\s*:\s*#(\d+)\s*$")
MAX_OPEN_OBJECT_PAGES = 20


@dataclasses.dataclass(frozen=True)
class OwnerRecord:
    number: int
    role: str
    authority: str
    task_key: str | None
    owner_issue: int | None
    supersedes: int | None
    created_at: str
    title: str


@dataclasses.dataclass(frozen=True)
class DuplicateAction:
    number: int
    role: str
    canonical_number: int
    reason: str


@dataclasses.dataclass
class Decision:
    authority: str
    status: str
    canonical_issue: int | None
    canonical_pr: int | None
    duplicates: list[DuplicateAction]
    ambiguities: list[str]

    def as_dict(self) -> dict[str, Any]:
        return {
            "authority": self.authority,
            "status": self.status,
            "canonical_issue": self.canonical_issue,
            "canonical_pr": self.canonical_pr,
            "duplicates": [dataclasses.asdict(item) for item in self.duplicates],
            "ambiguities": self.ambiguities,
        }


def normalize_authority(body: str | None) -> str | None:
    if not body:
        return None
    match = AUTHORITY_LINE_RE.search(body)
    if not match:
        return None
    page = NOTION_PAGE_RE.search(match.group(1))
    if not page:
        return None
    normalized = page.group(1).replace("-", "").lower()
    if len(normalized) != 32 or not all(ch in "0123456789abcdef" for ch in normalized):
        return None
    return normalized


def extract_owner_issue(body: str | None) -> int | None:
    if not body:
        return None
    match = OWNER_RE.search(body)
    return int(match.group(1)) if match else None


def extract_supersedes(body: str | None) -> int | None:
    if not body:
        return None
    match = SUPERSEDES_RE.search(body)
    return int(match.group(1)) if match else None


def normalize_task_key(title: str | None) -> str | None:
    if not title:
        return None
    value = title.strip()
    bracketed = re.match(r"^\[([A-Z0-9][A-Z0-9-]{3,})\]", value)
    if bracketed:
        return bracketed.group(1)
    plain = re.match(r"^([A-Z0-9][A-Z0-9-]{3,})(?=\s*[:—])", value)
    return plain.group(1) if plain else None


def record_from_api(item: dict[str, Any]) -> OwnerRecord | None:
    authority = normalize_authority(item.get("body"))
    if authority is None:
        return None
    role = "pr" if item.get("pull_request") is not None or item.get("_role") == "pr" else "issue"
    body = item.get("body") or ""
    return OwnerRecord(
        number=int(item["number"]),
        role=role,
        authority=authority,
        task_key=normalize_task_key(item.get("title")),
        owner_issue=extract_owner_issue(body) if role == "pr" else None,
        supersedes=extract_supersedes(body),
        created_at=str(item.get("created_at") or ""),
        title=str(item.get("title") or ""),
    )


def _sort_key(record: OwnerRecord) -> tuple[str, int]:
    return (record.created_at, record.number)


def decide(records: Iterable[OwnerRecord], authority: str) -> Decision:
    scoped = sorted((r for r in records if r.authority == authority), key=_sort_key)
    issues = [r for r in scoped if r.role == "issue"]
    prs = [r for r in scoped if r.role == "pr"]
    duplicates: list[DuplicateAction] = []
    ambiguities: list[str] = []

    issue_by_number = {r.number: r for r in issues}
    pr_by_owner: dict[int, list[OwnerRecord]] = {}
    for pr in prs:
        if pr.owner_issue is not None:
            pr_by_owner.setdefault(pr.owner_issue, []).append(pr)

    canonical_issue = issues[0] if issues else None
    if len(issues) > 1:
        for later in issues[1:]:
            if later.supersedes is not None:
                ambiguities.append(
                    f"issue #{later.number} declares Supersedes: #{later.supersedes} while predecessor remains live"
                )
                continue
            if (
                later.task_key
                and canonical_issue
                and canonical_issue.task_key
                and later.task_key != canonical_issue.task_key
            ):
                ambiguities.append(
                    f"issues #{canonical_issue.number} and #{later.number} share authority but task keys differ "
                    f"({canonical_issue.task_key} vs {later.task_key})"
                )
                continue
            if pr_by_owner.get(later.number):
                attached = ",".join(f"#{pr.number}" for pr in pr_by_owner[later.number])
                ambiguities.append(
                    f"later issue #{later.number} has live implementation PR(s) {attached}; refusing partial auto-close"
                )
                continue
            duplicates.append(
                DuplicateAction(
                    number=later.number,
                    role="issue",
                    canonical_number=canonical_issue.number,
                    reason="later live task issue for the same exact Notion authority",
                )
            )

    for pr in prs:
        if pr.owner_issue is None:
            ambiguities.append(f"PR #{pr.number} has the same authority but no exact Owner: #N relation")
        elif pr.owner_issue not in issue_by_number:
            ambiguities.append(
                f"PR #{pr.number} points to Owner: #{pr.owner_issue}, which is not a live issue for this authority"
            )

    for owner_number, owner_prs in sorted(pr_by_owner.items()):
        owner_prs = sorted(owner_prs, key=_sort_key)
        if len(owner_prs) <= 1:
            continue
        canonical_pr = owner_prs[0]
        for later in owner_prs[1:]:
            if later.supersedes is not None:
                ambiguities.append(
                    f"PR #{later.number} declares Supersedes: #{later.supersedes} while predecessor remains live"
                )
                continue
            if (
                later.task_key
                and canonical_pr.task_key
                and later.task_key != canonical_pr.task_key
            ):
                ambiguities.append(
                    f"PRs #{canonical_pr.number} and #{later.number} share authority/owner but task keys differ "
                    f"({canonical_pr.task_key} vs {later.task_key})"
                )
                continue
            duplicates.append(
                DuplicateAction(
                    number=later.number,
                    role="pr",
                    canonical_number=canonical_pr.number,
                    reason="later live implementation PR for the same exact Notion authority and Owner issue",
                )
            )

    canonical_pr: OwnerRecord | None = None
    if canonical_issue is not None:
        paired = sorted(pr_by_owner.get(canonical_issue.number, []), key=_sort_key)
        canonical_pr = paired[0] if paired else None

    if ambiguities:
        status = "ambiguous"
    elif duplicates:
        status = "duplicates_found"
    elif canonical_issue is not None and canonical_pr is not None:
        status = "valid_issue_pr_pair"
    elif len(scoped) == 1:
        status = "unique"
    elif not scoped:
        status = "not_found"
    else:
        status = "valid"

    return Decision(
        authority=authority,
        status=status,
        canonical_issue=canonical_issue.number if canonical_issue else None,
        canonical_pr=canonical_pr.number if canonical_pr else None,
        duplicates=duplicates,
        ambiguities=ambiguities,
    )


class GitHubClient:
    def __init__(self, token: str, repo: str, api_url: str = "https://api.github.com") -> None:
        self.token = token
        self.repo = repo
        self.api_url = api_url.rstrip("/")

    def request(self, method: str, path: str, payload: dict[str, Any] | None = None) -> Any:
        url = f"{self.api_url}{path}"
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(url, data=data, method=method)
        req.add_header("Accept", "application/vnd.github+json")
        req.add_header("Authorization", f"Bearer {self.token}")
        req.add_header("X-GitHub-Api-Version", "2022-11-28")
        req.add_header("User-Agent", "chaptera-execution-owner-uniqueness")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                raw = response.read()
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(
                f"GitHub API {method} {path} failed: {error.code} {detail}"
            ) from error
        return json.loads(raw) if raw else None

    def list_open_objects(self) -> list[dict[str, Any]]:
        out: list[dict[str, Any]] = []
        for page in range(1, MAX_OPEN_OBJECT_PAGES + 1):
            items = self.request(
                "GET",
                f"/repos/{self.repo}/issues?state=open&per_page=100&page={page}&sort=created&direction=asc",
            )
            if not isinstance(items, list):
                raise RuntimeError("GitHub open-issues response is not a list")
            out.extend(items)
            if len(items) < 100:
                return out
        raise RuntimeError(
            f"open GitHub object census exceeded {MAX_OPEN_OBJECT_PAGES * 100}; refusing incomplete uniqueness decision"
        )

    def comment(self, number: int, body: str) -> None:
        self.request("POST", f"/repos/{self.repo}/issues/{number}/comments", {"body": body})

    def close_issue_duplicate(self, number: int) -> None:
        self.request(
            "PATCH",
            f"/repos/{self.repo}/issues/{number}",
            {"state": "closed", "state_reason": "duplicate"},
        )

    def close_pr(self, number: int) -> None:
        self.request("PATCH", f"/repos/{self.repo}/pulls/{number}", {"state": "closed"})


def event_object(event: dict[str, Any]) -> dict[str, Any] | None:
    if isinstance(event.get("issue"), dict):
        return dict(event["issue"])
    if isinstance(event.get("pull_request"), dict):
        obj = dict(event["pull_request"])
        obj["_role"] = "pr"
        return obj
    return None


def merge_current_object(
    items: list[dict[str, Any]], current: dict[str, Any]
) -> list[dict[str, Any]]:
    number = int(current["number"])
    merged = [item for item in items if int(item.get("number", -1)) != number]
    merged.append(current)
    return merged


def write_receipt(
    path: Path,
    repo: str,
    event_name: str,
    current: OwnerRecord | None,
    decision: Decision | None,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    value = {
        "schema_version": "chaptera.execution-owner-uniqueness.v1",
        "repository": repo,
        "event_name": event_name,
        "current": None
        if current is None
        else {
            "number": current.number,
            "role": current.role,
            "authority": current.authority,
            "task_key": current.task_key,
            "owner_issue": current.owner_issue,
        },
        "decision": None if decision is None else decision.as_dict(),
    }
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def enforce(
    client: GitHubClient,
    event: dict[str, Any],
    event_name: str,
    receipt: Path,
    dry_run: bool,
) -> int:
    current_api = event_object(event)
    if current_api is None:
        write_receipt(receipt, client.repo, event_name, None, None)
        return 0
    current = record_from_api(current_api)
    if current is None:
        write_receipt(receipt, client.repo, event_name, None, None)
        return 0

    items = merge_current_object(client.list_open_objects(), current_api)
    records = [
        record
        for item in items
        if (record := record_from_api(item)) is not None
    ]
    decision = decide(records, current.authority)
    write_receipt(receipt, client.repo, event_name, current, decision)

    if decision.ambiguities:
        for message in decision.ambiguities:
            print(f"AMBIGUOUS: {message}", file=sys.stderr)
        return 2

    for duplicate in decision.duplicates:
        comment = (
            "Execution-owner uniqueness guard: this is a later same-role duplicate of "
            f"#{duplicate.canonical_number} for the exact Notion task authority. "
            "A parent issue plus its explicitly paired implementation PR is valid; "
            "this closure applies only to a same-role duplicate. "
            "If this object was intended as a successor, declare `Supersedes: #N` "
            "and resolve the predecessor state first."
        )
        print(
            f"duplicate #{duplicate.number} ({duplicate.role}) -> canonical "
            f"#{duplicate.canonical_number}: {duplicate.reason}"
        )
        if dry_run:
            continue
        client.comment(duplicate.number, comment)
        if duplicate.role == "issue":
            client.close_issue_duplicate(duplicate.number)
        else:
            client.close_pr(duplicate.number)

    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--event",
        type=Path,
        default=Path(os.environ.get("GITHUB_EVENT_PATH", "")),
    )
    parser.add_argument(
        "--receipt",
        type=Path,
        default=Path("target/execution-owner-uniqueness.json"),
    )
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    token = os.environ.get("GITHUB_TOKEN", "")
    event_name = os.environ.get("GITHUB_EVENT_NAME", "unknown")
    if not repo:
        raise SystemExit("GITHUB_REPOSITORY is required")
    if not token:
        raise SystemExit("GITHUB_TOKEN is required")
    if not args.event or not args.event.is_file():
        raise SystemExit(f"event file is required: {args.event}")
    event = json.loads(args.event.read_text(encoding="utf-8"))
    client = GitHubClient(
        token=token,
        repo=repo,
        api_url=os.environ.get("GITHUB_API_URL", "https://api.github.com"),
    )
    return enforce(client, event, event_name, args.receipt, args.dry_run)


if __name__ == "__main__":
    raise SystemExit(main())
