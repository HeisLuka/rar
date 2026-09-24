#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
REV = ROOT / "packages" / "protocol" / "revision" / "v1"
SCHEMA = REV / "producer-receipt.schema.json"


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def hash_id(value):
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def project_hash(project):
    return hash_id(project)


def state_id(document_id, source_hash, project):
    return hash_id({
        "protocol_version": "chaptera.authoring-state.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "project_schema_version": project["schema_version"],
        "project_hash": project_hash(project),
    })


def revision_id(document_id, source_hash, parent_revision_id, sid, transition_kind, transition_hash):
    return hash_id({
        "protocol_version": "chaptera.revision-node.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "parent_revision_id": parent_revision_id,
        "state_id": sid,
        "transition_kind": transition_kind,
        "transition_hash": transition_hash,
    })


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("producer receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    document_id = receipt["document_id"]
    source_hash = receipt["source_hash"]
    baseline = receipt["baseline"]
    request = receipt["request"]
    accepted = receipt["accepted"]
    resulting = receipt["resulting_project"]
    replayed = receipt["replayed_project"]

    if "before" in request.get("command", {}):
        raise AssertionError("browser request must not contain authoritative before-state")

    if request.get("document_id") != document_id or accepted.get("document_id") != document_id:
        raise AssertionError("document identity mismatch")
    if baseline["project"].get("source_hash") != source_hash:
        raise AssertionError("baseline project source identity mismatch")
    if request.get("source_hash") != source_hash or accepted.get("source_hash") != source_hash:
        raise AssertionError("source identity mismatch")
    if request.get("base_revision_id") != baseline["revision_id"]:
        raise AssertionError("request is not anchored to baseline revision")
    if accepted.get("base_revision_id") != baseline["revision_id"]:
        raise AssertionError("accepted result is not anchored to baseline revision")
    if accepted.get("client_operation_id") != request.get("client_operation_id"):
        raise AssertionError("client operation identity mismatch")

    if project_hash(baseline["project"]) != baseline["project_hash"]:
        raise AssertionError("baseline project_hash mismatch")
    baseline_sid = state_id(document_id, source_hash, baseline["project"])
    if baseline_sid != baseline["state_id"]:
        raise AssertionError("baseline state_id mismatch")
    expected_baseline_rid = revision_id(document_id, source_hash, None, baseline_sid, "baseline", None)
    if expected_baseline_rid != baseline["revision_id"]:
        raise AssertionError("baseline revision_id mismatch")

    op = accepted.get("canonical_operation")
    if not isinstance(op, dict) or op.get("kind") != "move_node":
        raise AssertionError("accepted canonical operation must be move_node")
    cmd = request.get("command") or {}
    if cmd.get("kind") != "move_node_to":
        raise AssertionError("request command must be move_node_to")
    if op.get("node_id") != cmd.get("node_id"):
        raise AssertionError("canonical operation target mismatch")
    before = op.get("before")
    after = op.get("after") or {}
    if not isinstance(before, dict):
        raise AssertionError("canonical before-state is missing")
    if after.get("x") != cmd.get("x_emu") or after.get("y") != cmd.get("y_emu"):
        raise AssertionError("canonical after-position does not match accepted intent")

    if resulting.get("source_hash") != source_hash or replayed.get("source_hash") != source_hash:
        raise AssertionError("source hash changed")
    if canonical_json(resulting) != canonical_json(replayed):
        raise AssertionError("replayed canonical project differs from resulting project")

    baseline_ops = baseline["project"]["operations"]
    result_ops = resulting["operations"]
    if len(result_ops) != len(baseline_ops) + 1:
        raise AssertionError("bounded MoveNode arm must add exactly one canonical operation")
    if canonical_json(result_ops[-1]) != canonical_json(op):
        raise AssertionError("resulting project tail is not the accepted canonical operation")

    transition_hash = hash_id(op)
    result_sid = state_id(document_id, source_hash, resulting)
    expected_rid = revision_id(
        document_id,
        source_hash,
        baseline["revision_id"],
        result_sid,
        "commit",
        transition_hash,
    )
    if accepted.get("project_schema_version") != resulting.get("schema_version"):
        raise AssertionError("accepted project schema version mismatch")
    if accepted.get("state_id") != result_sid:
        raise AssertionError("accepted state_id mismatch")
    if accepted.get("revision_id") != expected_rid:
        raise AssertionError("accepted revision_id mismatch")

    probes = receipt["probes"]
    if probes["stale_base"]["code"] != "stale_revision":
        raise AssertionError("stale-base probe code mismatch")
    if probes["idempotency_conflict"]["code"] != "idempotency_conflict":
        raise AssertionError("idempotency-conflict probe code mismatch")
    for key in ("stale_base", "idempotency_conflict"):
        probe = probes[key]
        if probe["before_revision_id"] != probe["after_revision_id"]:
            raise AssertionError(f"{key} mutated current revision")

    source_probe = probes["source_identity"]
    if not (source_probe["before"] == source_probe["after"] == source_probe["replay"] == source_hash):
        raise AssertionError("source identity changed across commit/replay")

    return {
        "receipt_kind": receipt["receipt_version"],
        "producer": receipt["producer"],
        "document_id": document_id,
        "source_hash": source_hash,
        "baseline_revision_id": baseline["revision_id"],
        "accepted_revision_id": accepted["revision_id"],
        "accepted_state_id": accepted["state_id"],
        "canonical_operation_hash": transition_hash,
        "replay_equal": True,
        "stale_no_mutation": True,
        "idempotent_retry_single_execution": True,
        "idempotency_conflict_no_mutation": True,
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_revision_producer_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    receipt = json.loads(path.read_text(encoding="utf-8"))
    validate_schema(receipt)
    summary = validate_semantics(receipt)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
