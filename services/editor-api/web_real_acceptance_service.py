#!/usr/bin/env python3
"""Real pinned-PUB backend for WEB-ACCEPTANCE-LOCAL-PRODUCER-01.

This reuses the public RevisionKernel/HTTP protocol and the already-proven
Rar-owned bounded producers:
- Producer B for canonical MoveNode semantics;
- LAYOUT-RESOLVED-SCENE-01 for current resolved-graph -> Scene;
- the bounded editable-export authority for IDML/ODG.

It is task-local integration glue, not a second document model.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlsplit

ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from adapt_viewer_scene_v1 import adapt_viewer_geometry
from resolved_graph_scene_bridge_v1 import (
    apply_project_to_resolved_graph,
    compare_viewer_and_adapter_scene,
    project_resolved_graph_scene,
)
from sample_newsletter_move_producer import (
    SampleNewsletterMoveSession,
    load_baseline,
)
from validate_export_preview import validate_schema as validate_export_preview_schema
from validate_export_preview import validate_semantics as validate_export_preview_semantics
from verify_editable_export_geometry import RectEmu, verify_export

from revision_store import RevisionKernel
from security.authz_v1 import AuthzDenied, AuthzKernel, CAP_EXPORT, CAP_VIEW
from security.authorized_revision_gateway import AuthorizedRevisionGateway

PINNED_SHA = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
PINNED_LEN = 291840

STATE = None


def canonical_json(value) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def load_json(path: pathlib.Path):
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"{path.name} must contain a JSON object")
    return value


def sha256_path(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class RealAcceptanceState:
    def __init__(
        self,
        *,
        fixture: pathlib.Path,
        resolved_graph: pathlib.Path,
        viewer_receipt: pathlib.Path,
        revision_receipt: pathlib.Path,
        exporter: pathlib.Path,
        work_dir: pathlib.Path,
        strict_acceptance: bool = True,
    ):
        self.fixture = fixture.resolve(strict=True)
        self.resolved_graph_path = resolved_graph.resolve(strict=True)
        self.viewer_receipt_path = viewer_receipt.resolve(strict=True)
        self.revision_receipt_path = revision_receipt.resolve(strict=True)
        self.exporter = exporter.resolve(strict=True)
        self.work_dir = work_dir.resolve()
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.strict_acceptance = strict_acceptance

        if self.fixture.stat().st_size != PINNED_LEN or sha256_path(self.fixture) != PINNED_SHA:
            raise RuntimeError("pinned SampleNewsletter source identity mismatch")

        self.revision_receipt = load_json(self.revision_receipt_path)
        self.document_id = self.revision_receipt["document_id"]
        self.source_hash = self.revision_receipt["source_hash"]
        if self.source_hash != PINNED_SHA:
            raise RuntimeError("revision receipt is not bound to pinned source")

        self.baseline_project = copy.deepcopy(self.revision_receipt["baseline"]["project"])
        self.expected_baseline_revision = self.revision_receipt["baseline"]["revision_id"]
        self.expected_accepted_revision = self.revision_receipt["accepted"]["revision_id"]
        self.canonical_operation = copy.deepcopy(
            self.revision_receipt["accepted"]["canonical_operation"]
        )
        self.canonical_request = copy.deepcopy(self.revision_receipt["request"])
        self.target_node_id = self.canonical_operation["node_id"]
        self.target_after = copy.deepcopy(self.canonical_operation["after"])

        self.kernel = RevisionKernel()
        baseline = self.kernel.register_baseline(
            document_id=self.document_id,
            source_hash=self.source_hash,
            project=copy.deepcopy(self.baseline_project),
        )
        if baseline.revision_id != self.expected_baseline_revision:
            raise RuntimeError("RevisionKernel baseline differs from Producer B")

        self.tenant_id = "web-acceptance-real"
        self.authz = AuthzKernel()
        for principal, role in (
            ("synthetic-editor", "editor"),
            ("synthetic-viewer", "viewer"),
        ):
            self.authz.set_role(
                tenant_id=self.tenant_id,
                document_id=self.document_id,
                principal_id=principal,
                role=role,
            )
        self.gateway = AuthorizedRevisionGateway(
            kernel=self.kernel,
            authz=self.authz,
            tenant_id=self.tenant_id,
        )

        self.commit_requests = 0
        self.history_requests = 0
        self.executor_calls = 0
        self.history_executor_calls = 0
        self.redo_stack = []
        self.last_operation = None
        self.reopen_count = 0
        self.export_cache = {}

        baseline_scene = self._scene_from_project(
            self.baseline_project,
            self.expected_baseline_revision,
            require_viewer_equivalence=True,
        )
        self.scenes = {self.expected_baseline_revision: baseline_scene}

    def _fresh_graph_and_viewer(self):
        graph = load_json(self.resolved_graph_path)
        viewer = load_json(self.viewer_receipt_path)
        return graph, viewer

    def _scene_from_project(
        self,
        project: dict,
        revision_id: str,
        *,
        require_viewer_equivalence: bool = False,
    ) -> dict:
        graph, viewer = self._fresh_graph_and_viewer()
        current_graph = apply_project_to_resolved_graph(graph, project)
        source_scene = project_resolved_graph_scene(current_graph)
        if require_viewer_equivalence:
            compare_viewer_and_adapter_scene(viewer, source_scene)
        current_viewer = copy.deepcopy(viewer)
        current_viewer["scene"] = source_scene
        return adapt_viewer_geometry(current_viewer, self.document_id, revision_id)

    def executor(self, project: dict, command: dict):
        self.executor_calls += 1
        self.redo_stack.clear()
        session = SampleNewsletterMoveSession(load_baseline())
        session.apply_project(copy.deepcopy(project))
        if not isinstance(command, dict) or command.get("kind") != "move_node_to":
            raise ValueError("unsupported real acceptance command")
        operation = session.move_node_to(
            command.get("node_id"),
            command.get("x_emu"),
            command.get("y_emu"),
        )
        self.last_operation = copy.deepcopy(operation)
        return (
            operation,
            session.project(),
            [{"key": "node.geometry.position", "state": "supported", "note": None}],
        )

    def history_executor(self, base_project: dict, transition_kind: str):
        self.history_executor_calls += 1
        project = copy.deepcopy(base_project)
        operations = list(project.get("operations", []))
        if transition_kind == "undo":
            if not operations:
                raise ValueError("nothing to undo")
            self.redo_stack.append(copy.deepcopy(operations.pop()))
        elif transition_kind == "redo":
            if not self.redo_stack:
                raise ValueError("nothing to redo")
            operations.append(self.redo_stack.pop())
        else:
            raise ValueError("unsupported history transition")
        project["operations"] = operations
        project["schema_version"] = "pub-editor-v0.4" if operations else "pub-editor-v0.2"
        return project, [{"key": "history." + transition_kind, "state": "supported", "note": None}]

    def commit(self, request: dict, principal_id: str) -> dict:
        protocol = request.get("protocol_version")
        if protocol == "chaptera.commit-request.v1":
            result = self.gateway.commit(
                request,
                principal_id=principal_id,
                executor=self.executor,
            )
        elif protocol == "chaptera.history-transition-intent.v1":
            result = self.gateway.commit(
                request,
                principal_id=principal_id,
                history_executor=self.history_executor,
            )
        else:
            raise ValueError("unsupported commit protocol")

        self.commit_requests += 1
        if protocol == "chaptera.history-transition-intent.v1":
            self.history_requests += 1

        if result.get("protocol_version") in {
            "chaptera.commit-accepted.v1",
            "chaptera.history-transition-accepted.v1",
        }:
            record = self.kernel.read_revision(
                document_id=self.document_id,
                revision_id=result["revision_id"],
            )
            self.scenes[result["revision_id"]] = self._scene_from_project(
                record.project,
                result["revision_id"],
            )
            if (
                self.strict_acceptance
                and protocol == "chaptera.commit-request.v1"
                and self.executor_calls == 1
            ):
                if self.last_operation != self.canonical_operation:
                    raise RuntimeError("browser MoveNode differs from canonical Producer B operation")
                if result["revision_id"] != self.expected_accepted_revision:
                    raise RuntimeError("browser accepted revision differs from Producer B")
        return copy.deepcopy(result)

    def _export_for_revision(self, revision_id: str, target: str):
        key = (revision_id, target)
        if key in self.export_cache:
            return self.export_cache[key]
        if target not in {"idml", "odg"}:
            raise ValueError("export target must be idml or odg")

        record = self.kernel.read_revision(
            document_id=self.document_id,
            revision_id=revision_id,
        )
        token = revision_id.removeprefix("sha256:")[:20]
        project_path = self.work_dir / f"{token}.project.json"
        artifact_path = self.work_dir / f"{token}.{target}"
        report_path = self.work_dir / f"{token}.export-report.json"
        project_path.write_bytes(canonical_json(record.project) + b"\n")

        completed = subprocess.run(
            [
                str(self.exporter),
                "editable-export",
                str(self.fixture),
                str(project_path),
                target,
                str(artifact_path),
                str(report_path),
            ],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                "editable export producer failed: " + completed.stderr.strip()
            )

        report = load_json(report_path)
        if report.get("schema_version") != "0.1":
            raise RuntimeError("unexpected export report schema")
        if report.get("source", {}).get("source_hash") != self.source_hash:
            raise RuntimeError("export report source identity mismatch")

        fence = report.get("conversion_fence")
        preview = {
            "protocol_version": "chaptera.export-preview.v1",
            "report_schema_version": report["schema_version"],
            "document_id": self.document_id,
            "source_hash": self.source_hash,
            "revision_id": revision_id,
            "target": copy.deepcopy(report["target"]),
            "conversion_fence_sha256": (
                fence.get("digest_sha256") if isinstance(fence, dict) else None
            ),
            "can_serialize": report["can_serialize"],
            "counts": copy.deepcopy(report["counts"]),
            "items": copy.deepcopy(report["items"]),
        }
        validate_export_preview_schema(preview)
        validate_export_preview_semantics(preview)

        value = {
            "preview": preview,
            "artifact": artifact_path,
            "report": report,
        }
        self.export_cache[key] = value
        return value

    def export_preview(self, target: str) -> dict:
        current = self.kernel.current_revision(self.document_id)
        return copy.deepcopy(
            self._export_for_revision(current.revision_id, target)["preview"]
        )

    def export_proof(self, target: str = "idml") -> dict:
        current = self.kernel.current_revision(self.document_id)
        value = self._export_for_revision(current.revision_id, target)
        rect = RectEmu(
            self.target_after["x"],
            self.target_after["y"],
            self.target_after["width"],
            self.target_after["height"],
        )
        return verify_export(
            value["artifact"],
            target,
            self.target_node_id,
            rect,
        )

    def reopen(self) -> dict:
        current = self.kernel.current_revision(self.document_id)
        record = self.kernel.read_revision(
            document_id=self.document_id,
            revision_id=current.revision_id,
        )
        # Re-read immutable inputs and rebuild from the persisted canonical project.
        if self.fixture.stat().st_size != PINNED_LEN or sha256_path(self.fixture) != PINNED_SHA:
            raise RuntimeError("immutable source changed before reopen")
        fresh = self._scene_from_project(record.project, current.revision_id)
        previous = self.scenes[current.revision_id]
        if fresh != previous:
            raise RuntimeError("fresh reopen Scene differs from persisted revision Scene")
        self.scenes[current.revision_id] = fresh
        self.reopen_count += 1
        return copy.deepcopy(fresh)

    def state(self) -> dict:
        current = self.kernel.current_revision(self.document_id)
        return {
            "receipt_class": "real_pub_browser",
            "real_pub": True,
            "product_acceptance": True,
            "document_id": self.document_id,
            "source_hash": self.source_hash,
            "fixture_sha256": sha256_path(self.fixture),
            "fixture_byte_len": self.fixture.stat().st_size,
            "current_revision_id": current.revision_id,
            "current_snapshot_id": self.scenes[current.revision_id]["snapshot_id"],
            "commit_requests": self.commit_requests,
            "history_requests": self.history_requests,
            "executor_calls": self.executor_calls,
            "history_executor_calls": self.history_executor_calls,
            "reopen_count": self.reopen_count,
            "last_operation": copy.deepcopy(self.last_operation),
        }


class Handler(BaseHTTPRequestHandler):
    server_version = "ChapteraRealAcceptance/1"

    def _headers(self, status=200):
        self.send_response(status)
        self.send_header("content-type", "application/json; charset=utf-8")
        self.send_header("cache-control", "no-store")
        self.send_header("access-control-allow-origin", "*")
        self.send_header("access-control-allow-methods", "GET, POST, OPTIONS")
        self.send_header(
            "access-control-allow-headers",
            "content-type, x-chaptera-principal-id, x-chaptera-trace-version, "
            "x-chaptera-trace-id, x-chaptera-interaction-id, "
            "x-chaptera-session-incarnation, x-chaptera-operation-class, "
            "x-chaptera-browser-family",
        )
        self.end_headers()

    def _json(self, value, status=200):
        self._headers(status)
        self.wfile.write(
            json.dumps(value, ensure_ascii=False, sort_keys=True).encode("utf-8")
        )

    def _principal_id(self):
        value = self.headers.get("x-chaptera-principal-id")
        if not value:
            raise AuthzDenied("principal_missing")
        return value

    def _authorize(self, capability):
        STATE.authz.authorize(
            tenant_id=STATE.tenant_id,
            document_id=STATE.document_id,
            principal_id=self._principal_id(),
            capability=capability,
        )

    def do_OPTIONS(self):
        self._headers(204)

    def do_GET(self):
        parsed = urlsplit(self.path)
        path = unquote(parsed.path)
        query = parse_qs(parsed.query)
        try:
            if path == "/health":
                self._json({"ok": True, "receipt_class": "real_pub_browser"})
                return
            if path == "/v1/export/preview":
                self._authorize(CAP_EXPORT)
                target = query.get("target", [""])[0]
                self._json(STATE.export_preview(target))
                return
            if path == "/v1/scenes/current":
                self._authorize(CAP_VIEW)
                current = STATE.kernel.current_revision(STATE.document_id).revision_id
                self._json(STATE.scenes[current])
                return
            if path.startswith("/v1/scenes/"):
                self._authorize(CAP_VIEW)
                revision_id = path.removeprefix("/v1/scenes/")
                scene = STATE.scenes.get(revision_id)
                if scene is None:
                    self._json({"error": "scene_not_found"}, 404)
                else:
                    self._json(scene)
                return
            if path == "/v1/harness/state":
                self._json(STATE.state())
                return
            if path == "/v1/harness/export-proof":
                self._json(STATE.export_proof(query.get("target", ["idml"])[0]))
                return
            self._json({"error": "not_found"}, 404)
        except AuthzDenied as exc:
            self._json({"error": "forbidden", "code": exc.code}, 403)
        except Exception as exc:
            self._json({"error": "real_acceptance_error", "detail": str(exc)}, 500)

    def do_POST(self):
        path = unquote(self.path.split("?", 1)[0])
        try:
            if path == "/v1/harness/reopen":
                self._json(STATE.reopen())
                return
            if path != "/v1/commit":
                self._json({"error": "not_found"}, 404)
                return
            length = int(self.headers.get("content-length", "0"))
            if length <= 0 or length > 1024 * 1024:
                raise ValueError("invalid request size")
            request = json.loads(self.rfile.read(length).decode("utf-8"))
            result = STATE.commit(request, self._principal_id())
            self._json(result)
        except AuthzDenied as exc:
            self._json({"error": "forbidden", "code": exc.code}, 403)
        except (ValueError, KeyError, TypeError) as exc:
            self._json({"error": "invalid_request", "detail": str(exc)}, 400)
        except Exception as exc:
            self._json({"error": "real_acceptance_error", "detail": str(exc)}, 500)

    def log_message(self, format, *args):
        return


def main():
    global STATE
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--resolved-graph", required=True, type=pathlib.Path)
    parser.add_argument("--viewer-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--revision-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--exporter", required=True, type=pathlib.Path)
    parser.add_argument("--work-dir", required=True, type=pathlib.Path)
    parser.add_argument(
        "--interactive",
        action="store_true",
        help="allow arbitrary capability-approved MoveNode edits for local product use",
    )
    args = parser.parse_args()

    STATE = RealAcceptanceState(
        fixture=args.fixture,
        resolved_graph=args.resolved_graph,
        viewer_receipt=args.viewer_receipt,
        revision_receipt=args.revision_receipt,
        exporter=args.exporter,
        work_dir=args.work_dir,
        strict_acceptance=not args.interactive,
    )
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(
        json.dumps(
            {
                "ready": True,
                "port": server.server_address[1],
                "receipt_class": "real_pub_browser",
                "real_pub": True,
                "product_acceptance": not args.interactive,
                "interactive": args.interactive,
            }
        ),
        flush=True,
    )
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
