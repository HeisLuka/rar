#!/usr/bin/env python3
"""Validate one source-free Rescue recovery receipt and map it to a product outcome."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
from typing import Any

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "product" / "rescue-recovery" / "v1"
PRODUCER_SCHEMA = BASE / "producer-receipt.schema.json"
PRODUCT_SCHEMA = BASE / "product-validation.schema.json"

PINNED_FIXTURES = {
    "natural_partial_cfb": "6eb0a85afb42d329e2241ce060ec3b6957ecd429a5122cfc908b4621ab4884b8",
    "healthy_control": "3ab75a6a9196e0a51fc9b0aa759459501c71d313030c06652aabffbae0a2ab09",
    "partial_control": "2173c7dd2349c6ced2f838df458cc2503e15606d073cc8d6146706d372dd64d8",
}

ROUTE_CLASS = {
    "bounded_recovered": {"bounded_exact_salvage"},
    "partially_recovered": {"partial_salvage"},
    "manual_review": {"manual_review", "partial_salvage"},
    "unsupported_no_safe_recovery": {"unsupported"},
    "diagnostic_only": {"diagnostic_only"},
}

PRODUCT_OUTCOME = {
    "bounded_recovered": "bounded_recovered",
    "partially_recovered": "partially_recovered",
    "manual_review": "manual_review",
    "unsupported_no_safe_recovery": "unsupported/no_safe_recovery",
    "diagnostic_only": "unsupported/no_safe_recovery",
}


def _schema(path: pathlib.Path) -> dict[str, Any]:
    schema = json.loads(path.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return schema


def _validate_schema(receipt: Any, path: pathlib.Path, label: str) -> None:
    errors = sorted(
        Draft202012Validator(_schema(path)).iter_errors(receipt),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(f"{label} schema validation failed\n{detail}")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def validate_producer_receipt(receipt: dict[str, Any]) -> dict[str, Any]:
    _validate_schema(receipt, PRODUCER_SCHEMA, "Rescue producer receipt")

    fixture = receipt["fixture"]
    kind = fixture["kind"]
    source_sha = fixture["source_sha256"]
    _require(
        source_sha == PINNED_FIXTURES[kind],
        f"{kind}: source_sha256 does not match the pinned acceptance witness",
    )

    immutability = receipt["source_immutability"]
    _require(immutability["unchanged"] is True, "source must be explicitly unchanged")
    _require(
        immutability["before_sha256"] == source_sha
        and immutability["after_sha256"] == source_sha,
        "source before/after hashes must equal the pinned fixture hash",
    )

    recovery = receipt["recovery"]
    _require(recovery["fabricated_bytes"] == 0, "fabricated_bytes must be zero")
    _require(recovery["silent_drops"] == 0, "silent_drops must be zero")

    route = receipt["proposed_route"]
    recovery_class = recovery["recovery_class"]
    _require(
        recovery_class in ROUTE_CLASS[route],
        f"recovery_class {recovery_class!r} is incompatible with route {route!r}",
    )

    artifacts = receipt["artifacts"]
    artifact_ids = [artifact["artifact_id"] for artifact in artifacts]
    _require(len(artifact_ids) == len(set(artifact_ids)), "artifact_id values must be unique")

    if route in {"bounded_recovered", "partially_recovered"}:
        _require(artifacts, f"{route} requires at least one recovered artifact")
        for artifact in artifacts:
            _require(artifact["exact"] is True, "current P0 recovered artifacts must be byte-verified exact")
            if artifact["kind"] != "native_pub":
                _require(
                    artifact["source_ranges"],
                    f"{artifact['artifact_id']}: recovered artifact must carry at least one verified source range",
                )

    if route == "partially_recovered":
        _require(receipt["loss"]["known_loss"] is True, "partial recovery must declare known loss")
        _require(receipt["loss"]["items"], "partial recovery must enumerate at least one loss item")

    native_state = receipt["native_validation"]["state"]
    native_artifacts = [artifact for artifact in artifacts if artifact["kind"] == "native_pub"]
    if native_state == "valid":
        _require(
            "receipt_sha256" in receipt["native_validation"],
            "valid native validation requires a separate native receipt hash",
        )
    if native_artifacts:
        _require(
            native_state == "valid",
            "native PUB artifact is forbidden unless separate native validation is valid",
        )

    if kind == "healthy_control":
        _require(route == "diagnostic_only", "healthy control must remain diagnostic_only")
        _require(recovery_class == "diagnostic_only", "healthy control must not be labeled recovered")
        _require(not artifacts, "healthy control must not emit recovered artifacts")

    if kind == "partial_control":
        _require(
            route in {"partially_recovered", "manual_review"},
            "partial control must remain partial/manual and cannot be promoted to complete",
        )

    if kind == "natural_partial_cfb":
        _require(
            route != "diagnostic_only",
            "natural damaged witness must resolve to recovered/partial/manual/unsupported, not diagnostic_only",
        )

    return {
        "fixture_kind": kind,
        "source_sha256": source_sha,
        "producer_route": route,
        "product_outcome": PRODUCT_OUTCOME[route],
        "artifact_count": len(artifacts),
        "known_loss": receipt["loss"]["known_loss"],
        "native_pub_delivery_allowed": bool(native_artifacts) and native_state == "valid",
    }


def build_product_validation(
    producer_receipt: dict[str, Any],
    *,
    producer_receipt_sha256: str,
) -> dict[str, Any]:
    summary = validate_producer_receipt(producer_receipt)
    result = {
        "receipt_version": "chaptera.rescue-product-validation.v1",
        "product": "Chaptera Rescue",
        "producer_receipt_sha256": producer_receipt_sha256,
        "fixture": {
            "kind": summary["fixture_kind"],
            "source_sha256": summary["source_sha256"],
        },
        "outcome": summary["product_outcome"],
        "source_immutable": True,
        "artifact_count": summary["artifact_count"],
        "known_loss": summary["known_loss"],
        "native_pub_delivery_allowed": summary["native_pub_delivery_allowed"],
        "fail_closed": True,
        "privacy": {
            "source_free": True,
            "private_content_serialized": False,
        },
    }
    _validate_schema(result, PRODUCT_SCHEMA, "Rescue product validation receipt")
    return result


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("producer_receipt", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()

    producer = json.loads(args.producer_receipt.read_text(encoding="utf-8"))
    product = build_product_validation(
        producer,
        producer_receipt_sha256=sha256_file(args.producer_receipt),
    )
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(product, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(json.dumps(product, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
