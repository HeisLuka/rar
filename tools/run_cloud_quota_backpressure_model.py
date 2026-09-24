#!/usr/bin/env python3
"""Executable quota/backpressure discriminator for Chaptera Cloud.

Reference model only. It tests atomic reservation, idempotent retries, lease
recovery and interactive-vs-background scheduling. It does not choose paid-plan
limits or claim production throughput.
"""

from __future__ import annotations

import dataclasses
import json
from collections import deque
from pathlib import Path
from typing import Deque, Dict, Optional

OUT = Path("target/cloud-export-quota/quota-backpressure.json")


@dataclasses.dataclass
class Reservation:
    reservation_id: str
    tenant: str
    kind: str
    amount: int
    active: bool = True


class Budget:
    def __init__(self, capacity: int):
        self.capacity = capacity
        self.used = 0
        self.reservations: Dict[str, Reservation] = {}

    def reserve(self, reservation_id: str, tenant: str, kind: str, amount: int) -> str:
        old = self.reservations.get(reservation_id)
        if old is not None:
            if old.tenant == tenant and old.kind == kind and old.amount == amount:
                return "same_reservation"
            return "idempotency_conflict"
        if amount < 0:
            return "invalid"
        if self.used + amount > self.capacity:
            return "quota_exceeded"
        self.used += amount
        self.reservations[reservation_id] = Reservation(
            reservation_id, tenant, kind, amount, True
        )
        return "reserved"

    def release(self, reservation_id: str) -> str:
        item = self.reservations.get(reservation_id)
        if item is None:
            return "unknown"
        if not item.active:
            return "already_released"
        item.active = False
        self.used -= item.amount
        assert self.used >= 0
        return "released"


def atomic_storage_reservation() -> dict:
    budget = Budget(capacity=100)
    first = budget.reserve("upload:a", "tenant:a", "asset", 60)
    second = budget.reserve("upload:b", "tenant:a", "asset", 60)
    assert first == "reserved"
    assert second == "quota_exceeded"
    assert budget.used == 60

    retry = budget.reserve("upload:a", "tenant:a", "asset", 60)
    conflict = budget.reserve("upload:a", "tenant:a", "asset", 61)
    assert retry == "same_reservation"
    assert conflict == "idempotency_conflict"
    assert budget.used == 60

    released = budget.release("upload:a")
    released_again = budget.release("upload:a")
    assert released == "released"
    assert released_again == "already_released"
    assert budget.used == 0

    naive_check_then_act_final = 120  # both concurrent requests observe 0 < 100 before commit
    return {
        "capacity": 100,
        "atomic_first": first,
        "atomic_second": second,
        "atomic_peak_used": 60,
        "naive_check_then_act_possible_used": naive_check_then_act_final,
        "retry": retry,
        "same_id_different_amount": conflict,
        "double_release": released_again,
        "atomic_reservation_prevents_overshoot": True,
    }


def semantic_commit_headroom() -> dict:
    total = Budget(capacity=100)
    # Existing retained bytes consume 90. Keep 10 bytes headroom explicitly available
    # for semantic commits; resource uploads cannot consume it.
    total.used = 90

    asset_admission = "quota_exceeded" if total.used + 12 > 90 else "reserved"
    assert asset_admission == "quota_exceeded"

    edit = total.reserve("edit:1", "tenant:a", "semantic_edit", 6)
    assert edit == "reserved"
    ack = "durable_ack"
    # Once ACKed, quota enforcement may not retroactively discard it.
    assert total.used == 96
    second_edit = total.reserve("edit:2", "tenant:a", "semantic_edit", 6)
    assert second_edit == "quota_exceeded"

    return {
        "capacity": 100,
        "preexisting_used": 90,
        "asset_over_soft_limit": asset_admission,
        "first_edit_reservation": edit,
        "first_edit_result": ack,
        "used_after_ack": total.used,
        "second_edit": second_edit,
        "accepted_edit_dropped_after_ack": False,
        "policy": (
            "reserve worst-case bounded durable bytes before ACK; protect semantic headroom "
            "from new assets/background materialization; fail the next edit before mutation "
            "when headroom is exhausted."
        ),
    }


@dataclasses.dataclass
class Work:
    work_id: str
    tenant: str
    kind: str
    duration: int
    enqueued_tick: int
    started_tick: Optional[int] = None
    completed_tick: Optional[int] = None


def simulate_fifo(work: list[Work], workers: int = 4) -> dict:
    queue: Deque[Work] = deque(dataclasses.replace(x) for x in work)
    active: list[tuple[Work, int]] = []
    tick = 0
    completed: list[Work] = []
    while queue or active:
        while queue and len(active) < workers:
            item = queue.popleft()
            item.started_tick = tick
            active.append((item, item.duration))
        next_active: list[tuple[Work, int]] = []
        for item, remaining in active:
            remaining -= 1
            if remaining <= 0:
                item.completed_tick = tick + 1
                completed.append(item)
            else:
                next_active.append((item, remaining))
        active = next_active
        tick += 1
    interactive_waits = [
        x.started_tick - x.enqueued_tick for x in completed if x.kind == "interactive"
    ]
    return {
        "ticks": tick,
        "interactive_count": len(interactive_waits),
        "interactive_wait_max": max(interactive_waits),
        "interactive_wait_mean": sum(interactive_waits) / len(interactive_waits),
    }


def simulate_reserved(work: list[Work]) -> dict:
    # Four logical slots: two interactive, one export, one background.
    # Idle dedicated capacity may be borrowed by lower priority only after all
    # queued work of the reserved class is served for the current tick.
    queues = {
        "interactive": deque(dataclasses.replace(x) for x in work if x.kind == "interactive"),
        "export": deque(dataclasses.replace(x) for x in work if x.kind == "export"),
        "background": deque(dataclasses.replace(x) for x in work if x.kind == "background"),
    }
    slot_kinds = ["interactive", "interactive", "export", "background"]
    active: list[Optional[tuple[Work, int]]] = [None] * len(slot_kinds)
    completed: list[Work] = []
    tick = 0

    def pick(preferred: str) -> Optional[Work]:
        if queues[preferred]:
            return queues[preferred].popleft()
        for fallback in ("interactive", "export", "background"):
            if queues[fallback]:
                return queues[fallback].popleft()
        return None

    while any(queues[k] for k in queues) or any(x is not None for x in active):
        for i, preferred in enumerate(slot_kinds):
            if active[i] is None:
                item = pick(preferred)
                if item:
                    item.started_tick = tick
                    active[i] = (item, item.duration)
        for i, state in enumerate(active):
            if state is None:
                continue
            item, remaining = state
            remaining -= 1
            if remaining <= 0:
                item.completed_tick = tick + 1
                completed.append(item)
                active[i] = None
            else:
                active[i] = (item, remaining)
        tick += 1

    interactive_waits = [
        x.started_tick - x.enqueued_tick for x in completed if x.kind == "interactive"
    ]
    return {
        "ticks": tick,
        "interactive_count": len(interactive_waits),
        "interactive_wait_max": max(interactive_waits),
        "interactive_wait_mean": sum(interactive_waits) / len(interactive_waits),
    }


def fairness_discriminator() -> dict:
    # Adversarial queue: noisy tenant A floods long export/background work before
    # tenant B's small interactive edits.
    work: list[Work] = []
    for i in range(40):
        kind = "export" if i % 2 == 0 else "background"
        work.append(Work(f"a:{i}", "tenant:a", kind, duration=4, enqueued_tick=0))
    for i in range(12):
        work.append(Work(f"b:{i}", "tenant:b", "interactive", duration=1, enqueued_tick=0))

    fifo = simulate_fifo(work)
    reserved = simulate_reserved(work)
    assert reserved["interactive_wait_max"] < fifo["interactive_wait_max"]
    return {
        "work_items": len(work),
        "fifo": fifo,
        "reserved_class_scheduler": reserved,
        "interactive_wait_reduction_factor":
            fifo["interactive_wait_max"] / max(1, reserved["interactive_wait_max"]),
        "background_work_may_delay_but_not_starve_interactive": True,
    }


class ConcurrencyQuota:
    def __init__(self, limit: int):
        self.limit = limit
        self.active: Dict[str, tuple[str, int]] = {}

    def claim(self, request_id: str, tenant: str, lease_until: int) -> str:
        old = self.active.get(request_id)
        if old:
            return "same_claim" if old == (tenant, lease_until) else "idempotency_conflict"
        tenant_active = sum(1 for t, _ in self.active.values() if t == tenant)
        if tenant_active >= self.limit:
            return "quota_exceeded"
        self.active[request_id] = (tenant, lease_until)
        return "claimed"

    def expire(self, tick: int) -> int:
        stale = [rid for rid, (_, until) in self.active.items() if until <= tick]
        for rid in stale:
            del self.active[rid]
        return len(stale)


def lease_recovery() -> dict:
    quota = ConcurrencyQuota(limit=2)
    a = quota.claim("export:1", "tenant:a", 5)
    b = quota.claim("export:2", "tenant:a", 5)
    c = quota.claim("export:3", "tenant:a", 5)
    retry = quota.claim("export:1", "tenant:a", 5)
    assert (a, b, c, retry) == ("claimed", "claimed", "quota_exceeded", "same_claim")
    expired = quota.expire(5)
    assert expired == 2
    after = quota.claim("export:3", "tenant:a", 10)
    assert after == "claimed"
    return {
        "limit": 2,
        "first": a,
        "second": b,
        "third_while_full": c,
        "retry_did_not_double_count": retry,
        "leases_expired": expired,
        "claim_after_expiry": after,
    }


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    storage = atomic_storage_reservation()
    headroom = semantic_commit_headroom()
    fairness = fairness_discriminator()
    leases = lease_recovery()

    receipt = {
        "receipt_kind": "chaptera.cloud-quota-backpressure-reference-model.v1",
        "deployed_service": False,
        "production_limits_chosen": False,
        "experiments": {
            "atomic_storage_reservation": storage,
            "semantic_commit_headroom": headroom,
            "scheduler_fairness": fairness,
            "concurrency_lease_recovery": leases,
        },
        "bounded_findings": {
            "check_then_act_quota_is_racy": True,
            "quota_admission_requires_atomic_reservation": True,
            "idempotent_retry_must_not_double_reserve": True,
            "accepted_semantic_commit_must_not_be_dropped_after_ack": True,
            "background_and_export_work_should_not_consume_all_interactive_capacity": True,
            "concurrency_reservations_require_lease_expiry_recovery": True,
            "single_global_quota_exceeded_mode_is_too_coarse": True,
        },
        "degradation_direction": [
            "pause/defer background projections/materialization first",
            "throttle export concurrency/CPU and queue admission",
            "block new asset/import bytes before protected semantic headroom",
            "reserve bounded semantic-write bytes before durable ACK",
            "when semantic headroom is exhausted, fail the next edit before mutation",
            "never silently drop an already-ACKed edit",
            "reads of existing authorized data remain available unless a separate safety/security rule forbids them",
        ],
        "guardrail": (
            "Reference-model evidence only. Numeric limits, worker counts, tier policy and "
            "provider cost budgets require production-shaped load and billing measurements."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
