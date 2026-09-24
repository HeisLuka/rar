#!/usr/bin/env python3
"""Deterministic source-neutral visual regression oracle for VISREG-01."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any


REPORT_SCHEMA = "chaptera.visreg.report.v1"
FIXTURE_SCHEMA = "chaptera.visreg.fixture.v1"


def read_json(path: Path) -> tuple[dict[str, Any], str]:
    raw = path.read_bytes()
    data = json.loads(raw.decode("utf-8"))
    if data.get("schema") != FIXTURE_SCHEMA:
        raise ValueError(f"{path}: expected schema {FIXTURE_SCHEMA!r}")
    return data, hashlib.sha256(raw).hexdigest()


def stable(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: stable(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        return [stable(item) for item in value]
    return value


def index_by(rows: list[dict[str, Any]], key: str, where: str) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for row in rows:
        value = row.get(key)
        if not isinstance(value, str) or not value:
            raise ValueError(f"{where}: each row must have non-empty {key}")
        if value in result:
            raise ValueError(f"{where}: duplicate {key}={value!r}")
        result[value] = row
    return result


def origin(page_id: str, obj: dict[str, Any] | None = None) -> dict[str, Any]:
    out: dict[str, Any] = {"page_id": page_id}
    if obj is not None:
        out["node_id"] = obj.get("node_id")
        if obj.get("story_id") is not None:
            out["story_id"] = obj.get("story_id")
    return out


def add_diff(
    diffs: list[dict[str, Any]],
    *,
    stage: str,
    code: str,
    at: dict[str, Any],
    golden: Any,
    candidate: Any,
) -> None:
    diffs.append(
        {
            "stage": stage,
            "code": code,
            "origin": stable(at),
            "golden": stable(golden),
            "candidate": stable(candidate),
        }
    )


def compare(golden: dict[str, Any], candidate: dict[str, Any]) -> list[dict[str, Any]]:
    diffs: list[dict[str, Any]] = []

    gp = index_by(golden.get("pages", []), "page_id", "golden.pages")
    cp = index_by(candidate.get("pages", []), "page_id", "candidate.pages")

    for page_id in sorted(set(gp) | set(cp)):
        gpage = gp.get(page_id)
        cpage = cp.get(page_id)
        if gpage is None:
            add_diff(
                diffs,
                stage="structural",
                code="page_added",
                at={"page_id": page_id},
                golden=None,
                candidate=cpage,
            )
            continue
        if cpage is None:
            add_diff(
                diffs,
                stage="structural",
                code="page_removed",
                at={"page_id": page_id},
                golden=gpage,
                candidate=None,
            )
            continue

        if stable(gpage.get("size_emu")) != stable(cpage.get("size_emu")):
            add_diff(
                diffs,
                stage="geometry",
                code="page_size_changed",
                at={"page_id": page_id},
                golden=gpage.get("size_emu"),
                candidate=cpage.get("size_emu"),
            )

        go = index_by(gpage.get("objects", []), "node_id", f"golden.pages[{page_id}].objects")
        co = index_by(cpage.get("objects", []), "node_id", f"candidate.pages[{page_id}].objects")

        for node_id in sorted(set(go) | set(co)):
            gobj = go.get(node_id)
            cobj = co.get(node_id)
            if gobj is None:
                add_diff(
                    diffs,
                    stage="structural",
                    code="object_added",
                    at=origin(page_id, cobj),
                    golden=None,
                    candidate=cobj,
                )
                continue
            if cobj is None:
                add_diff(
                    diffs,
                    stage="structural",
                    code="object_removed",
                    at=origin(page_id, gobj),
                    golden=gobj,
                    candidate=None,
                )
                continue

            at = origin(page_id, cobj)

            if gobj.get("kind") != cobj.get("kind"):
                add_diff(
                    diffs,
                    stage="structural",
                    code="object_kind_changed",
                    at=at,
                    golden=gobj.get("kind"),
                    candidate=cobj.get("kind"),
                )

            if stable(gobj.get("bounds_emu")) != stable(cobj.get("bounds_emu")):
                add_diff(
                    diffs,
                    stage="geometry",
                    code="object_bounds_changed",
                    at=at,
                    golden=gobj.get("bounds_emu"),
                    candidate=cobj.get("bounds_emu"),
                )

            if gobj.get("story_id") != cobj.get("story_id"):
                add_diff(
                    diffs,
                    stage="text_layout",
                    code="story_origin_changed",
                    at=at,
                    golden=gobj.get("story_id"),
                    candidate=cobj.get("story_id"),
                )

            if gobj.get("text") != cobj.get("text"):
                add_diff(
                    diffs,
                    stage="text_layout",
                    code="text_changed",
                    at=at,
                    golden=gobj.get("text"),
                    candidate=cobj.get("text"),
                )

            if stable(gobj.get("lines")) != stable(cobj.get("lines")):
                add_diff(
                    diffs,
                    stage="text_layout",
                    code="line_layout_changed",
                    at=at,
                    golden=gobj.get("lines"),
                    candidate=cobj.get("lines"),
                )

            grender = stable(gobj.get("render"))
            crender = stable(cobj.get("render"))
            if grender != crender:
                add_diff(
                    diffs,
                    stage="render",
                    code="render_artifact_changed",
                    at=at,
                    golden=grender,
                    candidate=crender,
                )

    return sorted(
        diffs,
        key=lambda d: (
            d["origin"].get("page_id", ""),
            d["origin"].get("node_id") or "",
            d["stage"],
            d["code"],
        ),
    )


def classify(diffs: list[dict[str, Any]]) -> str:
    if not diffs:
        return "clean"
    stages = {d["stage"] for d in diffs}
    if stages == {"render"}:
        return "render_only"
    if "structural" in stages:
        return "structural"
    if "geometry" in stages:
        return "geometry"
    if "text_layout" in stages:
        return "text_layout"
    return "render"


def build_report(
    golden: dict[str, Any],
    candidate: dict[str, Any],
    golden_sha: str,
    candidate_sha: str,
) -> dict[str, Any]:
    diffs = compare(golden, candidate)
    counts: dict[str, int] = {}
    for diff in diffs:
        counts[diff["stage"]] = counts.get(diff["stage"], 0) + 1
    return {
        "schema": REPORT_SCHEMA,
        "golden": {
            "fixture_id": golden.get("fixture_id"),
            "sha256": golden_sha,
        },
        "candidate": {
            "fixture_id": candidate.get("fixture_id"),
            "sha256": candidate_sha,
        },
        "summary": {
            "equivalent": not diffs,
            "primary_stage": classify(diffs),
            "difference_count": len(diffs),
            "stage_counts": {key: counts[key] for key in sorted(counts)},
        },
        "differences": diffs,
    }


def write_report(report: dict[str, Any], output: Path | None) -> str:
    rendered = json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    if output is None:
        sys.stdout.write(rendered)
    else:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered, encoding="utf-8", newline="\n")
    return rendered


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--golden", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument(
        "--expect-stage",
        choices=["clean", "structural", "geometry", "text_layout", "render_only", "render"],
    )
    args = parser.parse_args()

    try:
        golden, golden_sha = read_json(args.golden)
        candidate, candidate_sha = read_json(args.candidate)
        report = build_report(golden, candidate, golden_sha, candidate_sha)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"visreg: {exc}", file=sys.stderr)
        return 2

    write_report(report, args.out)
    actual = report["summary"]["primary_stage"]

    if args.expect_stage is not None:
        if actual != args.expect_stage:
            print(
                f"visreg: expected primary stage {args.expect_stage!r}, got {actual!r}",
                file=sys.stderr,
            )
            return 1
        return 0

    return 0 if report["summary"]["equivalent"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
