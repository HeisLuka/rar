#!/usr/bin/env python3
"""Build a deterministic self-contained Chaptera Local Windows ZIP."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import stat
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXED_TIME = (1980, 1, 1, 0, 0, 0)

TOOL_FILES = (
    "adapt_viewer_scene_v1.py",
    "resolved_graph_scene_bridge_v1.py",
    "sample_newsletter_move_producer.py",
    "scene_v1.py",
    "validate_export_preview.py",
    "verify_editable_export_geometry.py",
)
WEB_FILES = (
    "local-editor.html",
    "editor-shell-v1.mjs",
    "editor-service-client-v1.mjs",
    "observability-v1.mjs",
    "render-v1.mjs",
    "interaction-v1.mjs",
    "export-preview-v1.schema.json",
)


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def copy_file(source: pathlib.Path, target: pathlib.Path) -> None:
    if not source.is_file():
        raise RuntimeError(f"required package file missing: {source}")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)


def copy_tree(source: pathlib.Path, target: pathlib.Path, *, predicate=None) -> None:
    if not source.is_dir():
        raise RuntimeError(f"required package directory missing: {source}")
    for path in sorted(source.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(source)
        if "__pycache__" in relative.parts or path.suffix in {".pyc", ".pyo"}:
            continue
        if predicate is not None and not predicate(path, relative):
            continue
        copy_file(path, target / relative)


def patch_embedded_python(root: pathlib.Path) -> None:
    candidates = sorted(root.glob("python*._pth"))
    if len(candidates) != 1:
        raise RuntimeError(
            f"expected exactly one embedded Python _pth file, found {len(candidates)}"
        )
    pth = candidates[0]
    zip_names = sorted(p.name for p in root.glob("python*.zip"))
    if len(zip_names) != 1:
        raise RuntimeError("expected exactly one embedded Python stdlib ZIP")
    pth.write_text(
        "\n".join(
            [
                zip_names[0],
                ".",
                "..\\python-packages",
                "..\\..\\launcher",
                "..\\..\\tools",
                "..\\..\\services\\editor-api",
                "import site",
                "",
            ]
        ),
        encoding="utf-8",
    )


def zip_entry(name: str, executable: bool = False) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name.replace("\\", "/"), FIXED_TIME)
    info.compress_type = zipfile.ZIP_DEFLATED
    mode = 0o755 if executable else 0o644
    info.external_attr = (stat.S_IFREG | mode) << 16
    return info


def stage_package(
    stage: pathlib.Path,
    *,
    chaptera_exe: pathlib.Path,
    producer_exe: pathlib.Path,
    python_runtime: pathlib.Path,
    python_packages: pathlib.Path,
    source_commit: str,
    workflow_run: str,
) -> dict:
    copy_file(ROOT / "Start-Chaptera-Local.cmd", stage / "Start-Chaptera-Local.cmd")
    copy_file(chaptera_exe, stage / "bin/chaptera.exe")
    copy_file(producer_exe, stage / "bin/chaptera-producer-a.exe")

    copy_tree(python_runtime, stage / "runtime/python")
    copy_tree(python_packages, stage / "runtime/python-packages")
    patch_embedded_python(stage / "runtime/python")

    for name in ("run_local_full_stack.py", "run_local_real_editor.py", "run_cloud_config_receipt.py"):
        copy_file(ROOT / "tools" / name, stage / "launcher" / name)
    for name in TOOL_FILES:
        copy_file(ROOT / "tools" / name, stage / "tools" / name)

    def editor_api_predicate(path: pathlib.Path, relative: pathlib.Path) -> bool:
        if path.name.startswith("test_"):
            return False
        if path.suffix == ".py":
            return True
        if "security" in relative.parts and path.suffix == ".json":
            return True
        return False

    copy_tree(
        ROOT / "services/editor-api",
        stage / "services/editor-api",
        predicate=editor_api_predicate,
    )

    for name in WEB_FILES:
        copy_file(ROOT / "apps/web" / name, stage / "apps/web" / name)
    copy_tree(
        ROOT / "apps/web/acceptance/receipts",
        stage / "apps/web/acceptance/receipts",
    )
    copy_tree(
        ROOT / "packages/protocol/revision/v1",
        stage / "packages/protocol/revision/v1",
    )
    copy_file(
        ROOT / "deploy/config/chaptera.prod.example.toml",
        stage / "deploy/config/chaptera.prod.example.toml",
    )
    copy_file(
        ROOT / "packages/product/local-portable/v1/requirements.txt",
        stage / "PYTHON-REQUIREMENTS.txt",
    )
    copy_file(
        ROOT / "packages/product/local-portable/v1/README-FIRST-RUN.md",
        stage / "README-FIRST-RUN.md",
    )

    for forbidden in stage.rglob("*"):
        if forbidden.is_file() and (
            forbidden.name == "Cargo.toml"
            or forbidden.suffix.lower() == ".pub"
            or ".git" in forbidden.parts
            or forbidden.name.startswith("test_")
        ):
            raise RuntimeError(f"forbidden package entry: {forbidden.relative_to(stage)}")

    build = {
        "schema_version": "chaptera.local-portable-build.v1",
        "product": "Chaptera Local",
        "target": "windows-x86_64",
        "source_commit": source_commit,
        "workflow_run": workflow_run,
        "chaptera_sha256": sha256_file(stage / "bin/chaptera.exe"),
        "producer_sha256": sha256_file(stage / "bin/chaptera-producer-a.exe"),
        "python_executable_sha256": sha256_file(stage / "runtime/python/python.exe"),
    }
    (stage / "BUILD.json").write_text(
        json.dumps(build, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    rows = []
    for path in sorted(p for p in stage.rglob("*") if p.is_file()):
        rel = path.relative_to(stage).as_posix()
        if rel == "SHA256SUMS":
            continue
        rows.append(f"{sha256_file(path)}  {rel}")
    (stage / "SHA256SUMS").write_text("\n".join(rows) + "\n", encoding="ascii")
    return build


def write_zip(stage: pathlib.Path, output: pathlib.Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        output.unlink()
    with zipfile.ZipFile(output, "w", allowZip64=True) as archive:
        for path in sorted(p for p in stage.rglob("*") if p.is_file()):
            rel = path.relative_to(stage).as_posix()
            executable = path.suffix.lower() == ".exe"
            archive.writestr(zip_entry(rel, executable=executable), path.read_bytes())


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--chaptera-exe", required=True, type=pathlib.Path)
    parser.add_argument("--producer-exe", required=True, type=pathlib.Path)
    parser.add_argument("--python-runtime", required=True, type=pathlib.Path)
    parser.add_argument("--python-packages", required=True, type=pathlib.Path)
    parser.add_argument("--output-zip", required=True, type=pathlib.Path)
    parser.add_argument("--manifest", required=True, type=pathlib.Path)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--workflow-run", default="")
    args = parser.parse_args()

    if args.chaptera_exe.suffix.lower() != ".exe" or args.producer_exe.suffix.lower() != ".exe":
        raise RuntimeError("portable Windows package requires .exe binaries")

    with tempfile.TemporaryDirectory(prefix="chaptera-local-package-") as temp:
        stage = pathlib.Path(temp) / "Chaptera-Local"
        stage.mkdir()
        build = stage_package(
            stage,
            chaptera_exe=args.chaptera_exe,
            producer_exe=args.producer_exe,
            python_runtime=args.python_runtime,
            python_packages=args.python_packages,
            source_commit=args.source_commit,
            workflow_run=args.workflow_run,
        )
        write_zip(stage, args.output_zip)
        manifest = {
            "schema_version": "chaptera.local-portable-package.v1",
            **build,
            "zip_sha256": sha256_file(args.output_zip),
            "zip_size": args.output_zip.stat().st_size,
            "entry_count": sum(1 for p in stage.rglob("*") if p.is_file()),
        }
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
