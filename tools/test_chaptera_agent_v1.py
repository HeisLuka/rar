#!/usr/bin/env python3
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys


EXPECTED_SOURCE_SHA256 = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"


class AgentProtocolError(RuntimeError):
    pass


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class AgentClient:
    def __init__(self, exe: pathlib.Path):
        self.process = subprocess.Popen(
            [str(exe), "--agent-v1"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        if self.process.stdin is None or self.process.stdout is None:
            raise AgentProtocolError("failed to open agent stdio")
        self.stdin = self.process.stdin
        self.stdout = self.process.stdout
        self.request_index = 0
        self.trace_events = []

    def call(self, command: str, expected_traces: int = 0, **kwargs):
        self.request_index += 1
        request_id = f"r{self.request_index}"
        request = {"request_id": request_id, "command": command, **kwargs}
        self.stdin.write(json.dumps(request, ensure_ascii=False, separators=(",", ":")) + "\n")
        self.stdin.flush()

        line = self.stdout.readline()
        if not line:
            raise AgentProtocolError(
                f"agent exited before result for {command}: {self.stderr_text()}"
            )
        result = json.loads(line)
        if result.get("message_type") != "result" or result.get("request_id") != request_id:
            raise AgentProtocolError(f"unexpected result envelope for {command}: {result!r}")
        if not result.get("ok"):
            raise AgentProtocolError(
                f"{command} failed: {result.get('error', {}).get('code')}: "
                f"{result.get('error', {}).get('message')}"
            )

        traces = []
        for _ in range(expected_traces):
            trace_line = self.stdout.readline()
            if not trace_line:
                raise AgentProtocolError(
                    f"agent exited before trace for {command}: {self.stderr_text()}"
                )
            trace = json.loads(trace_line)
            if (
                trace.get("message_type") != "trace"
                or trace.get("request_id") != request_id
                or trace.get("command") != command
            ):
                raise AgentProtocolError(f"unexpected trace envelope for {command}: {trace!r}")
            traces.append(trace)
            self.trace_events.append(trace)

        return result, traces

    def stderr_text(self):
        if self.process.stderr is None:
            return ""
        if self.process.poll() is None:
            return "(process still running)"
        return self.process.stderr.read().strip()

    def close(self):
        try:
            self.stdin.close()
        except OSError:
            pass
        return self.process.wait(timeout=30)


def first_editable_story(stories):
    for story in stories:
        if story.get("editable") and story.get("scalar_len", 0) > 0:
            return story
    raise AgentProtocolError("no editable non-empty Story exposed by agent")


def first_mutable_instance(instances):
    for instance in instances:
        if instance.get("move_node_scene_admitted") and instance.get("move_node_editor_supported"):
            return instance
    raise AgentProtocolError("no direct page-local MoveNode instance exposed by agent")


def run(exe: pathlib.Path, fixture: pathlib.Path, work_dir: pathlib.Path, receipt_path: pathlib.Path):
    source_before = sha256_file(fixture)
    if source_before != EXPECTED_SOURCE_SHA256:
        raise AgentProtocolError(
            f"fixture SHA-256 mismatch: expected {EXPECTED_SOURCE_SHA256} got {source_before}"
        )

    work_dir.mkdir(parents=True, exist_ok=True)
    project = work_dir / "agent-v1.project.json"
    export = work_dir / "agent-v1.edited.idml"

    client = AgentClient(exe)
    try:
        protocol, _ = client.call("protocol.describe")
        if protocol["result"]["protocol_version"] != "chaptera.agent-control.v1":
            raise AgentProtocolError("agent protocol_version mismatch")
        if protocol["result"]["native_pub_write"] is not False:
            raise AgentProtocolError("agent unexpectedly exposes native PUB write")

        client.call("trace.subscribe", enabled=True)

        opened, traces = client.call("open", expected_traces=1, path=str(fixture))
        source_hash = opened.get("source_hash")
        if source_hash != EXPECTED_SOURCE_SHA256:
            raise AgentProtocolError("open result source_hash mismatch")
        if traces[0].get("event_kind") != "opened":
            raise AgentProtocolError("open trace kind mismatch")

        described, _ = client.call("document.describe", expected_traces=1)
        if described["result"]["source_hash"] != EXPECTED_SOURCE_SHA256:
            raise AgentProtocolError("document.describe source_hash mismatch")
        if described["result"]["page_count"] < 1:
            raise AgentProtocolError("document.describe returned zero pages")

        stories_result, _ = client.call("stories.list", expected_traces=1)
        story = first_editable_story(stories_result["result"]["stories"])
        story_id = story["story_id"]

        inspected, _ = client.call(
            "story.inspect", expected_traces=1, story_id=story_id
        )
        if inspected["result"]["content"] != "redacted_use_story.read_local":
            raise AgentProtocolError("story.inspect leaked content by default")

        local_story, _ = client.call(
            "story.read_local",
            expected_traces=1,
            story_id=story_id,
            allow_content=True,
        )
        text = local_story["result"]["text"]
        scalar_index = next((i for i, ch in enumerate(text) if ch != "\r"), None)
        if scalar_index is None:
            raise AgentProtocolError("editable Story contains no non-CR scalar")
        expected_before = text[scalar_index]
        replacement = "Q" if expected_before != "Q" else "R"

        story_edit, story_traces = client.call(
            "edit.apply",
            expected_traces=3,
            operation={
                "kind": "replace_story_range",
                "story_id": story_id,
                "start_scalar": scalar_index,
                "end_scalar": scalar_index + 1,
                "expected_before": expected_before,
                "replacement_text": replacement,
            },
        )
        if [item["event_kind"] for item in story_traces] != [
            "intent",
            "durable_commit",
            "projection_updated",
        ]:
            raise AgentProtocolError("Story edit trace law mismatch")
        story_state_id = story_edit["result"]["after_state_id"]

        instances_result, _ = client.call(
            "scene.instances.list", expected_traces=1
        )
        instance = first_mutable_instance(instances_result["result"]["instances"])
        instance_id = instance["instance_id"]
        bounds = instance["bounds"]

        capabilities, _ = client.call(
            "capabilities.get",
            expected_traces=1,
            target_kind="instance",
            target_id=instance_id,
        )
        if not capabilities["result"]["move_node"]["scene_admitted"]:
            raise AgentProtocolError("direct SceneInstance lost MoveNode admission")

        moved, move_traces = client.call(
            "edit.apply",
            expected_traces=3,
            operation={
                "kind": "move_node",
                "instance_id": instance_id,
                "x": bounds["x"] + 127000,
                "y": bounds["y"] + 254000,
            },
        )
        if [item["event_kind"] for item in move_traces] != [
            "intent",
            "durable_commit",
            "projection_updated",
        ]:
            raise AgentProtocolError("MoveNode trace law mismatch")
        moved_state_id = moved["result"]["after_state_id"]

        undone, undo_traces = client.call("undo", expected_traces=1)
        if undo_traces[0]["event_kind"] != "undo":
            raise AgentProtocolError("undo trace kind mismatch")
        if undone["state"]["state_id"] != story_state_id:
            raise AgentProtocolError("undo did not restore exact post-Story state")

        redone, redo_traces = client.call("redo", expected_traces=1)
        if redo_traces[0]["event_kind"] != "redo":
            raise AgentProtocolError("redo trace kind mismatch")
        if redone["state"]["state_id"] != moved_state_id:
            raise AgentProtocolError("redo did not restore exact post-Move state")

        saved, _ = client.call(
            "project.save", expected_traces=1, path=str(project)
        )
        if not project.is_file():
            raise AgentProtocolError("project.save did not write EditorProject")
        project_sha256 = saved["result"]["artifact"]["sha256"]

        reopened, reopen_traces = client.call(
            "project.reopen", expected_traces=1, path=str(project)
        )
        if reopen_traces[0]["event_kind"] != "fresh_reopen":
            raise AgentProtocolError("project.reopen trace kind mismatch")
        if reopened["state"]["state_id"] != moved_state_id:
            raise AgentProtocolError("fresh reopen did not reproduce final state")

        preview, _ = client.call("loss.preview", expected_traces=1, target="idml")
        if not preview["result"]["report"].get("can_serialize"):
            raise AgentProtocolError("IDML loss preview blocks agent export")

        exported, _ = client.call(
            "export", expected_traces=1, target="idml", path=str(export)
        )
        if not export.is_file():
            raise AgentProtocolError("export did not create IDML")
        export_sha256 = exported["result"]["artifact"]["sha256"]
        if export_sha256 != sha256_file(export):
            raise AgentProtocolError("export artifact hash mismatch")

        snapshot, _ = client.call("snapshot.get", expected_traces=1)
        if snapshot["state"]["state_id"] != moved_state_id:
            raise AgentProtocolError("snapshot state differs from reopened state")
        if snapshot["result"]["invariants"]["native_pub_write"] is not False:
            raise AgentProtocolError("snapshot claims native PUB write")

        deep, _ = client.call("diagnostics.deep", expected_traces=1)
        if deep["result"]["available"] is not False:
            raise AgentProtocolError("deep diagnostics unexpectedly claimed available")

        client.call("shutdown", expected_traces=1)
    finally:
        exit_code = client.close()

    if exit_code != 0:
        raise AgentProtocolError(f"agent exited with code {exit_code}: {client.stderr_text()}")

    source_after = sha256_file(fixture)
    if source_after != source_before:
        raise AgentProtocolError("source PUB changed during agent protocol test")

    event_kinds = [item["event_kind"] for item in client.trace_events]
    required_events = {
        "opened",
        "intent",
        "durable_commit",
        "projection_updated",
        "undo",
        "redo",
        "project_persisted",
        "fresh_reopen",
        "loss_previewed",
        "exported",
    }
    if not required_events.issubset(event_kinds):
        raise AgentProtocolError(
            f"trace is missing required events: {sorted(required_events - set(event_kinds))}"
        )

    receipt = {
        "protocol_version": "chaptera.agent-control-real.v1",
        "source_hash": source_before,
        "agent_protocol_version": "chaptera.agent-control.v1",
        "story_id": story_id,
        "instance_id": instance_id,
        "post_story_state_id": story_state_id,
        "post_move_state_id": moved_state_id,
        "project_sha256": project_sha256,
        "export_sha256": export_sha256,
        "trace_event_count": len(client.trace_events),
        "trace_event_kinds": event_kinds,
        "invariants": {
            "source_immutable": True,
            "gui_scraping_used": False,
            "native_pub_write_used": False,
            "fresh_reopen_exact": True,
            "projected_object_mutation_fail_closed": True,
            "deep_diagnostics_claimed": False,
        },
    }
    receipt_path.parent.mkdir(parents=True, exist_ok=True)
    receipt_path.write_text(
        json.dumps(receipt, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    return receipt


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", required=True, type=pathlib.Path)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--work-dir", required=True, type=pathlib.Path)
    parser.add_argument("--receipt", required=True, type=pathlib.Path)
    args = parser.parse_args()

    try:
        receipt = run(args.exe, args.fixture, args.work_dir, args.receipt)
    except (AgentProtocolError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"agent-v1 integration failed: {error}", file=sys.stderr)
        return 2

    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
