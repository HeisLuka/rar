#!/usr/bin/env python3
"""Generic operation-reachable editor asset registry for EditorProject V1.

Durable truth is:
canonical operation log / durable refs -> editor asset identities -> metadata ->
exact byte resolver. Imported-but-unreferenced bytes are runtime state only.
"""

from __future__ import annotations

import copy
from dataclasses import asdict, dataclass
import hashlib
from typing import Any, Callable, Mapping

from image_asset_intrinsic_v1 import (
    ImageAssetIntrinsicError,
    derive_image_asset_intrinsic_v1,
)


PROJECT_SCHEMA_V1 = "chaptera.editor-project.assets.v1"
LEGACY_PROJECT_SCHEMA_V1 = "chaptera.editor-project.replacement-assets.v1"
REGISTRY_SCHEMA_V1 = "chaptera.editor-asset-registry.v1"
SUPPORTED_MIME = {"image/png", "image/jpeg"}

KNOWN_DIRECT_ASSET_OPERATION_KINDS = {
    "replace_image",
    "create_picture_frame",
}
EXPLICIT_NON_ASSET_OPERATION_KINDS = {
    "delete_node",
    "move_node",
    "move_nodes",
    "resize_node",
    "set_image_crop",
    "fit_group_to_contents",
}


class EditorProjectAssetRegistryError(ValueError):
    pass


@dataclass(frozen=True)
class EditorAssetIntrinsicV1:
    width_px: int
    height_px: int
    orientation_class: str
    exif_orientation: int | None


@dataclass(frozen=True)
class EditorAssetMetadataV1:
    asset_sha256: str
    mime_type: str
    byte_len: int
    intrinsic: EditorAssetIntrinsicV1 | None = None

    def public_dict(self) -> dict:
        result = asdict(self)
        if self.intrinsic is None:
            result["intrinsic"] = None
        return result


def _sha(value: str, label: str = "asset_sha256") -> str:
    if (
        not isinstance(value, str)
        or len(value) != 64
        or any(ch not in "0123456789abcdef" for ch in value)
    ):
        raise EditorProjectAssetRegistryError(f"{label} must be lowercase SHA-256")
    return value


def _metadata_from_dict(value: dict) -> EditorAssetMetadataV1:
    if not isinstance(value, dict):
        raise EditorProjectAssetRegistryError("asset metadata entry must be an object")
    sha = _sha(value.get("asset_sha256"))
    mime = value.get("mime_type")
    if mime not in SUPPORTED_MIME:
        raise EditorProjectAssetRegistryError("asset metadata MIME is unsupported")
    byte_len = value.get("byte_len")
    if not isinstance(byte_len, int) or isinstance(byte_len, bool) or byte_len <= 0:
        raise EditorProjectAssetRegistryError("asset metadata byte_len must be positive")
    intrinsic_raw = value.get("intrinsic")
    intrinsic = None
    if intrinsic_raw is not None:
        if not isinstance(intrinsic_raw, dict):
            raise EditorProjectAssetRegistryError("intrinsic metadata must be an object or null")
        required = {"width_px", "height_px", "orientation_class", "exif_orientation"}
        if set(intrinsic_raw) != required:
            raise EditorProjectAssetRegistryError("intrinsic metadata fields are not V1")
        intrinsic = EditorAssetIntrinsicV1(
            width_px=intrinsic_raw["width_px"],
            height_px=intrinsic_raw["height_px"],
            orientation_class=intrinsic_raw["orientation_class"],
            exif_orientation=intrinsic_raw["exif_orientation"],
        )
    return EditorAssetMetadataV1(sha, mime, byte_len, intrinsic)


def derive_asset_metadata_v1(
    *,
    asset_bytes: bytes,
    mime_type: str,
    include_intrinsic: bool = True,
) -> EditorAssetMetadataV1:
    if not isinstance(asset_bytes, bytes) or not asset_bytes:
        raise EditorProjectAssetRegistryError("asset bytes must be non-empty exact bytes")
    if mime_type not in SUPPORTED_MIME:
        raise EditorProjectAssetRegistryError("asset MIME is unsupported")
    sha = hashlib.sha256(asset_bytes).hexdigest()
    intrinsic = None
    if include_intrinsic:
        try:
            facts = derive_image_asset_intrinsic_v1(
                asset_bytes=asset_bytes,
                mime_type=mime_type,
                expected_sha256=sha,
            )
        except ImageAssetIntrinsicError as exc:
            raise EditorProjectAssetRegistryError(str(exc)) from exc
        intrinsic = EditorAssetIntrinsicV1(
            facts.width_px,
            facts.height_px,
            facts.orientation_class,
            facts.exif_orientation,
        )
    return EditorAssetMetadataV1(sha, mime_type, len(asset_bytes), intrinsic)


def validate_asset_bytes_v1(
    metadata: EditorAssetMetadataV1,
    asset_bytes: bytes,
) -> None:
    if not isinstance(metadata, EditorAssetMetadataV1):
        raise EditorProjectAssetRegistryError("EditorAssetMetadataV1 is required")
    if not isinstance(asset_bytes, bytes):
        raise EditorProjectAssetRegistryError("resolved asset payload must be bytes")
    if len(asset_bytes) != metadata.byte_len:
        raise EditorProjectAssetRegistryError("resolved asset byte_len mismatch")
    if hashlib.sha256(asset_bytes).hexdigest() != metadata.asset_sha256:
        raise EditorProjectAssetRegistryError("resolved asset SHA-256 mismatch")
    try:
        facts = derive_image_asset_intrinsic_v1(
            asset_bytes=asset_bytes,
            mime_type=metadata.mime_type,
            expected_sha256=metadata.asset_sha256,
        )
    except ImageAssetIntrinsicError as exc:
        raise EditorProjectAssetRegistryError(str(exc)) from exc
    if metadata.intrinsic is not None:
        expected = metadata.intrinsic
        actual = EditorAssetIntrinsicV1(
            facts.width_px,
            facts.height_px,
            facts.orientation_class,
            facts.exif_orientation,
        )
        if actual != expected:
            raise EditorProjectAssetRegistryError("persisted intrinsic metadata mismatch")


def _walk_for_unaccounted_asset_keys(value: Any, *, at: str = "$") -> list[str]:
    found = []
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {"asset_sha256", "editor_asset_sha256", "editor_asset_refs"}:
                found.append(f"{at}.{key}")
            found.extend(_walk_for_unaccounted_asset_keys(child, at=f"{at}.{key}"))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found.extend(_walk_for_unaccounted_asset_keys(child, at=f"{at}[{index}]"))
    return found


def operation_editor_asset_refs_v1(operation: dict) -> tuple[str, ...]:
    if not isinstance(operation, dict):
        raise EditorProjectAssetRegistryError("canonical operation must be an object")
    kind = operation.get("kind")
    if not isinstance(kind, str) or not kind:
        raise EditorProjectAssetRegistryError("canonical operation kind is required")

    refs: list[str] = []
    accounted_paths = 0

    if kind in KNOWN_DIRECT_ASSET_OPERATION_KINDS:
        refs.append(_sha(operation.get("asset_sha256"), f"{kind}.asset_sha256"))
        accounted_paths += 1

    explicit = operation.get("editor_asset_refs")
    if explicit is not None:
        if not isinstance(explicit, list):
            raise EditorProjectAssetRegistryError("editor_asset_refs must be a list")
        for index, value in enumerate(explicit):
            refs.append(_sha(value, f"editor_asset_refs[{index}]"))
        accounted_paths += 1

    all_asset_paths = _walk_for_unaccounted_asset_keys(operation)
    # Direct known field and explicit refs are the only V1 conventions. Any
    # additional asset-looking field must fail closed rather than disappear
    # from project reachability.
    expected_paths = set()
    if kind in KNOWN_DIRECT_ASSET_OPERATION_KINDS:
        expected_paths.add("$.asset_sha256")
    if explicit is not None:
        expected_paths.add("$.editor_asset_refs")
    unaccounted = [path for path in all_asset_paths if path not in expected_paths]
    if unaccounted:
        raise EditorProjectAssetRegistryError(
            "canonical operation contains unaccounted editor asset reference field"
        )

    if kind not in KNOWN_DIRECT_ASSET_OPERATION_KINDS and explicit is None:
        # Unknown non-asset operations are safe only because the recursive fence
        # above proves they contain none of the V1 asset-reference keys.
        return ()

    return tuple(sorted(set(refs)))


def required_editor_asset_ids_v1(project: dict) -> tuple[str, ...]:
    if not isinstance(project, dict):
        raise EditorProjectAssetRegistryError("EditorProject must be an object")
    operations = project.get("operations")
    if not isinstance(operations, list):
        raise EditorProjectAssetRegistryError("EditorProject operations must be a list")
    refs: set[str] = set()
    for operation in operations:
        refs.update(operation_editor_asset_refs_v1(operation))
    durable = project.get("durable_editor_asset_refs", [])
    if not isinstance(durable, list):
        raise EditorProjectAssetRegistryError("durable_editor_asset_refs must be a list")
    for index, value in enumerate(durable):
        refs.add(_sha(value, f"durable_editor_asset_refs[{index}]"))
    return tuple(sorted(refs))


def _registry_map(entries: list[dict], label: str) -> dict[str, EditorAssetMetadataV1]:
    if not isinstance(entries, list):
        raise EditorProjectAssetRegistryError(f"{label} must be a list")
    result: dict[str, EditorAssetMetadataV1] = {}
    for raw in entries:
        meta = _metadata_from_dict(raw)
        if meta.asset_sha256 in result:
            raise EditorProjectAssetRegistryError(f"{label} contains duplicate asset identity")
        result[meta.asset_sha256] = meta
    return result


def normalize_project_asset_registry_v1(project: dict) -> dict:
    """Read current or one explicit legacy schema without changing operations."""
    if not isinstance(project, dict):
        raise EditorProjectAssetRegistryError("EditorProject must be an object")
    schema = project.get("schema_version")
    out = copy.deepcopy(project)
    if schema == PROJECT_SCHEMA_V1:
        if "replacement_assets" in out:
            raise EditorProjectAssetRegistryError("current schema must not contain legacy replacement_assets")
        _registry_map(out.get("editor_assets", []), "editor_assets")
        return out
    if schema == LEGACY_PROJECT_SCHEMA_V1:
        if "editor_assets" in out:
            raise EditorProjectAssetRegistryError("legacy schema cannot already contain editor_assets")
        legacy = out.pop("replacement_assets", None)
        _registry_map(legacy, "replacement_assets")
        out["schema_version"] = PROJECT_SCHEMA_V1
        out["editor_assets"] = copy.deepcopy(legacy)
        return out
    raise EditorProjectAssetRegistryError("unsupported EditorProject asset-registry schema")


def serialize_project_asset_registry_v1(
    project: dict,
    available_metadata: Mapping[str, EditorAssetMetadataV1],
) -> dict:
    """Create the durable registry from operation-log/durable-reference reachability."""
    normalized = normalize_project_asset_registry_v1(project)
    required = required_editor_asset_ids_v1(normalized)
    entries = []
    for sha in required:
        meta = available_metadata.get(sha)
        if not isinstance(meta, EditorAssetMetadataV1):
            raise EditorProjectAssetRegistryError("required editor asset metadata is missing")
        if meta.asset_sha256 != sha:
            raise EditorProjectAssetRegistryError("asset metadata map key/identity mismatch")
        entries.append(meta.public_dict())
    out = copy.deepcopy(normalized)
    out["editor_asset_registry_schema"] = REGISTRY_SCHEMA_V1
    out["editor_assets"] = entries
    # Runtime/import cache is explicitly not durable project truth.
    out.pop("imported_editor_assets", None)
    return out


def apply_project_with_assets_v1(
    project: dict,
    *,
    asset_bytes_by_sha: Mapping[str, bytes],
    fresh_session: Any,
    apply_operation: Callable[[Any, dict, Mapping[str, bytes]], Any],
) -> Any:
    """Validate the complete required byte set before replaying any operation."""
    normalized = normalize_project_asset_registry_v1(project)
    if normalized.get("editor_asset_registry_schema") not in {None, REGISTRY_SCHEMA_V1}:
        raise EditorProjectAssetRegistryError("unsupported editor asset registry version")
    required = required_editor_asset_ids_v1(normalized)
    metadata = _registry_map(normalized.get("editor_assets", []), "editor_assets")

    validated: dict[str, bytes] = {}
    for sha in required:
        meta = metadata.get(sha)
        if meta is None:
            raise EditorProjectAssetRegistryError("required editor asset metadata entry is missing")
        payload = asset_bytes_by_sha.get(sha)
        if payload is None:
            raise EditorProjectAssetRegistryError("required editor asset bytes are missing")
        validate_asset_bytes_v1(meta, payload)
        validated[sha] = payload

    # Transactional replay: caller's fresh_session is never mutated directly.
    candidate = copy.deepcopy(fresh_session)
    try:
        for operation in normalized["operations"]:
            candidate = apply_operation(candidate, copy.deepcopy(operation), validated)
    except Exception as exc:
        raise EditorProjectAssetRegistryError(f"transactional project replay failed: {exc}") from exc
    return candidate
