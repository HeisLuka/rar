#!/usr/bin/env python3
"""Validate the canonical Chaptera desktop-suite product/executable manifest."""

from __future__ import annotations

import argparse
import json
import pathlib
import tomllib
from typing import Any

EXPECTED_PRODUCTS = {
    "chaptera.reader": "chaptera-reader.exe",
    "chaptera.rescue": "chaptera-rescue.exe",
    "chaptera.editor": "chaptera-editor.exe",
    "chaptera.migration": "chaptera-migration.exe",
}
GENERIC_LEGACY_BINARY = "chaptera.exe"
EXPECTED_IMPLEMENTATION_STATE = {
    "chaptera.reader": "current_rar_target",
    "chaptera.rescue": "current_rar_target",
    "chaptera.editor": "current_rar_target",
    "chaptera.migration": "separate_target_pending",
}


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def validate_manifest(value: Any) -> dict[str, Any]:
    _require(isinstance(value, dict), "manifest must be a JSON object")
    _require(value.get("schema_version") == "chaptera.desktop-suite-products.v1", "unexpected desktop-suite manifest schema_version")
    products = value.get("products")
    _require(isinstance(products, list), "products must be an array")
    by_id: dict[str, dict[str, Any]] = {}
    executables: set[str] = set()
    packages: set[str] = set()

    for product in products:
        _require(isinstance(product, dict), "each product must be an object")
        product_id = product.get("product_id")
        _require(isinstance(product_id, str) and product_id, "product_id is required")
        _require(product_id not in by_id, f"duplicate product_id: {product_id}")
        executable = product.get("canonical_windows_executable")
        package = product.get("canonical_windows_package")
        capabilities = product.get("capability_surface")
        implementation_state = product.get("implementation_state")
        _require(isinstance(executable, str), f"{product_id}: executable is required")
        _require(executable == executable.lower(), f"{product_id}: executable must be lowercase")
        _require(executable.startswith("chaptera-") and executable.endswith(".exe"), f"{product_id}: executable must be product-qualified")
        _require(executable != GENERIC_LEGACY_BINARY, f"{product_id}: generic chaptera.exe cannot be canonical")
        _require(executable not in executables, f"duplicate canonical executable: {executable}")
        _require(isinstance(package, str), f"{product_id}: package is required")
        _require(package.startswith("Chaptera-") and package.endswith(".zip"), f"{product_id}: package must be product-qualified")
        _require(package not in packages, f"duplicate canonical package: {package}")
        _require(isinstance(capabilities, list) and capabilities, f"{product_id}: capability_surface must be non-empty")
        _require(all(isinstance(item, str) and item for item in capabilities), f"{product_id}: invalid capability_surface")
        _require(len(capabilities) == len(set(capabilities)), f"{product_id}: duplicate capability_surface entries")
        _require(
            implementation_state in {"current_rar_target", "separate_target_pending"},
            f"{product_id}: invalid implementation_state",
        )
        executables.add(executable)
        packages.add(package)
        by_id[product_id] = product

    _require(set(by_id) == set(EXPECTED_PRODUCTS), "manifest must contain exactly Reader, Rescue, Editor and Migration")
    for product_id, executable in EXPECTED_PRODUCTS.items():
        _require(by_id[product_id]["canonical_windows_executable"] == executable, f"{product_id}: unexpected canonical executable")
        _require(
            by_id[product_id]["implementation_state"] == EXPECTED_IMPLEMENTATION_STATE[product_id],
            f"{product_id}: stale implementation_state",
        )

    aliases = value.get("legacy_binary_aliases")
    _require(isinstance(aliases, list), "legacy_binary_aliases must be an array")
    generic_aliases = [item for item in aliases if isinstance(item, dict) and item.get("name") == GENERIC_LEGACY_BINARY]
    _require(len(generic_aliases) == 1, "chaptera.exe must have exactly one explicit legacy alias record")
    alias = generic_aliases[0]
    _require(alias.get("product_id") == "chaptera.editor", "chaptera.exe legacy identity must belong only to Editor")
    _require(alias.get("policy") == "historical_receipts_only_no_new_builds", "chaptera.exe legacy alias policy must forbid new builds")

    return {
        "schema_version": value["schema_version"],
        "product_ids": sorted(by_id),
        "canonical_executables": sorted(executables),
        "canonical_packages": sorted(packages),
        "legacy_chaptera_exe_owner": "chaptera.editor",
        "implementation_state": {
            product_id: by_id[product_id]["implementation_state"]
            for product_id in sorted(by_id)
        },
    }


def validate_editor_cargo(path: pathlib.Path, manifest: dict[str, Any]) -> None:
    cargo = tomllib.loads(path.read_text(encoding="utf-8"))
    bins = cargo.get("bin", [])
    names = [item.get("name") for item in bins if isinstance(item, dict)]
    expected_exe = next(item["canonical_windows_executable"] for item in manifest["products"] if item["product_id"] == "chaptera.editor")
    expected_bin = expected_exe.removesuffix(".exe")
    _require(names == [expected_bin], f"Editor Cargo bin must be exactly {expected_bin!r}; got {names!r}")


def validate_editor_workflow(path: pathlib.Path) -> None:
    workflow = path.read_text(encoding="utf-8")
    _require(
        "Chaptera-Editor.exe" in workflow,
        "Editor Windows workflow must package Chaptera-Editor.exe",
    )
    _require(
        "Chaptera.exe" not in workflow,
        "Editor Windows workflow must not ship the generic Chaptera.exe entry",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=pathlib.Path)
    parser.add_argument("--editor-cargo", type=pathlib.Path)
    parser.add_argument("--editor-workflow", type=pathlib.Path)
    args = parser.parse_args()
    value = json.loads(args.manifest.read_text(encoding="utf-8"))
    summary = validate_manifest(value)
    if args.editor_cargo is not None:
        validate_editor_cargo(args.editor_cargo, value)
        summary["editor_cargo_bound"] = True
    if args.editor_workflow is not None:
        validate_editor_workflow(args.editor_workflow)
        summary["editor_workflow_bound"] = True
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
