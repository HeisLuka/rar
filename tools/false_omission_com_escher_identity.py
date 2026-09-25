#!/usr/bin/env python3
"""Candidate COM Shape.ID <-> Publisher Escher ClientData.ShapeId audit.

Inputs:
  - FALSE-OMISSION-01 runtime oracle/false-omission.json
  - pub-cli escher FILE.pub --json

The tool does not claim an identity edge from numeric equality alone.
It reports whether the one-shape runtime contract and a unique Escher
ClientData field 0x6801 match line up, and preserves ClientAnchor fields
as independent structural evidence.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

PUBLISHER_FIELD_XS = 0x2001
PUBLISHER_FIELD_YS = 0x2002
PUBLISHER_FIELD_XE = 0x2003
PUBLISHER_FIELD_YE = 0x2004
PUBLISHER_FIELD_SHAPE_ID = 0x6801


def safe_value(node: dict, name: str):
    value = node.get(name)
    if not isinstance(value, dict) or value.get("state") != "value":
        return None
    return value.get("value")


def field_values(record: dict | None, field_id: int) -> list[int]:
    if not record:
        return []
    return [
        int(field["value"])
        for field in record.get("fields", [])
        if int(field.get("id", -1)) == field_id
    ]


def escher_rows(escher: dict) -> list[dict]:
    rows = []
    for index, shape in enumerate(escher.get("shapes", [])):
        client_data = shape.get("client_data")
        anchor = shape.get("client_anchor")
        fsp = shape.get("fsp")
        rows.append({
            "shape_index": index,
            "client_data_shape_ids": field_values(client_data, PUBLISHER_FIELD_SHAPE_ID),
            "anchor_xs": field_values(anchor, PUBLISHER_FIELD_XS),
            "anchor_ys": field_values(anchor, PUBLISHER_FIELD_YS),
            "anchor_xe": field_values(anchor, PUBLISHER_FIELD_XE),
            "anchor_ye": field_values(anchor, PUBLISHER_FIELD_YE),
            "spid": None if fsp is None else int(fsp["spid"]),
            "shape_type": None if fsp is None else int(fsp["shape_type"]),
            "source": shape.get("source"),
        })
    return rows


def analyze(runtime: dict, escher: dict) -> dict:
    before = runtime.get("before") or {}
    contract = ((runtime.get("identity_contract") or {}).get("source") or {})
    com_shape_id = safe_value(before, "shape_id")
    geometry = {
        "left": safe_value(before, "left"),
        "top": safe_value(before, "top"),
        "width": safe_value(before, "width"),
        "height": safe_value(before, "height"),
    }

    rows = escher_rows(escher)
    matches = []
    if com_shape_id is not None:
        for row in rows:
            if int(com_shape_id) in row["client_data_shape_ids"]:
                matches.append(row)

    one_shape_contract = (
        contract.get("pages_count") == 1
        and contract.get("target_page_shapes_count") == 1
        and contract.get("unique_top_level_target") is True
        and contract.get("target_page_index") == 1
        and contract.get("target_shape_index") == 1
    )

    unique_numeric_match = com_shape_id is not None and len(matches) == 1
    candidate_bridge = one_shape_contract and unique_numeric_match

    return {
        "schema": "pub-false-omission-01/com-escher-identity-audit/v1",
        "runtime": {
            "case_id": runtime.get("case_id"),
            "source_sha256": runtime.get("source_sha256"),
            "com_shape_id": com_shape_id,
            "geometry_points": geometry,
            "one_shape_contract": one_shape_contract,
        },
        "escher": {
            "spcontainer_count": len(rows),
            "rows": rows,
        },
        "match": {
            "matching_spcontainers": len(matches),
            "unique_numeric_match": unique_numeric_match,
            "candidate_bridge": candidate_bridge,
            "matches": matches,
        },
        "guardrails": [
            "candidate_bridge is not a universal Shape.ID mapping.",
            "The result is scoped to one exact runtime output and its Escher inventory.",
            "ClientData.ShapeId and FSP.spid remain separate namespaces.",
            "ClientAnchor values are preserved without assuming point/EMU conversion.",
            "A production IdentityGraph edge still needs explicit version scope and provenance.",
        ],
    }


def synthetic_runtime(shape_id: int) -> dict:
    def v(name: str, value):
        return {"state": "value", "member": name, "value": value}
    return {
        "case_id": "synthetic",
        "source_sha256": "0" * 64,
        "identity_contract": {
            "source": {
                "pages_count": 1,
                "target_page_index": 1,
                "target_shape_index": 1,
                "target_page_shapes_count": 1,
                "unique_top_level_target": True,
            }
        },
        "before": {
            "shape_id": v("Shape.ID", shape_id),
            "left": v("Shape.Left", 73),
            "top": v("Shape.Top", 91),
            "width": v("Shape.Width", 181),
            "height": v("Shape.Height", 103),
        },
    }


def synthetic_escher(shape_id: int) -> dict:
    return {
        "shapes": [
            {
                "fsp": {"spid": 1001, "shape_type": 1},
                "client_data": {
                    "fields": [{"id": PUBLISHER_FIELD_SHAPE_ID, "value": shape_id}]
                },
                "client_anchor": {
                    "fields": [
                        {"id": PUBLISHER_FIELD_XS, "value": 73000},
                        {"id": PUBLISHER_FIELD_YS, "value": 91000},
                        {"id": PUBLISHER_FIELD_XE, "value": 254000},
                        {"id": PUBLISHER_FIELD_YE, "value": 194000},
                    ]
                },
                "source": {"stream": "/Escher/EscherStm", "offset": 100, "len": 80},
            }
        ]
    }


def self_test() -> None:
    good = analyze(synthetic_runtime(42), synthetic_escher(42))
    assert good["match"]["candidate_bridge"] is True
    assert good["match"]["matching_spcontainers"] == 1
    assert good["match"]["matches"][0]["spid"] == 1001

    bad = analyze(synthetic_runtime(42), synthetic_escher(77))
    assert bad["match"]["candidate_bridge"] is False
    assert bad["match"]["matching_spcontainers"] == 0

    duplicate = synthetic_escher(42)
    duplicate["shapes"].append(duplicate["shapes"][0].copy())
    amb = analyze(synthetic_runtime(42), duplicate)
    assert amb["match"]["candidate_bridge"] is False
    assert amb["match"]["matching_spcontainers"] == 2
    print("FALSE_OMISSION_COM_ESCHER_IDENTITY_SELF_TEST=OK")


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--runtime-json", type=Path)
    p.add_argument("--escher-json", type=Path)
    p.add_argument("--out-json", type=Path)
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args()

    if args.self_test:
        self_test()
        return 0

    if not args.runtime_json or not args.escher_json or not args.out_json:
        p.error("--runtime-json, --escher-json and --out-json are required")

    runtime = json.loads(args.runtime_json.read_text(encoding="utf-8"))
    escher = json.loads(args.escher_json.read_text(encoding="utf-8"))
    report = analyze(runtime, escher)
    args.out_json.write_text(
        json.dumps(report, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({
        "com_shape_id": report["runtime"]["com_shape_id"],
        "one_shape_contract": report["runtime"]["one_shape_contract"],
        "spcontainers": report["escher"]["spcontainer_count"],
        "matching_spcontainers": report["match"]["matching_spcontainers"],
        "candidate_bridge": report["match"]["candidate_bridge"],
    }, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
