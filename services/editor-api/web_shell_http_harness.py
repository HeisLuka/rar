#!/usr/bin/env python3
"""Synthetic HTTP process boundary for Web Editor browser/service plumbing.

This harness intentionally uses the public RevisionKernel plus a Scene V1 fixture.
It is not the canonical private EditorSession producer and cannot satisfy
WEB-ACCEPTANCE-01 real-PUB closure.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlsplit

from revision_store import RevisionKernel
from observability import TraceRecorder, trace_context_from_headers
from security.authz_v1 import (
    AuthzDenied,
    AuthzKernel,
    CAP_EXPORT,
    CAP_VIEW,
)
from security.authorized_revision_gateway import AuthorizedRevisionGateway

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "packages" / "protocol" / "scene" / "v1" / "fixtures" / "simple-text.json"
ENVIRONMENT = "sha256:" + "e" * 64
STAGES = {"layout": "layout-v1", "scene": "scene-v1"}


def canonical_json(value) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def scene_snapshot_id(scene: dict) -> str:
    value = copy.deepcopy(scene)
    value.pop("snapshot_id", None)
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


class HarnessState:
    def __init__(self) -> None:
        raw_scene = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.document_id = raw_scene["document_id"]
        self.source_hash = raw_scene["source_hash"]
        self.kernel = RevisionKernel()
        self.tenant_id = "synthetic-tenant"
        self.authz = AuthzKernel()
        self.authz.set_role(
            tenant_id=self.tenant_id,
            document_id=self.document_id,
            principal_id="synthetic-editor",
            role="editor",
        )
        self.gateway = AuthorizedRevisionGateway(
            kernel=self.kernel,
            authz=self.authz,
            tenant_id=self.tenant_id,
        )
        baseline_project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": self.source_hash,
            "operations": [],
        }
        baseline = self.kernel.register_baseline(
            document_id=self.document_id,
            source_hash=self.source_hash,
            project=baseline_project,
        )
        raw_scene["revision_id"] = baseline.revision_id
        raw_scene["snapshot_id"] = scene_snapshot_id(raw_scene)
        self.baseline_scene = copy.deepcopy(raw_scene)
        self.scenes = {baseline.revision_id: copy.deepcopy(raw_scene)}
        self.commit_requests = 0
        self.history_requests = 0
        self.executor_calls = 0
        self.history_executor_calls = 0
        self.redo_stack = []

    def executor(self, project: dict, command: dict):
        self.executor_calls += 1
        self.redo_stack.clear()
        base_revision = self.kernel.current_revision(self.document_id).revision_id
        base_scene = self.scenes[base_revision]
        node = next((item for item in base_scene["nodes"] if item["node_id"] == command["node_id"]), None)
        if node is None:
            raise ValueError("unknown node")
        before = copy.deepcopy(node["bounds"])
        operation = {
            "kind": "move_node",
            "node_id": command["node_id"],
            "before": before,
            "after": {
                "x": command["x_emu"],
                "y": command["y_emu"],
                "width": before["width"],
                "height": before["height"],
            },
        }
        next_project = copy.deepcopy(project)
        next_project["operations"] = list(next_project["operations"]) + [copy.deepcopy(operation)]
        return operation, next_project, []

    def _scene_from_project(self, project: dict, revision_id: str) -> dict:
        scene = copy.deepcopy(self.baseline_scene)
        scene["revision_id"] = revision_id
        for operation in project.get("operations", []):
            if operation.get("kind") != "move_node":
                raise ValueError("synthetic HTTP harness only projects MoveNode operations")
            node = next(
                (item for item in scene["nodes"] if item["node_id"] == operation["node_id"]),
                None,
            )
            if node is None:
                raise ValueError("project references unknown node")
            node["bounds"] = copy.deepcopy(operation["after"])
        scene["snapshot_id"] = scene_snapshot_id(scene)
        return scene

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
        return project, []

    def export_preview(self, target: str) -> dict:
        if target not in {"idml", "odg"}:
            raise ValueError("export preview target must be idml or odg")

        current = self.kernel.current_revision(self.document_id)
        scene = self.scenes[current.revision_id]
        story_frame_nodes = {frame["node_id"] for frame in scene.get("story_frames", [])}
        items = []

        for page in scene.get("pages", []):
            items.append({
                "feature": "page.geometry",
                "origin": page["page_id"],
                "property_path": "page.size",
                "disposition": "preserved",
            })

        for story in scene.get("stories", []):
            items.append({
                "feature": "story.text",
                "origin": story["story_id"],
                "property_path": "story.text",
                "disposition": "preserved",
            })

        for node in scene.get("nodes", []):
            if node["node_id"] in story_frame_nodes:
                items.append({
                    "feature": "story.linked_frames",
                    "origin": node["node_id"],
                    "property_path": "node.story_frame",
                    "disposition": "preserved",
                })
            else:
                items.append({
                    "feature": "node.unsupported",
                    "origin": node["node_id"],
                    "property_path": "node",
                    "disposition": "unsupported",
                    "loss_kind": "unsupported",
                    "severity": "semantic",
                    "reversible": False,
                    "code": "export.unsupported.node.unsupported",
                })

        counts = {
            "preserved": 0,
            "approximated": 0,
            "flattened": 0,
            "rasterized": 0,
            "unsupported": 0,
            "blocking": 0,
        }
        for item in items:
            counts[item["disposition"]] += 1
            if item.get("severity") == "blocking":
                counts["blocking"] += 1

        return {
            "protocol_version": "chaptera.export-preview.v1",
            "report_schema_version": "0.1",
            "document_id": self.document_id,
            "source_hash": self.source_hash,
            "revision_id": current.revision_id,
            "target": {
                "format": target,
                "adapter_version": "idml-v0.1" if target == "idml" else "odg-v0.1",
                "profile": "bounded-editable",
                "schema_fence": (
                    "legacy-spec-8.02/dom-7.0" if target == "idml" else "odf-1.4"
                ),
            },
            "conversion_fence_sha256": None,
            "can_serialize": counts["blocking"] == 0,
            "counts": counts,
            "items": items,
        }

    def commit(self, request: dict, principal_id: str) -> dict:
        self.commit_requests += 1
        protocol = request.get("protocol_version")
        if protocol == "chaptera.commit-request.v1":
            result = self.gateway.commit(
                request,
                principal_id=principal_id,
                executor=self.executor,
            )
        elif protocol == "chaptera.history-transition-intent.v1":
            self.history_requests += 1
            result = self.gateway.commit(
                request,
                principal_id=principal_id,
                history_executor=self.history_executor,
            )
        else:
            raise ValueError("unsupported commit protocol")

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
        return copy.deepcopy(result)


STATE = HarnessState()
RECORDER = TraceRecorder()


class Handler(BaseHTTPRequestHandler):
    server_version = "ChapteraSyntheticHarness/1"

    def _headers(self, status: int = 200, content_type: str = "application/json; charset=utf-8") -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("cache-control", "no-store")
        self.send_header("access-control-allow-origin", "*")
        self.send_header("access-control-allow-methods", "GET, POST, OPTIONS")
        self.send_header(
            "access-control-allow-headers",
            "content-type, x-chaptera-trace-version, x-chaptera-trace-id, "
            "x-chaptera-interaction-id, x-chaptera-session-incarnation, "
            "x-chaptera-operation-class, x-chaptera-browser-family, "
            "x-chaptera-principal-id"
        )
        self.end_headers()

    def _json(self, value, status: int = 200) -> None:
        self._headers(status)
        self.wfile.write(json.dumps(value, ensure_ascii=False, sort_keys=True).encode("utf-8"))

    def _principal_id(self) -> str:
        value = self.headers.get("x-chaptera-principal-id")
        if not value:
            raise AuthzDenied("principal_missing")
        return value

    def _authorize(self, capability: str) -> None:
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
        trace_context = trace_context_from_headers(self.headers)
        if path == "/health":
            self._json({"ok": True, "receipt_class": "synthetic_http_service_plumbing"})
            return
        if path == "/v1/export/preview":
            target = query.get("target", [""])[0]
            try:
                self._authorize(CAP_EXPORT)
                with RECORDER.span("gateway.export_preview", trace_context):
                    preview = STATE.export_preview(target)
                self._json(preview)
            except AuthzDenied as exc:
                self._json({"error": "forbidden", "code": exc.code}, 403)
            except ValueError as exc:
                self._json({"error": "invalid_request", "detail": str(exc)}, 400)
            return
        if path == "/v1/scenes/current":
            try:
                self._authorize(CAP_VIEW)
                with RECORDER.span("gateway.scene_current", trace_context):
                    current = STATE.kernel.current_revision(STATE.document_id).revision_id
                    scene = STATE.scenes[current]
                self._json(scene)
            except AuthzDenied as exc:
                self._json({"error": "forbidden", "code": exc.code}, 403)
            return
        if path.startswith("/v1/observability/traces/"):
            trace_id = path.removeprefix("/v1/observability/traces/")
            self._json(RECORDER.trace_summary(trace_id))
            return
        if path == "/v1/observability/metrics":
            self._json({
                "protocol_version": "chaptera.observability-metrics.v1",
                "metrics": RECORDER.metrics_snapshot(),
            })
            return
        if path.startswith("/v1/scenes/"):
            revision_id = path.removeprefix("/v1/scenes/")
            try:
                self._authorize(CAP_VIEW)
                with RECORDER.span("gateway.scene_revision", trace_context):
                    scene = STATE.scenes.get(revision_id)
                if scene is None:
                    self._json({"error": "scene_not_found"}, 404)
                else:
                    self._json(scene)
            except AuthzDenied as exc:
                self._json({"error": "forbidden", "code": exc.code}, 403)
            return
        if path == "/v1/harness/state":
            current = STATE.kernel.current_revision(STATE.document_id).revision_id
            self._json({
                "receipt_class": "synthetic_http_service_plumbing",
                "real_pub": False,
                "product_acceptance": False,
                "document_id": STATE.document_id,
                "source_hash": STATE.source_hash,
                "current_revision_id": current,
                "commit_requests": STATE.commit_requests,
                "history_requests": STATE.history_requests,
                "executor_calls": STATE.executor_calls,
                "history_executor_calls": STATE.history_executor_calls,
                "authz_version": STATE.authz.authz_version(
                    tenant_id=STATE.tenant_id,
                    document_id=STATE.document_id,
                ),
            })
            return
        self._json({"error": "not_found"}, 404)

    def do_POST(self):
        path = unquote(self.path.split("?", 1)[0])
        trace_context = trace_context_from_headers(self.headers)
        if path != "/v1/commit":
            self._json({"error": "not_found"}, 404)
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            if length <= 0 or length > 1024 * 1024:
                raise ValueError("invalid request size")
            request = json.loads(self.rfile.read(length).decode("utf-8"))
            if trace_context is not None and "client_operation_id" in request:
                trace_context = dict(trace_context)
                trace_context["client_operation_id"] = request["client_operation_id"]
            with RECORDER.span("gateway.commit", trace_context):
                result = STATE.commit(request, self._principal_id())
            self._json(result)
        except AuthzDenied as exc:
            self._json({"error": "forbidden", "code": exc.code}, 403)
        except (ValueError, KeyError, TypeError) as exc:
            self._json({"error": "invalid_request", "detail": str(exc)}, 400)

    def log_message(self, format, *args):
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(json.dumps({
        "ready": True,
        "port": server.server_address[1],
        "receipt_class": "synthetic_http_service_plumbing",
        "real_pub": False,
        "product_acceptance": False,
    }), flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
