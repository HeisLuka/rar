"""Fixed-site copy/materialization counters for hosted and local ledger runs.

The hot-path recorder owns a fixed-size counter bank. Recording never appends an
event, builds a dict, serializes JSON, or captures source bytes. Receipt/event
objects are reconstructed only when a scenario snapshot is requested.
"""

from __future__ import annotations

import json
from array import array
from dataclasses import dataclass
from typing import Any

SITE_NAMES = (
    "revision.move_nodes_request_normalize",
    "revision.executor_project_detach",
    "revision.retained_project_snapshot",
    "revision.idempotency_result_snapshot",
)

SITE_INDEX = {name: index for index, name in enumerate(SITE_NAMES)}


@dataclass(frozen=True)
class FixedSiteSnapshotV1:
    site: str
    materialized_bytes: int
    instances: int
    allocation_count: int
    retained_bytes_after: int


class FixedCopyCountersV1:
    """Preallocated fixed-site counters.

    Python integer arithmetic itself is an interpreter concern, but the
    recorder performs no per-record container/event allocation: storage is
    allocated once and indexed by a fixed site id.
    """

    __slots__ = ("_bytes", "_instances", "_allocations", "_retained")

    def __init__(self) -> None:
        count = len(SITE_NAMES)
        self._bytes = array("Q", [0]) * count
        self._instances = array("Q", [0]) * count
        self._allocations = array("Q", [0]) * count
        self._retained = array("Q", [0]) * count

    def reset(self) -> None:
        for bank in (
            self._bytes,
            self._instances,
            self._allocations,
            self._retained,
        ):
            for index in range(len(bank)):
                bank[index] = 0

    def record(
        self,
        site: str,
        *,
        materialized_bytes: int,
        instances: int = 1,
        allocation_count: int = 1,
        retained_bytes_after: int = 0,
    ) -> None:
        index = SITE_INDEX[site]
        if min(
            materialized_bytes,
            instances,
            allocation_count,
            retained_bytes_after,
        ) < 0:
            raise ValueError("copy-ledger counters must be non-negative")
        self._bytes[index] += materialized_bytes
        self._instances[index] += instances
        self._allocations[index] += allocation_count
        self._retained[index] += retained_bytes_after

    def snapshot(self) -> tuple[FixedSiteSnapshotV1, ...]:
        return tuple(
            FixedSiteSnapshotV1(
                site=name,
                materialized_bytes=int(self._bytes[index]),
                instances=int(self._instances[index]),
                allocation_count=int(self._allocations[index]),
                retained_bytes_after=int(self._retained[index]),
            )
            for index, name in enumerate(SITE_NAMES)
        )


def canonical_byte_len(value: Any) -> int:
    """Stable source-neutral byte equivalent used only in instrumentation mode.

    The revision kernel already defines canonical JSON as its identity boundary.
    Using the same representation keeps repeat attribution deterministic. This
    helper is not called when instrumentation is disabled.
    """

    return len(
        json.dumps(
            value,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    )
