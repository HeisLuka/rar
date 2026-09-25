#!/usr/bin/env python3
"""Local/native Chaptera fixed-page trial acceptance runner.

This runner closes the orchestration gap between already-validated upstream
capability receipts and one continuous fixed-page product trial. It does not
invent document semantics. The authorized local producer must execute the real
Windows Editor path and:
- consume the bound PUB plus runner-generated replacement PNG;
- write the final EditorProject to {project};
- write one edited IDML/ODG package to {export};
- print exactly one source-free observation JSON to stdout.

Rar independently validates all upstream receipts, binds their exact SHA-256,
checks the final EditorProject operation stream, verifies the edited export,
re-hashes the immutable PUB, and emits only the existing
chaptera.editor-fixed-page-trial-acceptance.v1 receipt.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import tempfile
import zipfile
import xml.etree.ElementTree as ET
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from validate_auth_wrap_native_receipt import validate_receipt as validate_auth_wrap  # noqa: E402
from validate_editor_fixed_page_trial_receipt import (  # noqa: E402
    validate_schema as validate_trial_schema,
    validate_semantics as validate_trial_semantics,
)
from validate_editor_live_trial_package_receipt import validate_receipt as validate_package  # noqa: E402
from validate_replace_image_receipt import validate_pair_receipts  # noqa: E402
from validate_resize_node_producer_receipt import validate_receipt as validate_resize  # noqa: E402
from verify_editable_export_geometry import (  # noqa: E402
    RectEmu,
    verify_export as verify_editable_export_geometry,
)

SAMPLE_SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
SAMPLE_SOURCE_BYTE_LEN = 291840
OBSERVATION_VERSION = "chaptera.editor-fixed-page-trial-observation.v1"
RECEIPT_VERSION = "chaptera.editor-fixed-page-trial-acceptance.v1"
STORY_WITNESS = "ChapteraFixedPageTrialV1"
MAX_PROJECT_BYTES = 64 * 1024 * 1024
MAX_EXPORT_BYTES = 512 * 1024 * 1024

# Valid deterministic 1x1 PNG. The runner owns these exact bytes and verifies
# that the same content identity reaches both EditorProject and editable export.
TRIAL_REPLACEMENT_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl8kAAAAASUVORK5CYII="
)


class FixedPageTrialError(RuntimeError):
    pass


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise FixedPageTrialError(f"{label} is not readable JSON") from error
    if not isinstance(value, dict):
        raise FixedPageTrialError(f"{label} must be a JSON object")
    return value


def git_head() -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    value = completed.stdout.strip()
    if completed.returncode != 0 or len(value) != 40 or any(
        ch not in "0123456789abcdef" for ch in value
    ):
        raise FixedPageTrialError("cannot bind trial receipt to current Rar git commit")
    return value


def bind_fixture(
    fixture: pathlib.Path,
    *,
    expected_hash: str,
    expected_len: int,
) -> pathlib.Path:
    path = fixture.expanduser().resolve(strict=True)
    if not path.is_file():
        raise FixedPageTrialError("fixture path is not a regular file")
    if path.stat().st_size != expected_len:
        raise FixedPageTrialError("fixture byte length mismatch")
    if sha256_file(path) != expected_hash:
        raise FixedPageTrialError("fixture SHA-256 mismatch")
    return path


def require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise FixedPageTrialError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise FixedPageTrialError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} "
            f"extra={sorted(actual-expected)}"
        )
    return value


def normalize_sha256(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise FixedPageTrialError(f"{label} must be a SHA-256 string")
    normalized = value.removeprefix("sha256:").lower()
    if not re.fullmatch(r"[0-9a-f]{64}", normalized):
        raise FixedPageTrialError(f"{label} is not a SHA-256 value")
    return normalized


def validate_upstream_evidence(
    *,
    package_path: pathlib.Path,
    resize_path: pathlib.Path,
    replace_ui_path: pathlib.Path,
    replace_export_path: pathlib.Path,
    auth_wrap_path: pathlib.Path,
    allow_synthetic_for_test: bool = False,
) -> dict[str, Any]:
    package = load_json(package_path, "package receipt")
    resize = load_json(resize_path, "ResizeNode receipt")
    replace_ui = load_json(replace_ui_path, "ReplaceImage UI receipt")
    replace_export = load_json(replace_export_path, "ReplaceImage export receipt")
    auth_wrap = load_json(auth_wrap_path, "AUTH-WRAP receipt")

    try:
        package_summary = validate_package(package)
        resize_summary = validate_resize(resize)
        replace_summary = validate_pair_receipts(replace_ui, replace_export)
        auth_summary = validate_auth_wrap(auth_wrap)
    except AssertionError as error:
        raise FixedPageTrialError(f"upstream evidence validation failed: {error}") from error

    if not allow_synthetic_for_test:
        if package.get("fixture_kind") != "real_pub_sanitized":
            raise FixedPageTrialError("package receipt is not real_pub_sanitized")
        if resize.get("fixture_kind") != "real_pub_sanitized":
            raise FixedPageTrialError("ResizeNode receipt is not real_pub_sanitized")
        if resize.get("producer", {}).get("integration") != "local_private":
            raise FixedPageTrialError("ResizeNode receipt is not local_private evidence")
        if replace_ui.get("fixture_kind") != "real_pub_sanitized":
            raise FixedPageTrialError("ReplaceImage UI receipt is not real_pub_sanitized")
        if replace_export.get("fixture_kind") != "real_pub_sanitized":
            raise FixedPageTrialError("ReplaceImage export receipt is not real_pub_sanitized")
        if auth_wrap.get("receipt_kind") != "native_observation":
            raise FixedPageTrialError("AUTH-WRAP receipt is not native observation")
        if auth_wrap.get("scope", {}).get("closure_candidate") is not True:
            raise FixedPageTrialError("AUTH-WRAP receipt is not a closure candidate")
        if auth_wrap.get("conclusion", {}).get("authority_class") == "inconclusive":
            raise FixedPageTrialError("AUTH-WRAP authority remains inconclusive")
        environment = auth_wrap.get("environment", {})
        if environment.get("reset_provider_receipt_verified") is not True:
            raise FixedPageTrialError("AUTH-WRAP lacks verified reset provider")
        if environment.get("cold_restore_pair_verified") is not True:
            raise FixedPageTrialError("AUTH-WRAP lacks verified cold restore pair")

    return {
        "package": package,
        "resize": resize,
        "replace_ui": replace_ui,
        "replace_export": replace_export,
        "auth_wrap": auth_wrap,
        "package_summary": package_summary,
        "resize_summary": resize_summary,
        "replace_summary": replace_summary,
        "auth_summary": auth_summary,
        "package_sha256": sha256_file(package_path),
        "resize_sha256": sha256_file(resize_path),
        "replace_ui_sha256": sha256_file(replace_ui_path),
        "replace_export_sha256": sha256_file(replace_export_path),
        "auth_wrap_sha256": sha256_file(auth_wrap_path),
        "replacement_binding_id": replace_summary["replacement_binding_id"],
    }


def render_command(
    template: list[str],
    *,
    fixture: pathlib.Path,
    project: pathlib.Path,
    export: pathlib.Path,
    replacement: pathlib.Path,
) -> list[str]:
    if not template:
        raise FixedPageTrialError("trial producer command is empty")
    replacements = {
        "{fixture}": str(fixture),
        "{project}": str(project),
        "{export}": str(export),
        "{replacement}": str(replacement),
    }
    for placeholder in replacements:
        count = sum(part.count(placeholder) for part in template)
        if count != 1:
            raise FixedPageTrialError(
                f"trial producer command must contain {placeholder} exactly once"
            )
    rendered = list(template)
    for placeholder, replacement_value in replacements.items():
        rendered = [part.replace(placeholder, replacement_value) for part in rendered]
    return rendered


def validate_observation(
    value: Any,
    *,
    expected_hash: str,
    rar_commit: str,
    replacement_binding_id: str,
    auth_wrap_sha256: str,
    export_target: str,
) -> dict[str, Any]:
    observation = require_exact_keys(
        value,
        {
            "protocol_version",
            "source_hash",
            "rar_commit",
            "replacement_binding_id",
            "auth_wrap_receipt_sha256",
            "saved_project_sha256",
            "reopened_project_sha256",
            "user_path",
            "export_result",
            "safety",
        },
        "fixed-page observation",
    )
    if observation["protocol_version"] != OBSERVATION_VERSION:
        raise FixedPageTrialError("trial observation protocol_version mismatch")
    if observation["source_hash"] != expected_hash:
        raise FixedPageTrialError("trial observation source hash mismatch")
    if observation["rar_commit"] != rar_commit:
        raise FixedPageTrialError("trial producer ran a different Rar commit")
    if observation["replacement_binding_id"] != replacement_binding_id:
        raise FixedPageTrialError("trial replacement binding differs from paired evidence")
    if observation["auth_wrap_receipt_sha256"] != auth_wrap_sha256:
        raise FixedPageTrialError("trial did not bind the selected AUTH-WRAP receipt")
    user_path = require_exact_keys(
        observation["user_path"],
        {
            "launch_without_dev_toolchain",
            "pub_opened",
            "supported_story_edited",
            "supported_object_moved",
            "supported_object_resized_from_canvas",
            "supported_image_replaced",
            "bounded_wrap_preserved",
            "undo_redo_verified",
            "editor_project_saved",
            "close_reopen_reproduced_state",
            "capability_loss_state_visible",
        },
        "user_path",
    )
    if not all(value is True for value in user_path.values()):
        raise FixedPageTrialError("continuous trial user path is incomplete")

    export_result = require_exact_keys(
        observation["export_result"],
        {
            "target",
            "edited_story_present",
            "moved_geometry_present",
            "resized_geometry_present",
            "replacement_image_exact",
            "bounded_wrap_result_preserved",
            "blocking_loss_count",
            "approximations_explicit",
        },
        "export_result",
    )
    if export_result["target"] != export_target:
        raise FixedPageTrialError("trial export target differs from output suffix")
    if export_result["blocking_loss_count"] != 0:
        raise FixedPageTrialError("continuous trial has blocking export loss")
    for key in (
        "edited_story_present",
        "moved_geometry_present",
        "resized_geometry_present",
        "replacement_image_exact",
        "bounded_wrap_result_preserved",
        "approximations_explicit",
    ):
        if export_result[key] is not True:
            raise FixedPageTrialError(f"continuous trial export proof missing: {key}")

    safety = require_exact_keys(
        observation["safety"],
        {
            "source_pub_immutable",
            "native_save_pub_claimed",
            "unsupported_mutation_fails_closed",
            "no_silent_source_image_fallback",
            "no_hidden_network_upload",
        },
        "safety",
    )
    expected_safety = {
        "source_pub_immutable": True,
        "native_save_pub_claimed": False,
        "unsupported_mutation_fails_closed": True,
        "no_silent_source_image_fallback": True,
        "no_hidden_network_upload": True,
    }
    if safety != expected_safety:
        raise FixedPageTrialError("trial safety boundary changed")

    for label in ("saved_project_sha256", "reopened_project_sha256"):
        normalize_sha256(observation[label], label)

    return observation


def load_and_verify_project(
    path: pathlib.Path,
    *,
    source_hash: str,
    replacement_sha256: str,
) -> tuple[bytes, dict[str, Any], dict[str, dict[str, int]]]:
    if not path.is_file():
        raise FixedPageTrialError("trial producer did not write EditorProject")
    raw = path.read_bytes()
    if not raw or len(raw) > MAX_PROJECT_BYTES:
        raise FixedPageTrialError("EditorProject byte length outside bounded range")
    try:
        project = json.loads(raw)
    except json.JSONDecodeError as error:
        raise FixedPageTrialError("EditorProject is not valid JSON") from error
    if not isinstance(project, dict):
        raise FixedPageTrialError("EditorProject must be a JSON object")
    if normalize_sha256(project.get("source_hash"), "EditorProject.source_hash") != source_hash:
        raise FixedPageTrialError("EditorProject source identity mismatch")
    if project.get("schema_version") != "pub-editor-v0.5":
        raise FixedPageTrialError("fixed-page trial requires pub-editor-v0.5")
    operations = project.get("operations")
    if not isinstance(operations, list):
        raise FixedPageTrialError("EditorProject.operations must be an array")

    expected_kinds = {
        "replace_story_range",
        "move_node",
        "resize_node",
        "replace_image",
    }
    counts = {kind: 0 for kind in expected_kinds}
    geometry_ops: list[dict[str, Any]] = []
    replace_image = None
    story = None
    for operation in operations:
        if not isinstance(operation, dict):
            raise FixedPageTrialError("EditorProject operation must be an object")
        kind = operation.get("kind")
        if kind not in expected_kinds:
            raise FixedPageTrialError(f"unexpected fixed-page trial operation kind: {kind!r}")
        counts[kind] += 1
        if kind in {"move_node", "resize_node"}:
            geometry_ops.append(operation)
        elif kind == "replace_image":
            replace_image = operation
        elif kind == "replace_story_range":
            story = operation

    if len(operations) != 4 or any(count != 1 for count in counts.values()):
        raise FixedPageTrialError(
            "fixed-page EditorProject must contain exactly one Story/Move/Resize/ReplaceImage operation"
        )
    if story is None or story.get("replacement_text") != STORY_WITNESS:
        raise FixedPageTrialError("Story operation does not carry the trial witness")
    if replace_image is None:
        raise FixedPageTrialError("ReplaceImage operation missing")
    if normalize_sha256(replace_image.get("after_asset"), "ReplaceImage.after_asset") != replacement_sha256:
        raise FixedPageTrialError("ReplaceImage operation does not use runner replacement bytes")

    assets = project.get("assets")
    if not isinstance(assets, list):
        raise FixedPageTrialError("EditorProject.assets must be an array")
    if not any(
        isinstance(asset, dict)
        and normalize_sha256(asset.get("sha256"), "EditorProjectAsset.sha256") == replacement_sha256
        for asset in assets
    ):
        raise FixedPageTrialError("EditorProject does not register the runner replacement asset")

    final_bounds: dict[str, dict[str, int]] = {}
    touched_nodes: set[str] = set()
    for operation in geometry_ops:
        node_id = operation.get("node_id")
        after = operation.get("after")
        if not isinstance(node_id, str) or not isinstance(after, dict):
            raise FixedPageTrialError("geometry operation is malformed")
        rect = require_exact_keys(after, {"x", "y", "width", "height"}, "geometry after")
        if not all(isinstance(rect[key], int) and not isinstance(rect[key], bool) for key in rect):
            raise FixedPageTrialError("geometry after must use integer EMU")
        if rect["width"] <= 0 or rect["height"] <= 0:
            raise FixedPageTrialError("geometry after must have positive size")
        final_bounds[node_id] = dict(rect)
        touched_nodes.add(node_id)

    return raw, project, {node: final_bounds[node] for node in touched_nodes}


def _idml_contains_replacement(archive: zipfile.ZipFile, replacement: bytes) -> bool:
    for name in archive.namelist():
        if not name.lower().endswith(".xml"):
            continue
        try:
            text = archive.read(name).decode("utf-8")
        except (UnicodeDecodeError, KeyError):
            continue
        for encoded in re.findall(
            r"<Contents>\s*<!\[CDATA\[(.*?)\]\]>\s*</Contents>",
            text,
            flags=re.DOTALL,
        ):
            compact = "".join(encoded.split())
            try:
                decoded = base64.b64decode(compact, validate=True)
            except (ValueError, base64.binascii.Error):
                continue
            if decoded == replacement:
                return True
    return False


def _odg_contains_replacement(archive: zipfile.ZipFile, replacement_sha256: str) -> bool:
    for name in archive.namelist():
        if name.endswith("/"):
            continue
        try:
            raw = archive.read(name)
        except KeyError:
            continue
        if sha256_bytes(raw) == replacement_sha256:
            return True
    return False


def verify_export_package(
    path: pathlib.Path,
    export_target: str,
    *,
    geometry: dict[str, dict[str, int]],
    replacement: bytes,
) -> dict[str, bool]:
    if not path.is_file():
        raise FixedPageTrialError("trial producer did not write edited export")
    raw = path.read_bytes()
    if not raw or len(raw) > MAX_EXPORT_BYTES:
        raise FixedPageTrialError("edited export byte length outside bounded range")
    if not zipfile.is_zipfile(path):
        raise FixedPageTrialError("edited export is not a ZIP package")

    replacement_sha = sha256_bytes(replacement)
    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        if export_target == "idml":
            if "designmap.xml" not in names:
                raise FixedPageTrialError("IDML package is missing designmap.xml")
            story_parts = sorted(
                name for name in names if name.startswith("Stories/") and name.endswith(".xml")
            )
            try:
                edited_story_present = any(
                    STORY_WITNESS in "".join(ET.fromstring(archive.read(name)).itertext())
                    for name in story_parts
                )
            except ET.ParseError as error:
                raise FixedPageTrialError("IDML Story XML is not parseable") from error
            replacement_exact = _idml_contains_replacement(archive, replacement)
        elif export_target == "odg":
            if "mimetype" not in names or "content.xml" not in names:
                raise FixedPageTrialError("ODG package is missing mimetype/content.xml")
            if archive.read("mimetype") != b"application/vnd.oasis.opendocument.graphics":
                raise FixedPageTrialError("ODG mimetype mismatch")
            try:
                edited_story_present = STORY_WITNESS in "".join(
                    ET.fromstring(archive.read("content.xml")).itertext()
                )
            except ET.ParseError as error:
                raise FixedPageTrialError("ODG content.xml is not parseable") from error
            replacement_exact = _odg_contains_replacement(archive, replacement_sha)
        else:
            raise FixedPageTrialError("unsupported edited export target")

    if not edited_story_present:
        raise FixedPageTrialError("edited export does not contain the Story witness")
    if not replacement_exact:
        raise FixedPageTrialError("edited export does not contain exact replacement bytes")

    for node_id, rect in geometry.items():
        try:
            proof = verify_editable_export_geometry(
                path,
                export_target,
                node_id,
                RectEmu(
                    x=rect["x"],
                    y=rect["y"],
                    width=rect["width"],
                    height=rect["height"],
                ),
            )
        except (AssertionError, KeyError, TypeError, ValueError, zipfile.BadZipFile) as error:
            raise FixedPageTrialError(
                "edited export does not reproduce final Move/Resize geometry"
            ) from error
        if proof.get("geometry_matches_edit") is not True:
            raise FixedPageTrialError("edited export geometry proof is not affirmative")

    return {
        "edited_story_present": True,
        "moved_geometry_present": True,
        "resized_geometry_present": True,
        "replacement_image_exact": True,
    }


def run_local_fixed_page_trial(
    *,
    fixture: pathlib.Path,
    package_receipt: pathlib.Path,
    resize_receipt: pathlib.Path,
    replace_ui_receipt: pathlib.Path,
    replace_export_receipt: pathlib.Path,
    auth_wrap_receipt: pathlib.Path,
    project_output: pathlib.Path,
    export_output: pathlib.Path,
    receipt_output: pathlib.Path,
    command_template: list[str],
    expected_hash: str = SAMPLE_SOURCE_HASH,
    expected_len: int = SAMPLE_SOURCE_BYTE_LEN,
    rar_commit: str | None = None,
    host_system: str | None = None,
    allow_synthetic_upstream_for_test: bool = False,
) -> dict[str, Any]:
    host_system = host_system or platform.system()
    if host_system != "Windows":
        raise FixedPageTrialError("real fixed-page trial must execute on Windows")

    fixture = bind_fixture(fixture, expected_hash=expected_hash, expected_len=expected_len)
    rar_commit = rar_commit or git_head()
    evidence = validate_upstream_evidence(
        package_path=package_receipt,
        resize_path=resize_receipt,
        replace_ui_path=replace_ui_receipt,
        replace_export_path=replace_export_receipt,
        auth_wrap_path=auth_wrap_receipt,
        allow_synthetic_for_test=allow_synthetic_upstream_for_test,
    )

    project_output = project_output.expanduser().resolve()
    export_output = export_output.expanduser().resolve()
    receipt_output = receipt_output.expanduser().resolve()
    suffix = export_output.suffix.lower()
    if suffix not in {".idml", ".odg"}:
        raise FixedPageTrialError("export output must end in .idml or .odg")
    export_target = suffix[1:]

    for output in (project_output, export_output, receipt_output):
        output.parent.mkdir(parents=True, exist_ok=True)
        if output.exists():
            output.unlink()

    replacement_sha = sha256_bytes(TRIAL_REPLACEMENT_PNG)

    with tempfile.TemporaryDirectory(prefix="chaptera-fixed-page-trial-") as tmp:
        replacement_path = pathlib.Path(tmp) / "replacement.png"
        replacement_path.write_bytes(TRIAL_REPLACEMENT_PNG)
        command = render_command(
            command_template,
            fixture=fixture,
            project=project_output,
            export=export_output,
            replacement=replacement_path,
        )

        env = dict(os.environ)
        env["CHAPTERA_RAR_COMMIT"] = rar_commit
        env["CHAPTERA_SOURCE_HASH"] = expected_hash
        env["CHAPTERA_FIXED_PAGE_TRIAL_V1"] = "1"
        env["CHAPTERA_TRIAL_STORY_WITNESS"] = STORY_WITNESS
        env["CHAPTERA_REPLACEMENT_BINDING_ID"] = evidence["replacement_binding_id"]
        env["CHAPTERA_AUTH_WRAP_RECEIPT_SHA256"] = evidence["auth_wrap_sha256"]

        completed = subprocess.run(
            command,
            cwd=ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            check=False,
        )
        if completed.returncode != 0:
            detail = completed.stderr.decode("utf-8", errors="replace").strip()
            raise FixedPageTrialError(
                "local fixed-page producer failed" + (f": {detail}" if detail else "")
            )
        try:
            observation_raw = json.loads(completed.stdout.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise FixedPageTrialError(
                "trial producer stdout is not exactly one UTF-8 JSON value"
            ) from error

    observation = validate_observation(
        observation_raw,
        expected_hash=expected_hash,
        rar_commit=rar_commit,
        replacement_binding_id=evidence["replacement_binding_id"],
        auth_wrap_sha256=evidence["auth_wrap_sha256"],
        export_target=export_target,
    )

    if fixture.stat().st_size != expected_len or sha256_file(fixture) != expected_hash:
        raise FixedPageTrialError("source PUB changed during fixed-page trial")

    project_raw, _project, geometry = load_and_verify_project(
        project_output,
        source_hash=expected_hash,
        replacement_sha256=replacement_sha,
    )
    project_sha = sha256_bytes(project_raw)
    if normalize_sha256(observation["saved_project_sha256"], "saved_project_sha256") != project_sha:
        raise FixedPageTrialError("saved project hash differs from produced EditorProject")
    if normalize_sha256(observation["reopened_project_sha256"], "reopened_project_sha256") != project_sha:
        raise FixedPageTrialError("fresh reopen did not reproduce exact saved EditorProject")

    independent_export = verify_export_package(
        export_output,
        export_target,
        geometry=geometry,
        replacement=TRIAL_REPLACEMENT_PNG,
    )
    for key, value in independent_export.items():
        if observation["export_result"][key] is not value:
            raise FixedPageTrialError(f"producer export claim differs from independent proof: {key}")

    auth_wrap = evidence["auth_wrap"]
    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "receipt_kind": "real_trial",
        "product": "Chaptera Editor",
        "trial_phase": "public_surrogate",
        "source_fixture": {
            "public_lineage": "apache_poi_sample_newsletter",
            "public_sha256": expected_hash,
            "private_identity_redacted": True,
        },
        "evidence_chain": {
            "package": {
                "receipt_sha256": evidence["package_sha256"],
                "validated": True,
            },
            "resize_node": {
                "receipt_sha256": evidence["resize_sha256"],
                "validated": True,
            },
            "replace_image_ui": {
                "receipt_sha256": evidence["replace_ui_sha256"],
                "validated": True,
                "replacement_binding_id": evidence["replacement_binding_id"],
            },
            "replace_image_export": {
                "receipt_sha256": evidence["replace_export_sha256"],
                "validated": True,
                "replacement_binding_id": evidence["replacement_binding_id"],
            },
            "auth_wrap": {
                "receipt_sha256": evidence["auth_wrap_sha256"],
                "validated": True,
                "native_observation": auth_wrap.get("receipt_kind") == "native_observation",
                "closure_candidate": auth_wrap.get("scope", {}).get("closure_candidate") is True,
                "authority_class": auth_wrap.get("conclusion", {}).get("authority_class"),
            },
        },
        "user_path": observation["user_path"],
        "export_result": {
            **observation["export_result"],
            **independent_export,
        },
        "safety": observation["safety"],
        "privacy": {
            "pub_bytes_in_receipt": False,
            "private_filename_in_receipt": False,
            "local_path_in_receipt": False,
            "document_text_in_receipt": False,
            "replacement_asset_bytes_in_receipt": False,
            "customer_identity_in_receipt": False,
        },
    }

    try:
        validate_trial_schema(receipt)
        validate_trial_semantics(receipt)
    except AssertionError as error:
        raise FixedPageTrialError(str(error)) from error

    receipt_output.write_text(
        json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run and independently verify the Chaptera fixed-page public-surrogate trial"
    )
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--package-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--resize-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--replace-ui-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--replace-export-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--auth-wrap-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--project-output", required=True, type=pathlib.Path)
    parser.add_argument("--export-output", required=True, type=pathlib.Path)
    parser.add_argument("--receipt-output", required=True, type=pathlib.Path)
    parser.add_argument("trial_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.trial_command)
    if command and command[0] == "--":
        command = command[1:]

    try:
        receipt = run_local_fixed_page_trial(
            fixture=args.fixture,
            package_receipt=args.package_receipt,
            resize_receipt=args.resize_receipt,
            replace_ui_receipt=args.replace_ui_receipt,
            replace_export_receipt=args.replace_export_receipt,
            auth_wrap_receipt=args.auth_wrap_receipt,
            project_output=args.project_output,
            export_output=args.export_output,
            receipt_output=args.receipt_output,
            command_template=command,
        )
    except (FixedPageTrialError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 2

    print(
        json.dumps(
            {
                "status": "valid",
                "receipt": str(args.receipt_output),
                "trial_phase": receipt["trial_phase"],
                "export_target": receipt["export_result"]["target"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
