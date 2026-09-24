#!/usr/bin/env python3
from __future__ import annotations

import dataclasses
import hashlib
import json
from pathlib import Path
from typing import List

OUT = Path("target/rd-fault-injection/lifecycle-dr.json")
OUT.parent.mkdir(parents=True, exist_ok=True)


@dataclasses.dataclass(frozen=True)
class Lifecycle:
    version: int
    generation: int
    home: str
    state: str
    purged: bool = False


@dataclasses.dataclass(frozen=True)
class Transition:
    from_version: int
    to_version: int
    from_generation: int
    to_generation: int
    home: str
    state: str
    purged: bool = False


class Journal:
    def __init__(self, genesis: Lifecycle):
        self.genesis = genesis
        self.transitions: List[Transition] = []

    def append(self, *, generation: int, home: str, state: str, purged: bool = False):
        current = self.current()
        nxt = Transition(
            from_version=current.version,
            to_version=current.version + 1,
            from_generation=current.generation,
            to_generation=generation,
            home=home,
            state=state,
            purged=purged,
        )
        if nxt.to_version <= current.version:
            raise AssertionError("lifecycle version must increase")
        if generation < current.generation:
            raise AssertionError("generation rollback")
        if current.purged and not purged:
            raise AssertionError("PURGED is terminal")
        self.transitions.append(nxt)

    def current(self) -> Lifecycle:
        cur = self.genesis
        for t in self.transitions:
            if t.from_version != cur.version:
                raise AssertionError("journal version gap")
            if t.from_generation != cur.generation:
                raise AssertionError("journal generation mismatch")
            cur = Lifecycle(t.to_version, t.to_generation, t.home, t.state, t.purged)
        return cur

    def materialize_at(self, version: int) -> Lifecycle:
        cur = self.genesis
        if version == cur.version:
            return cur
        for t in self.transitions:
            cur = Lifecycle(t.to_version, t.to_generation, t.home, t.state, t.purged)
            if cur.version == version:
                return cur
        raise KeyError(version)

    def rebuild(self) -> Lifecycle:
        return self.current()


def reconcile(restored: Lifecycle, journal: Journal) -> dict:
    canonical = journal.current()
    stale = restored.version < canonical.version
    illegal_future = restored.version > canonical.version
    if illegal_future:
        return {"state": "RECOVERING", "reason": "directory_ahead_of_lifecycle_authority"}
    if stale:
        return {
            "state": "RECOVERING",
            "reason": "stale_directory",
            "reconciled": dataclasses.asdict(canonical),
        }
    if dataclasses.asdict(restored) != dataclasses.asdict(canonical):
        return {"state": "RECOVERING", "reason": "same_version_content_mismatch"}
    return {"state": "ACTIVE" if canonical.state == "ACTIVE" and not canonical.purged else canonical.state}


def route_valid(token: dict, journal: Journal, now: int) -> bool:
    current = journal.current()
    if token["expires_at"] < now:
        return False
    if current.purged or current.state != "ACTIVE":
        return False
    if token["generation"] != current.generation:
        return False
    if token["lifecycle_version"] != current.version:
        return False
    if token["home"] != current.home:
        return False
    return True


def issue_route(stale_issuer: Lifecycle, journal: Journal, now: int) -> dict:
    # The issuer may have stale cached state, but must consult monotonic lifecycle truth
    # before minting a capability.
    current = journal.current()
    if stale_issuer.version != current.version or stale_issuer.generation != current.generation:
        return {"issued": False, "reason": "issuer_state_stale"}
    if current.purged or current.state != "ACTIVE":
        return {"issued": False, "reason": "lifecycle_not_active"}
    return {
        "issued": True,
        "token": {
            "generation": current.generation,
            "lifecycle_version": current.version,
            "home": current.home,
            "expires_at": now + 3600,
        },
    }


def directory_aba_and_purge() -> dict:
    g17 = Lifecycle(version=17, generation=17, home="A", state="ACTIVE")
    journal = Journal(g17)
    backup_v17 = g17

    # Planned migration: freeze then activate a new generation/home.
    journal.append(generation=17, home="A", state="FREEZING")  # v18
    journal.append(generation=18, home="B", state="ACTIVE")    # v19
    active_v19 = journal.current()

    stale_reconcile = reconcile(backup_v17, journal)
    assert stale_reconcile["state"] == "RECOVERING"
    assert stale_reconcile["reconciled"]["generation"] == 18

    old_token = {
        "generation": 17,
        "lifecycle_version": 17,
        "home": "A",
        "expires_at": 999999,
    }
    assert not route_valid(old_token, journal, now=100)

    stale_issue = issue_route(backup_v17, journal, now=100)
    assert stale_issue == {"issued": False, "reason": "issuer_state_stale"}

    rebuilt = journal.rebuild()
    assert rebuilt == active_v19

    # Take a content/directory backup while active in B, then hard purge later.
    pre_purge_backup = {
        "directory": dataclasses.asdict(active_v19),
        "content": {
            "document_id": "doc:X",
            "blob_sha256": hashlib.sha256(b"secret-document").hexdigest(),
            "decryptable": True,
        },
    }
    journal.append(generation=18, home="B", state="PURGED", purged=True)  # v20
    purged = journal.current()
    assert purged.purged

    restored_directory = Lifecycle(**pre_purge_backup["directory"])
    purge_reconcile = reconcile(restored_directory, journal)
    assert purge_reconcile["state"] == "RECOVERING"
    assert purge_reconcile["reconciled"]["purged"] is True

    # Serving gate applies current lifecycle truth before restored content can become visible.
    restored_content_accessible = (
        pre_purge_backup["content"]["decryptable"]
        and not journal.current().purged
        and journal.current().state == "ACTIVE"
    )
    assert not restored_content_accessible
    assert not route_valid(
        {
            "generation": 18,
            "lifecycle_version": 19,
            "home": "B",
            "expires_at": 999999,
        },
        journal,
        now=100,
    )
    assert issue_route(active_v19, journal, now=100)["issued"] is False

    # Every older materialized directory snapshot must reconcile to terminal purge.
    stale_snapshots_checked = 0
    for version in (17, 18, 19):
        snap = journal.materialize_at(version)
        res = reconcile(snap, journal)
        assert res["state"] == "RECOVERING"
        assert res["reconciled"]["purged"] is True
        stale_snapshots_checked += 1

    return {
        "migration": {
            "restored_v17_after_v19": stale_reconcile,
            "old_route_token_rejected": True,
            "stale_issuer_rejected": stale_issue,
            "directory_rebuild_exact": rebuilt == active_v19,
        },
        "purge_restore": {
            "backup_version": 19,
            "current_version": purged.version,
            "current_state": purged.state,
            "restored_content_accessible": restored_content_accessible,
            "old_generation_route_rejected": True,
            "stale_snapshots_checked": stale_snapshots_checked,
            "all_stale_snapshots_dominated_by_purge": True,
        },
    }


def dr_old_tail() -> dict:
    source_generation = 17
    source_history = ["R100", "R101", "R102", "R103", "R104"]
    replica_frontier = "R102"
    replicated = source_history[: source_history.index(replica_frontier) + 1]
    acknowledged_not_recovered = source_history[len(replicated):]

    # Promote only the independently proven frontier into a new generation.
    target_generation = 18
    target_history = [replica_frontier, "R200", "R201"]
    recovered_old_tail = acknowledged_not_recovered

    # By contract the recovered old tail is evidence, not automatically ancestry.
    auto_spliced = any(r in target_history for r in recovered_old_tail)
    assert not auto_spliced
    classification = [
        {
            "revision": r,
            "source_generation": source_generation,
            "classification": "RecoveredOldGeneration",
        }
        for r in recovered_old_tail
    ]

    return {
        "source_generation": source_generation,
        "target_generation": target_generation,
        "source_history": source_history,
        "proven_replica_frontier": replica_frontier,
        "acknowledged_but_not_recovered_at_promotion": acknowledged_not_recovered,
        "target_history": target_history,
        "recovered_old_tail": classification,
        "auto_spliced": auto_spliced,
        "loss_window_revision_count": len(acknowledged_not_recovered),
    }


def main():
    lifecycle = directory_aba_and_purge()
    old_tail = dr_old_tail()

    receipt = {
        "receipt_kind": "chaptera.lifecycle-dr-reference-fault-injection.v1",
        "deployed_control_plane": False,
        "real_provider_replication": False,
        "experiments": [
            "EXP-DIRECTORY-ABA-01",
            "EXP-DIRECTORY-TOKEN-01",
            "EXP-DIRECTORY-ISSUER-ROLLBACK-01",
            "EXP-DIRECTORY-REBUILD-01",
            "EXP-DR-PURGE-NORESURRECT-01",
            "EXP-DR-OLDTAIL-01",
        ],
        "lifecycle": lifecycle,
        "dr_old_tail": old_tail,
        "bounded_findings": {
            "stale_directory_can_be_detected_by_monotonic_lifecycle_authority": True,
            "retired_generation_token_rejected_even_before_ttl": True,
            "stale_issuer_cannot_mint_retired_generation_route": True,
            "pre_purge_backup_cannot_resurrect_content_after_reconciliation": True,
            "late_old_generation_tail_is_not_auto_merged": True,
        },
        "guardrail": (
            "Reference-model evidence only. It does not prove a specific database, IAM, KMS, "
            "cross-region replication SLA, or deployed restore runbook."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
