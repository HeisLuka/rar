#!/usr/bin/env python3
from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Dict, Optional

OUT = Path("target/rd-fault-injection/revisionstream.json")
OUT.parent.mkdir(parents=True, exist_ok=True)


def h(value) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(raw).hexdigest()


@dataclass(frozen=True)
class Edge:
    parent: str
    child: str
    operation_id: str
    intent_hash: str
    delta: int

    @staticmethod
    def build(parent: str, operation_id: str, intent_hash: str, delta: int) -> "Edge":
        event = {
            "schema": "chaptera.synthetic-event.v1",
            "operation_id": operation_id,
            "intent_hash": intent_hash,
            "delta": delta,
        }
        child = h({"parent": parent, "event": event})
        return Edge(parent, child, operation_id, intent_hash, delta)

    def valid(self) -> bool:
        return self.child == Edge.build(
            self.parent, self.operation_id, self.intent_hash, self.delta
        ).child


class Stream:
    def __init__(self):
        self.edges: Dict[str, Edge] = {}

    def create(self, edge: Edge) -> str:
        old = self.edges.get(edge.parent)
        if old is None:
            self.edges[edge.parent] = edge
            return "Created"
        if old.operation_id == edge.operation_id and old.intent_hash == edge.intent_hash:
            if old != edge:
                return "InvariantMismatch"
            return "AlreadyCreatedSame"
        if old.operation_id == edge.operation_id and old.intent_hash != edge.intent_hash:
            return "IdempotencyConflict"
        return "ParentOccupied"

    def successor(self, parent: str) -> Optional[Edge]:
        return self.edges.get(parent)


def genesis() -> str:
    return h({"genesis": "revisionstream-v2-fault-injection"})


def intent(op_id: str, observed: str, delta: int) -> str:
    return h({"operation_id": op_id, "observed": observed, "delta": delta, "semantics": 1})


def append_other(stream: Stream, parent: str, index: int) -> str:
    e = Edge.build(parent, f"other-{index}", h({"other": index}), 1)
    assert stream.create(e) == "Created"
    return e.child


def scan_for_operation(
    stream: Stream, causal_base: str, operation_id: str, expected_intent: str, max_edges: int
):
    cursor = causal_base
    reads = 0
    for _ in range(max_edges):
        edge = stream.successor(cursor)
        reads += 1
        if edge is None:
            return {"status": "not_found", "reads": reads, "last_revision": cursor}
        if not edge.valid():
            return {"status": "corrupt", "reads": reads, "last_revision": cursor}
        if edge.operation_id == operation_id:
            if edge.intent_hash == expected_intent:
                return {
                    "status": "found",
                    "reads": reads,
                    "revision": edge.child,
                    "last_revision": edge.child,
                }
            return {"status": "idempotency_conflict", "reads": reads}
        cursor = edge.child
    return {"status": "horizon_exhausted", "reads": reads, "last_revision": cursor}


def rebase_case(lag: int, k: int, post_accept_tail: int = 2000) -> dict:
    stream = Stream()
    observed = genesis()
    current = observed
    for i in range(lag):
        current = append_other(stream, current, i)

    op_id = f"op-rebase-{lag}-{k}"
    ih = intent(op_id, observed, 7)

    if lag > k:
        before_edges = len(stream.edges)
        lookup = scan_for_operation(stream, observed, op_id, ih, k + 1)
        assert lookup["status"] in {"not_found", "horizon_exhausted"}
        assert len(stream.edges) == before_edges
        return {
            "lag": lag,
            "max_online_rebase_edges": k,
            "accepted": False,
            "reason": "NeedsRefresh",
            "retry_scan_reads": lookup["reads"],
        }

    accepted = Edge.build(current, op_id, ih, 7)
    assert stream.create(accepted) == "Created"  # ACK is now intentionally lost.

    retry = scan_for_operation(stream, observed, op_id, ih, k + 1)
    assert retry["status"] == "found"
    assert retry["revision"] == accepted.child
    assert retry["reads"] == lag + 1

    conflict = scan_for_operation(
        stream, observed, op_id, intent(op_id, observed, 8), k + 1
    )
    assert conflict["status"] == "idempotency_conflict"

    # Advance the document arbitrarily far after acceptance. The retry lookup remains
    # bounded by the original causal base and K, not by the future current head.
    future = accepted.child
    for i in range(post_accept_tail):
        future = append_other(stream, future, 100000 + i)
    late_retry = scan_for_operation(stream, observed, op_id, ih, k + 1)
    assert late_retry["status"] == "found"
    assert late_retry["reads"] == lag + 1

    return {
        "lag": lag,
        "max_online_rebase_edges": k,
        "accepted": True,
        "accepted_revision": accepted.child,
        "retry_scan_reads": retry["reads"],
        "late_retry_scan_reads_after_future_edges": late_retry["reads"],
        "future_edges_after_accept": post_accept_tail,
        "same_id_different_intent": conflict["status"],
    }


def recovery(stream: Stream, snapshot_revision: str, snapshot_state: int, expected_tail: int):
    cursor = snapshot_revision
    state = snapshot_state
    replayed = 0
    while replayed < expected_tail:
        edge = stream.successor(cursor)
        if edge is None:
            raise RuntimeError(f"missing_tail_at:{cursor}")
        if not edge.valid():
            raise RuntimeError(f"corrupt_tail_at:{cursor}")
        state += edge.delta
        cursor = edge.child
        replayed += 1
    return cursor, state


def wal_collapse_case() -> dict:
    stream = Stream()
    g = genesis()
    current = g
    state = 0
    states = {g: state}
    revisions = [g]

    for i in range(1000):
        delta = (i % 7) - 3
        op_id = f"hist-{i}"
        e = Edge.build(current, op_id, h({"hist": i}), delta)
        assert stream.create(e) == "Created"
        state += delta
        current = e.child
        states[current] = state
        revisions.append(current)

    checkpoint_index = 700
    checkpoint_revision = revisions[checkpoint_index]
    checkpoint_state = states[checkpoint_revision]
    recovered_revision, recovered_state = recovery(
        stream,
        checkpoint_revision,
        checkpoint_state,
        1000 - checkpoint_index,
    )
    assert recovered_revision == current
    assert recovered_state == state

    # Crash before durable: no edge exists; retry may create it.
    parent_before = current
    op_before = "crash-before"
    ih_before = h({"case": op_before})
    candidate = Edge.build(parent_before, op_before, ih_before, 11)
    assert stream.successor(parent_before) is None
    assert stream.create(candidate) == "Created"

    # Crash after durable before ACK: retry resolves to exactly the stored edge.
    parent_after = candidate.child
    op_after = "crash-after"
    ih_after = h({"case": op_after})
    durable = Edge.build(parent_after, op_after, ih_after, 13)
    assert stream.create(durable) == "Created"
    retry_outcome = stream.create(copy.deepcopy(durable))
    assert retry_outcome == "AlreadyCreatedSame"

    # Same-parent race: exactly one winner.
    race_parent = durable.child
    winner = Edge.build(race_parent, "race-a", h({"race": "a"}), 1)
    loser = Edge.build(race_parent, "race-b", h({"race": "b"}), 2)
    assert stream.create(winner) == "Created"
    assert stream.create(loser) == "ParentOccupied"
    assert stream.successor(race_parent) == winner

    # Missing-tail recovery must fail closed.
    broken = Stream()
    broken.edges = copy.deepcopy(stream.edges)
    missing_parent = revisions[850]
    removed = broken.edges.pop(missing_parent)
    missing_failed = False
    try:
        recovery(
            broken,
            checkpoint_revision,
            checkpoint_state,
            1000 - checkpoint_index,
        )
    except RuntimeError as exc:
        missing_failed = str(exc).startswith("missing_tail_at:")
    assert missing_failed
    broken.edges[missing_parent] = removed

    # Corruption must fail closed.
    corrupt_parent = revisions[900]
    original = broken.edges[corrupt_parent]
    broken.edges[corrupt_parent] = Edge(
        parent=original.parent,
        child="sha256:" + "0" * 64,
        operation_id=original.operation_id,
        intent_hash=original.intent_hash,
        delta=original.delta,
    )
    corrupt_failed = False
    try:
        recovery(
            broken,
            checkpoint_revision,
            checkpoint_state,
            1000 - checkpoint_index,
        )
    except RuntimeError as exc:
        corrupt_failed = str(exc).startswith("corrupt_tail_at:")
    assert corrupt_failed

    return {
        "history_edges": 1000,
        "checkpoint_index": checkpoint_index,
        "tail_replayed": 1000 - checkpoint_index,
        "exact_recovery": True,
        "crash_before_durable_retry_created": True,
        "crash_after_durable_before_ack_retry": retry_outcome,
        "same_parent_race": {
            "winner": winner.operation_id,
            "loser_outcome": "ParentOccupied",
            "successor_count": 1,
        },
        "missing_tail_failed_closed": missing_failed,
        "corrupt_tail_failed_closed": corrupt_failed,
        "second_canonical_wal_required": False,
    }


def main():
    rebase = []
    for k in (1, 10, 32, 100, 1000):
        for lag in (1, 10, 100, 1000):
            rebase.append(rebase_case(lag, k))
    wal = wal_collapse_case()

    accepted = [x for x in rebase if x["accepted"]]
    rejected = [x for x in rebase if not x["accepted"]]
    assert all(x["retry_scan_reads"] <= x["max_online_rebase_edges"] + 1 for x in accepted)
    assert all(x["lag"] > x["max_online_rebase_edges"] for x in rejected)

    receipt = {
        "receipt_kind": "chaptera.revisionstream-v2-reference-fault-injection.v1",
        "deployed_backend": False,
        "canonical_private_core": False,
        "experiments": [
            "EXP-REBASE-IDEMPOTENCY-01",
            "EXP-STREAM-OUTCOME-01",
            "EXP-WAL-COLLAPSE-01",
        ],
        "rebase_matrix": rebase,
        "wal_collapse": wal,
        "bounded_findings": {
            "accepted_retry_lookup_bound": "lag + 1 reads, capped by K + 1",
            "future_head_distance_affects_retry_lookup": False,
            "same_id_different_intent_fails_closed": True,
            "successor_edge_can_satisfy_reference_wal_recovery": True,
            "separate_canonical_wal_needed_by_reference_model": False,
        },
        "guardrail": (
            "Reference-model evidence only. It does not prove provider atomic conditional-create "
            "semantics, network latency, private EditorSession determinism, or production durability."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
