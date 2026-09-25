#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CORPUS_TOOLS = ROOT / "tools" / "corpus"
if str(CORPUS_TOOLS) not in sys.path:
    sys.path.insert(0, str(CORPUS_TOOLS))

from cfb_physical_diff import CFB  # noqa: E402

SCHEMA = "chaptera.operation-blast-radius.v1"
CLASSIFICATIONS = {
    "requested_semantic",
    "save_normalization",
    "expected_derived",
    "unexplained_collateral",
    "unavailable",
}


class BlastRadiusError(RuntimeError):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def artifact_identity(data: bytes, producer: dict[str, Any] | None = None) -> dict[str, Any]:
    out = {"sha256": sha256(data), "byte_len": len(data)}
    if producer:
        out["producer"] = producer
    return out


def stream_bytes(cfb: CFB, entry: dict[str, Any]) -> bytes:
    size = entry["size"]
    if size == 0:
        return b""
    chunks: list[bytes] = []
    if size >= cfb.cut:
        for sector in cfb.schain(entry):
            chunks.append(cfb.sec(sector))
    else:
        for mini_sector in cfb.schain(entry):
            offset = cfb.rawmini(mini_sector * cfb.ms)
            chunks.append(cfb.b[offset : offset + cfb.ms])
    payload = b"".join(chunks)
    if len(payload) < size:
        raise BlastRadiusError(f"stream {entry['i']} shorter than directory size")
    return payload[:size]


def stream_inventory(data: bytes) -> tuple[CFB, dict[str, dict[str, Any]]]:
    cfb = CFB(data)
    streams: dict[str, dict[str, Any]] = {}
    for entry in cfb.dirs:
        if entry["type"] != 2:
            continue
        payload = stream_bytes(cfb, entry)
        stream_id = f"dir:{entry['i']}:{entry['name']}"
        streams[stream_id] = {
            "stream_id": stream_id,
            "directory_index": entry["i"],
            "name": entry["name"],
            "storage": "minifat" if entry["size"] < cfb.cut else "fat",
            "size": entry["size"],
            "sha256": sha256(payload),
        }
    return cfb, dict(sorted(streams.items()))


def changed_ranges(left: bytes, right: bytes, left_cfb: CFB, right_cfb: CFB) -> list[dict[str, Any]]:
    limit = min(len(left), len(right))
    offsets = [i for i in range(limit) if left[i] != right[i]]
    if len(left) != len(right):
        offsets.extend(range(limit, max(len(left), len(right))))
    if not offsets:
        return []

    def label_at(offset: int) -> str:
        l = left_cfb.lab[offset] if offset < len(left_cfb.lab) else "eof"
        r = right_cfb.lab[offset] if offset < len(right_cfb.lab) else "eof"
        return l if l == r else f"{l}->{r}"

    out: list[dict[str, Any]] = []
    start = previous = offsets[0]
    label = label_at(start)
    for offset in offsets[1:]:
        current = label_at(offset)
        if offset == previous + 1 and current == label:
            previous = offset
            continue
        out.append({"offset": start, "length": previous - start + 1, "physical_label": label})
        start = previous = offset
        label = current
    out.append({"offset": start, "length": previous - start + 1, "physical_label": label})
    return out


def topology_delta(left: dict[str, dict[str, Any]], right: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for stream_id in sorted(set(left) | set(right)):
        a = left.get(stream_id)
        b = right.get(stream_id)
        if a is None:
            out.append({"stream_id": stream_id, "change": "created", "before": None, "after": b})
        elif b is None:
            out.append({"stream_id": stream_id, "change": "deleted", "before": a, "after": None})
        elif a["size"] != b["size"] or a["storage"] != b["storage"]:
            out.append({
                "stream_id": stream_id,
                "change": "metadata_changed",
                "before": {"size": a["size"], "storage": a["storage"]},
                "after": {"size": b["size"], "storage": b["storage"]},
            })
    return out


def stream_delta(left: dict[str, dict[str, Any]], right: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for stream_id in sorted(set(left) | set(right)):
        a = left.get(stream_id)
        b = right.get(stream_id)
        if a == b:
            continue
        out.append({
            "stream_id": stream_id,
            "before_sha256": None if a is None else a["sha256"],
            "after_sha256": None if b is None else b["sha256"],
            "before_size": None if a is None else a["size"],
            "after_size": None if b is None else b["size"],
        })
    return out


def pair_index(items: Any, key_fields: tuple[str, ...], label: str) -> dict[tuple[Any, ...], dict[str, Any]]:
    if items is None:
        return {}
    if not isinstance(items, list):
        raise BlastRadiusError(f"{label} must be an array")
    out: dict[tuple[Any, ...], dict[str, Any]] = {}
    for index, item in enumerate(items):
        if not isinstance(item, dict):
            raise BlastRadiusError(f"{label}[{index}] must be an object")
        try:
            key = tuple(item[field] for field in key_fields)
            digest = item["sha256"]
        except KeyError as error:
            raise BlastRadiusError(f"{label}[{index}] missing {error.args[0]}") from error
        if not isinstance(digest, str) or len(digest) != 64:
            raise BlastRadiusError(f"{label}[{index}].sha256 invalid")
        if key in out:
            raise BlastRadiusError(f"{label} duplicate key {key}")
        out[key] = item
    return out


def structured_delta(
    left: Any,
    right: Any,
    *,
    key_fields: tuple[str, ...],
    label: str,
    requested: set[tuple[Any, ...]],
    expected: set[tuple[Any, ...]],
) -> list[dict[str, Any]]:
    a = pair_index(left, key_fields, f"{label}.left")
    b = pair_index(right, key_fields, f"{label}.right")
    out = []
    for key in sorted(set(a) | set(b)):
        if a.get(key) == b.get(key):
            continue
        classification = (
            "requested_semantic"
            if key in requested
            else "expected_derived"
            if key in expected
            else "unexplained_collateral"
        )
        item = {field: value for field, value in zip(key_fields, key)}
        item.update({
            "before_sha256": None if key not in a else a[key]["sha256"],
            "after_sha256": None if key not in b else b[key]["sha256"],
            "classification": classification,
        })
        out.append(item)
    return out


def classify_stream(stream_id: str, evidence: dict[str, Any], phase: str) -> str:
    if phase == "source_control":
        return "save_normalization"
    if stream_id in set(evidence.get("requested_streams", [])):
        return "requested_semantic"
    if stream_id in set(evidence.get("expected_derived_streams", [])):
        return "expected_derived"
    return "unexplained_collateral"


def classify_physical_label(label: str, evidence: dict[str, Any]) -> str:
    for stream_id in evidence.get("requested_streams", []):
        name = stream_id.split(":", 2)[-1]
        if f"stream_payload:{name}" in label or f"mini_stream_payload:{name}" in label:
            return "requested_semantic"
    for stream_id in evidence.get("expected_derived_streams", []):
        name = stream_id.split(":", 2)[-1]
        if f"stream_payload:{name}" in label or f"mini_stream_payload:{name}" in label:
            return "expected_derived"
    if any(token in label for token in ("fat_sector", "difat_sector", "minifat_sector", "directory")):
        return "expected_derived"
    return "unexplained_collateral"


def arm_evidence(evidence: dict[str, Any], arm: str) -> dict[str, Any]:
    arms = evidence.get("arms", {})
    if not isinstance(arms, dict):
        raise BlastRadiusError("evidence.arms must be an object")
    value = arms.get(arm, {})
    if not isinstance(value, dict):
        raise BlastRadiusError(f"evidence.arms.{arm} must be an object")
    return value


def parser_outcome(value: dict[str, Any]) -> dict[str, Any]:
    parser = value.get("parser")
    if parser is None:
        return {"status": "unavailable", "diagnostic_codes": []}
    if not isinstance(parser, dict) or not isinstance(parser.get("accepted"), bool):
        raise BlastRadiusError("parser evidence must contain boolean accepted")
    codes = parser.get("diagnostic_codes", [])
    if not isinstance(codes, list) or not all(isinstance(x, str) for x in codes):
        raise BlastRadiusError("parser diagnostic_codes invalid")
    return {"status": "accepted" if parser["accepted"] else "rejected", "diagnostic_codes": sorted(set(codes))}


def build_receipt(
    source: bytes,
    control: bytes,
    mutation: bytes,
    *,
    evidence: dict[str, Any],
    second_save: bytes | None = None,
) -> dict[str, Any]:
    if not all(isinstance(x, bytes) for x in (source, control, mutation)):
        raise BlastRadiusError("artifact inputs must be bytes")
    source_cfb, source_streams = stream_inventory(source)
    control_cfb, control_streams = stream_inventory(control)
    mutation_cfb, mutation_streams = stream_inventory(mutation)

    producer = evidence.get("producer")
    if producer is not None and not isinstance(producer, dict):
        raise BlastRadiusError("evidence.producer must be an object")

    source_control_streams = stream_delta(source_streams, control_streams)
    control_mutation_streams = stream_delta(control_streams, mutation_streams)
    for item in source_control_streams:
        item["classification"] = "save_normalization"
    for item in control_mutation_streams:
        item["classification"] = classify_stream(item["stream_id"], evidence, "control_mutation")

    source_control_topology = topology_delta(source_streams, control_streams)
    control_mutation_topology = topology_delta(control_streams, mutation_streams)
    for item in source_control_topology:
        item["classification"] = "save_normalization"
    for item in control_mutation_topology:
        item["classification"] = classify_stream(item["stream_id"], evidence, "control_mutation")

    byte_ranges = changed_ranges(control, mutation, control_cfb, mutation_cfb)
    for item in byte_ranges:
        item["classification"] = classify_physical_label(item["physical_label"], evidence)

    requested_records = {
        (item["family"], item["id"])
        for item in evidence.get("requested_records", [])
        if isinstance(item, dict) and "family" in item and "id" in item
    }
    expected_records = {
        (item["family"], item["id"])
        for item in evidence.get("expected_derived_records", [])
        if isinstance(item, dict) and "family" in item and "id" in item
    }
    requested_entities = {
        (item["kind"], item["id"])
        for item in evidence.get("requested_entities", [])
        if isinstance(item, dict) and "kind" in item and "id" in item
    }
    expected_entities = {
        (item["kind"], item["id"])
        for item in evidence.get("expected_derived_entities", [])
        if isinstance(item, dict) and "kind" in item and "id" in item
    }

    c = arm_evidence(evidence, "control")
    m = arm_evidence(evidence, "mutation")
    record_deltas = structured_delta(
        c.get("records"),
        m.get("records"),
        key_fields=("family", "id"),
        label="records",
        requested=requested_records,
        expected=expected_records,
    )
    semantic_deltas = structured_delta(
        c.get("semantic_entities"),
        m.get("semantic_entities"),
        key_fields=("kind", "id"),
        label="semantic_entities",
        requested=requested_entities,
        expected=expected_entities,
    )

    if second_save is None:
        convergence = {"status": "unavailable"}
    else:
        second_cfb, second_streams = stream_inventory(second_save)
        second_delta = stream_delta(mutation_streams, second_streams)
        convergence = {
            "status": "converged" if sha256(second_save) == sha256(mutation) else "changed",
            "artifact": artifact_identity(second_save, producer),
            "changed_stream_count": len(second_delta),
            "different_byte_count": len([1 for a, b in zip(mutation, second_save) if a != b])
            + abs(len(mutation) - len(second_save)),
        }
        del second_cfb

    all_classified = (
        source_control_streams
        + control_mutation_streams
        + source_control_topology
        + control_mutation_topology
        + byte_ranges
        + record_deltas
        + semantic_deltas
    )
    counts = {key: 0 for key in sorted(CLASSIFICATIONS)}
    for item in all_classified:
        classification = item["classification"]
        if classification not in CLASSIFICATIONS:
            raise BlastRadiusError(f"unknown classification {classification}")
        counts[classification] += 1

    operation = evidence.get("operation", {"kind": "unknown"})
    if not isinstance(operation, dict) or not isinstance(operation.get("kind"), str):
        raise BlastRadiusError("evidence.operation.kind must be a string")

    return {
        "schema_version": SCHEMA,
        "operation": operation,
        "artifacts": {
            "source": artifact_identity(source, producer),
            "control": artifact_identity(control, producer),
            "mutation": artifact_identity(mutation, producer),
        },
        "cfb": {
            "source_control_topology_delta": source_control_topology,
            "control_mutation_topology_delta": control_mutation_topology,
            "source_control_stream_delta": source_control_streams,
            "control_mutation_stream_delta": control_mutation_streams,
            "control_mutation_byte_ranges": byte_ranges,
        },
        "parsed_record_family_delta": record_deltas,
        "semantic_graph_delta": semantic_deltas,
        "parser_outcomes": {
            "source": parser_outcome(arm_evidence(evidence, "source")),
            "control": parser_outcome(c),
            "mutation": parser_outcome(m),
        },
        "second_save_convergence": convergence,
        "classification_counts": counts,
        "invariants": {
            "raw_byte_inequality_is_not_semantic_evidence": True,
            "matched_noop_control_used": True,
            "unexplained_collateral_preserved": True,
            "public_receipt_contains_raw_document_bytes": False,
            "native_pub_writer_capability_granted": False,
        },
    }


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise BlastRadiusError("evidence JSON must be an object")
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description="Build OperationBlastRadiusV1 from source/control/mutation PUB artifacts")
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--control", required=True, type=Path)
    parser.add_argument("--mutation", required=True, type=Path)
    parser.add_argument("--second-save", type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    try:
        receipt = build_receipt(
            args.source.read_bytes(),
            args.control.read_bytes(),
            args.mutation.read_bytes(),
            evidence=load_json(args.evidence),
            second_save=args.second_save.read_bytes() if args.second_save else None,
        )
    except (OSError, ValueError, json.JSONDecodeError, BlastRadiusError) as error:
        print(f"operation-blast-radius: {error}", file=sys.stderr)
        return 2
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({
        "schema_version": receipt["schema_version"],
        "classification_counts": receipt["classification_counts"],
        "second_save_convergence": receipt["second_save_convergence"]["status"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
