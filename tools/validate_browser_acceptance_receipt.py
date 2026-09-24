#!/usr/bin/env python3
import copy
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BROWSER_SCHEMA = ROOT / "apps" / "web" / "acceptance" / "browser-acceptance-receipt.schema.json"

sys.path.insert(0, str(ROOT / "tools"))
from scene_v1 import finalize_snapshot
from validate_revision_producer_receipt import hash_id, revision_id


def validate_schema(receipt):
    schema = json.loads(BROWSER_SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema).iter_errors(receipt),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError("browser acceptance receipt schema validation failed\n" + detail)


def expected_history_revision_ids(revision_receipt):
    document_id = revision_receipt["document_id"]
    source_hash = revision_receipt["source_hash"]
    baseline_revision_id = revision_receipt["baseline"]["revision_id"]
    baseline_state_id = revision_receipt["baseline"]["state_id"]
    accepted_revision_id = revision_receipt["accepted"]["revision_id"]
    accepted_state_id = revision_receipt["accepted"]["state_id"]

    undo_transition_hash = hash_id({
        "protocol_version": "chaptera.history-transition.v1",
        "kind": "undo",
        "base_revision_id": accepted_revision_id,
        "base_state_id": accepted_state_id,
        "resulting_state_id": baseline_state_id,
    })
    undo_revision_id = revision_id(
        document_id,
        source_hash,
        accepted_revision_id,
        baseline_state_id,
        "undo",
        undo_transition_hash,
    )

    redo_transition_hash = hash_id({
        "protocol_version": "chaptera.history-transition.v1",
        "kind": "redo",
        "base_revision_id": undo_revision_id,
        "base_state_id": baseline_state_id,
        "resulting_state_id": accepted_state_id,
    })
    redo_revision_id = revision_id(
        document_id,
        source_hash,
        undo_revision_id,
        accepted_state_id,
        "redo",
        redo_transition_hash,
    )
    return undo_revision_id, redo_revision_id


def _scene_with_revision_and_bounds(initial_scene, revision_id_value, node_id, bounds):
    scene = copy.deepcopy(initial_scene)
    scene["revision_id"] = revision_id_value
    matches = [node for node in scene["nodes"] if node.get("node_id") == node_id]
    if len(matches) != 1:
        raise AssertionError("selected canonical node is not unique in initial Scene")
    matches[0]["bounds"] = copy.deepcopy(bounds)
    return finalize_snapshot(scene)


def expected_scene_snapshot_ids(revision_receipt, initial_scene):
    operation = revision_receipt["accepted"]["canonical_operation"]
    node_id = operation["node_id"]
    before = operation["before"]
    after = operation["after"]
    baseline_revision_id = revision_receipt["baseline"]["revision_id"]
    accepted_revision_id = revision_receipt["accepted"]["revision_id"]
    undo_revision_id, redo_revision_id = expected_history_revision_ids(revision_receipt)

    if initial_scene.get("revision_id") != baseline_revision_id:
        raise AssertionError("initial Scene is not bound to canonical baseline revision")
    initial_nodes = [node for node in initial_scene["nodes"] if node.get("node_id") == node_id]
    if len(initial_nodes) != 1:
        raise AssertionError("canonical MoveNode target is not unique in initial Scene")
    if initial_nodes[0].get("bounds") != before:
        raise AssertionError("initial Scene geometry does not equal canonical MoveNode before-state")

    accepted_scene = _scene_with_revision_and_bounds(
        initial_scene,
        accepted_revision_id,
        node_id,
        after,
    )
    undo_scene = _scene_with_revision_and_bounds(
        initial_scene,
        undo_revision_id,
        node_id,
        before,
    )
    redo_scene = _scene_with_revision_and_bounds(
        initial_scene,
        redo_revision_id,
        node_id,
        after,
    )
    return {
        "initial": initial_scene["snapshot_id"],
        "accepted": accepted_scene["snapshot_id"],
        "undo": undo_scene["snapshot_id"],
        "redo": redo_scene["snapshot_id"],
        # Reopen loads the already-persisted final redo revision; it does not create a revision.
        "reopen": redo_scene["snapshot_id"],
    }


def validate_semantics(browser, revision_receipt, initial_scene):
    operation = revision_receipt["accepted"]["canonical_operation"]
    request = revision_receipt["request"]

    if browser["initial_revision_id"] != revision_receipt["baseline"]["revision_id"]:
        raise AssertionError("browser initial revision is not canonical baseline")
    if browser["accepted_revision_id"] != revision_receipt["accepted"]["revision_id"]:
        raise AssertionError("browser accepted revision is not canonical accepted revision")
    if browser["selected_node_id"] != operation["node_id"]:
        raise AssertionError("browser selected NodeId is not canonical MoveNode target")
    if browser["client_operation_id"] != request["client_operation_id"]:
        raise AssertionError("browser client operation id is not canonical accepted request")
    if browser["before_rect"] != operation["before"]:
        raise AssertionError("browser before_rect is not canonical server before-state")
    if browser["after_rect"] != operation["after"]:
        raise AssertionError("browser after_rect is not canonical accepted after-state")

    before = operation["before"]
    after = operation["after"]
    if (before["width"], before["height"]) != (after["width"], after["height"]):
        raise AssertionError("bounded MoveNode must preserve width/height")

    undo_revision_id, redo_revision_id = expected_history_revision_ids(revision_receipt)
    if browser["undo_revision_id"] != undo_revision_id:
        raise AssertionError("browser Undo revision does not match public history law")
    if browser["redo_revision_id"] != redo_revision_id:
        raise AssertionError("browser Redo revision does not match public history law")

    revisions = [
        browser["initial_revision_id"],
        browser["accepted_revision_id"],
        browser["undo_revision_id"],
        browser["redo_revision_id"],
    ]
    if len(set(revisions)) != 4:
        raise AssertionError("baseline/Move/Undo/Redo must be fresh distinct revisions")

    expected_snapshots = expected_scene_snapshot_ids(revision_receipt, initial_scene)
    if browser["scene_snapshot_ids"] != expected_snapshots:
        raise AssertionError("browser Scene snapshot chain does not match canonical Move/Undo/Redo/reopen states")

    return {
        "receipt_kind": browser["receipt_version"],
        "selected_node_id": browser["selected_node_id"],
        "client_operation_id": browser["client_operation_id"],
        "initial_revision_id": browser["initial_revision_id"],
        "accepted_revision_id": browser["accepted_revision_id"],
        "undo_revision_id": undo_revision_id,
        "redo_revision_id": redo_revision_id,
        "scene_snapshot_ids": expected_snapshots,
        "canonical_move_bound": True,
        "history_bound": True,
        "scene_chain_bound": True,
    }


def main():
    if len(sys.argv) != 4:
        print(
            "usage: validate_browser_acceptance_receipt.py "
            "BROWSER.json REVISION.json INITIAL_SCENE.json",
            file=sys.stderr,
        )
        return 2
    browser = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    revision = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
    initial_scene = json.loads(pathlib.Path(sys.argv[3]).read_text(encoding="utf-8"))
    validate_schema(browser)
    summary = validate_semantics(browser, revision, initial_scene)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
