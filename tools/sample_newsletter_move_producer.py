#!/usr/bin/env python3
"""Minimum real-fixture MoveNode producer for WEB-REVISION-ADAPTER-01.

This is intentionally not a PUB parser or a general Editor implementation.
It carries only the task-local mutation semantics required by the Rar revision
receipt and a source-free baseline extracted from a pinned real SampleNewsletter
resolved-graph artifact.

The bounded session reproduces the historical EditorSession invariants used by
page_owned_node_move_is_canonical_undoable_and_replayable:
- source identity is immutable;
- the target must be a positive-size, directly page-owned identity-transform node;
- the command supplies only target x/y;
- canonical before-state comes from current session state;
- MoveNode preserves width/height and fails closed on i64 overflow/no-op;
- project replay regenerates the same canonical operation through move_node_to.
"""

from __future__ import annotations

import copy
import json
import pathlib
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE_PATH = (
    ROOT
    / "packages"
    / "protocol"
    / "revision"
    / "v1"
    / "fixtures"
    / "sample-newsletter-move-baseline.real.json"
)

I64_MIN = -(1 << 63)
I64_MAX = (1 << 63) - 1
IDENTITY_TRANSFORM = {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0}


class ProducerError(RuntimeError):
    pass


def require_exact_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        raise ProducerError(
            f"{label} fields mismatch: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )


def require_int(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ProducerError(f"{label} must be an integer")
    if value < I64_MIN or value > I64_MAX:
        raise ProducerError(f"{label} is outside signed i64")
    return value


def checked_i64_add(left: int, right: int, label: str) -> int:
    result = left + right
    if result < I64_MIN or result > I64_MAX:
        raise ProducerError(f"{label} overflows signed i64")
    return result


def validate_rect(rect: dict[str, Any], label: str) -> dict[str, int]:
    require_exact_keys(rect, {"x", "y", "width", "height"}, label)
    normalized = {
        field: require_int(rect[field], f"{label}.{field}")
        for field in ("x", "y", "width", "height")
    }
    if normalized["width"] <= 0 or normalized["height"] <= 0:
        raise ProducerError(f"{label} must have positive width/height")
    checked_i64_add(normalized["x"], normalized["width"], f"{label}.right")
    checked_i64_add(normalized["y"], normalized["height"], f"{label}.bottom")
    return normalized


def load_baseline(path: pathlib.Path = BASELINE_PATH) -> dict[str, Any]:
    baseline = json.loads(path.read_text(encoding="utf-8"))
    require_exact_keys(
        baseline,
        {"baseline_version", "source", "provenance", "page_ids", "move_candidate"},
        "baseline",
    )
    if baseline["baseline_version"] != "chaptera.sample-newsletter-move-baseline.v1":
        raise ProducerError("unsupported baseline version")

    source = baseline["source"]
    require_exact_keys(source, {"sha256", "byte_len"}, "baseline.source")
    if source["sha256"] != "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf":
        raise ProducerError("unexpected pinned source hash")
    if source["byte_len"] != 291840:
        raise ProducerError("unexpected pinned source length")

    page_ids = baseline["page_ids"]
    if not isinstance(page_ids, list) or not page_ids or len(set(page_ids)) != len(page_ids):
        raise ProducerError("page_ids must be a non-empty unique list")

    candidate = baseline["move_candidate"]
    require_exact_keys(
        candidate,
        {"node_id", "parent_page_id", "kind", "transform", "before"},
        "move_candidate",
    )
    if candidate["parent_page_id"] not in page_ids:
        raise ProducerError("move candidate is not directly page-owned")
    if candidate["transform"] != IDENTITY_TRANSFORM:
        raise ProducerError("move candidate transform is not identity")
    candidate["before"] = validate_rect(candidate["before"], "move_candidate.before")
    return baseline


class SampleNewsletterMoveSession:
    """Task-local canonical mutation authority for one pinned real baseline."""

    def __init__(self, baseline: dict[str, Any]):
        self.baseline = copy.deepcopy(baseline)
        self.source_hash = baseline["source"]["sha256"]
        self.node_id = baseline["move_candidate"]["node_id"]
        self.current_rect = copy.deepcopy(baseline["move_candidate"]["before"])
        self.operations: list[dict[str, Any]] = []

    def project(self) -> dict[str, Any]:
        return {
            "schema_version": "pub-editor-v0.4" if self.operations else "pub-editor-v0.2",
            "source_hash": self.source_hash,
            "operations": copy.deepcopy(self.operations),
        }

    def move_node_to(self, node_id: str, x: Any, y: Any) -> dict[str, Any]:
        if node_id != self.node_id:
            raise ProducerError("node_move_unsupported")

        x = require_int(x, "command.x_emu")
        y = require_int(y, "command.y_emu")
        before = copy.deepcopy(self.current_rect)
        after = {
            "x": x,
            "y": y,
            "width": before["width"],
            "height": before["height"],
        }
        validate_rect(after, "canonical_move.after")
        if before == after:
            raise ProducerError("node_move_no_change")

        operation = {
            "kind": "move_node",
            "node_id": node_id,
            "before": before,
            "after": after,
        }
        self.current_rect = copy.deepcopy(after)
        self.operations.append(copy.deepcopy(operation))
        return operation

    def apply_project(self, project: dict[str, Any]) -> None:
        require_exact_keys(project, {"schema_version", "source_hash", "operations"}, "project")
        if self.operations:
            raise ProducerError("project replay requires a fresh session")
        if project["source_hash"] != self.source_hash:
            raise ProducerError("project source hash mismatch")
        if project["schema_version"] not in {
            "pub-editor-v0.1",
            "pub-editor-v0.2",
            "pub-editor-v0.3",
            "pub-editor-v0.4",
        }:
            raise ProducerError("unsupported project schema")
        operations = project["operations"]
        if not isinstance(operations, list):
            raise ProducerError("project.operations must be an array")

        for expected in operations:
            if not isinstance(expected, dict):
                raise ProducerError("project operation must be an object")
            if expected.get("kind") != "move_node":
                raise ProducerError("this bounded slice accepts only MoveNode replay")
            if project["schema_version"] != "pub-editor-v0.4":
                raise ProducerError("MoveNode requires pub-editor-v0.4")
            require_exact_keys(expected, {"kind", "node_id", "before", "after"}, "move_node")
            validate_rect(expected["before"], "move_node.before")
            after = validate_rect(expected["after"], "move_node.after")
            actual = self.move_node_to(expected["node_id"], after["x"], after["y"])
            if actual != expected:
                raise ProducerError("canonical operation mismatch during replay")

        if self.project() != project:
            raise ProducerError("replayed project is not canonical")


def handle(payload: dict[str, Any], baseline: dict[str, Any] | None = None) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise ProducerError("request must be an object")
    baseline = load_baseline() if baseline is None else copy.deepcopy(baseline)
    source_hash = baseline["source"]["sha256"]

    action = payload.get("action")
    if action == "baseline":
        require_exact_keys(payload, {"action", "source_hash"}, "baseline request")
        if payload["source_hash"] != source_hash:
            raise ProducerError("source hash mismatch")
        session = SampleNewsletterMoveSession(baseline)
        return {
            "source_hash": source_hash,
            "baseline_project": session.project(),
            "move_candidate": {
                "node_id": session.node_id,
                "before": copy.deepcopy(session.current_rect),
            },
        }

    if action != "commit":
        raise ProducerError("unsupported action")

    require_exact_keys(
        payload,
        {"action", "source_hash", "base_project", "command"},
        "commit request",
    )
    if payload["source_hash"] != source_hash:
        raise ProducerError("source hash mismatch")

    session = SampleNewsletterMoveSession(baseline)
    if payload["base_project"] != session.project():
        raise ProducerError("commit base_project is not the canonical baseline")

    command = payload["command"]
    if not isinstance(command, dict):
        raise ProducerError("command must be an object")
    require_exact_keys(command, {"kind", "node_id", "x_emu", "y_emu"}, "command")
    if command["kind"] != "move_node_to":
        raise ProducerError("unsupported command kind")

    operation = session.move_node_to(
        command["node_id"],
        command["x_emu"],
        command["y_emu"],
    )
    resulting_project = session.project()

    replay = SampleNewsletterMoveSession(baseline)
    replay.apply_project(resulting_project)
    replayed_project = replay.project()
    if replayed_project != resulting_project:
        raise ProducerError("fresh replay project mismatch")

    return {
        "canonical_operation": operation,
        "resulting_project": resulting_project,
        "replayed_project": replayed_project,
        "consequences": [
            {"key": "node.geometry.position", "state": "supported", "note": None}
        ],
        "source_hash_after": session.source_hash,
        "source_hash_replay": replay.source_hash,
    }


def main() -> int:
    try:
        payload = json.load(sys.stdin)
        result = handle(payload)
    except (ProducerError, json.JSONDecodeError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2
    json.dump(result, sys.stdout, ensure_ascii=False, separators=(",", ":"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
