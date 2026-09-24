#!/usr/bin/env python3
from __future__ import annotations

import gzip
import hashlib
import json
import statistics
import time
from pathlib import Path

OUT = Path("target/rd-fault-injection/text-delta.json")
OUT.parent.mkdir(parents=True, exist_ok=True)

PATTERN = "abcd😀e\u0301Ж漢"
INSERTED = "Ω👨\u200d👩\u200d👧\u200d👦"


def canonical_bytes(value) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def story(n_scalars: int) -> str:
    repeated = (PATTERN * ((n_scalars // len(PATTERN)) + 1))[:n_scalars]
    assert len(repeated) == n_scalars
    return repeated


def apply_delta(before: str, start: int, end: int, inserted: str) -> str:
    return before[:start] + inserted + before[end:]


def inverse_delta(after: str, start: int, inserted: str, removed: str) -> str:
    return after[:start] + removed + after[start + len(inserted):]


def update_spans(spans, start: int, end: int, inserted_len: int):
    delta = inserted_len - (end - start)
    out = []
    for span in spans:
        a, b, label = span
        if b <= start:
            out.append((a, b, label))
        elif a >= end:
            out.append((a + delta, b + delta, label))
        else:
            # Bounded reference policy: the overlapping part is explicitly marked
            # affected instead of pretending an unchanged provenance range.
            new_a = min(a, start)
            new_b = max(start + inserted_len, start)
            out.append((new_a, new_b, label + ":affected"))
    return out


def build_spans(length: int):
    spans = []
    step = max(64, length // 50)
    width = max(8, min(128, step // 2))
    i = 0
    while i + width <= length:
        spans.append((i, i + width, f"span-{i}"))
        i += step
    return spans


def timed_encode(value, iterations: int):
    samples = []
    last = None
    for _ in range(iterations):
        t0 = time.perf_counter()
        raw = canonical_bytes(value)
        hashlib.sha256(raw).digest()
        samples.append((time.perf_counter() - t0) * 1000)
        last = raw
    return {
        "iterations": iterations,
        "p50_ms": statistics.median(samples),
        "p95_ms": sorted(samples)[min(len(samples) - 1, int(len(samples) * 0.95))],
        "max_ms": max(samples),
        "bytes": len(last),
        "gzip_bytes": len(gzip.compress(last, compresslevel=9)),
    }


def make_case(size: int, position: str, mode: str):
    before = story(size)
    if position == "start":
        anchor = 3
    elif position == "middle":
        anchor = size // 2
    elif position == "end":
        anchor = size - 6
    else:
        raise AssertionError(position)

    if mode == "insert":
        start = end = anchor
        inserted = INSERTED
    elif mode == "delete":
        start = anchor
        end = anchor + 3
        inserted = ""
    else:
        raise AssertionError(mode)

    removed = before[start:end]
    after = apply_delta(before, start, end, inserted)
    assert inverse_delta(after, start, inserted, removed) == before

    spans = build_spans(size)
    updated_spans = update_spans(spans, start, end, len(inserted))
    assert all(0 <= a <= b <= len(after) for a, b, _ in updated_spans)

    # Unaffected spans before and after the edit preserve exact underlying text.
    shift = len(inserted) - (end - start)
    for a, b, label in spans:
        if b <= start:
            matching = next(s for s in updated_spans if s[2] == label)
            assert before[a:b] == after[matching[0]:matching[1]]
        elif a >= end:
            matching = next(s for s in updated_spans if s[2] == label)
            assert matching[0] == a + shift and matching[1] == b + shift
            assert before[a:b] == after[matching[0]:matching[1]]

    whole_event = {
        "schema": "chaptera.synthetic-whole-story-event.v1",
        "story_id": "story:test",
        "before": before,
        "after": after,
    }
    delta_event = {
        "schema": "chaptera.synthetic-story-range-delta.v1",
        "story_id": "story:test",
        "scalar_range": [start, end],
        "removed_text": removed,
        "inserted_text": inserted,
        "resulting_story_hash": hashlib.sha256(after.encode("utf-8")).hexdigest(),
    }

    iterations = 5 if size >= 2_000_000 else (20 if size >= 100_000 else 100)
    whole = timed_encode(whole_event, iterations)
    delta = timed_encode(delta_event, iterations)

    return {
        "story_scalars": size,
        "story_utf8_bytes": len(before.encode("utf-8")),
        "position": position,
        "mode": mode,
        "start_scalar": start,
        "end_scalar": end,
        "removed_scalars": len(removed),
        "inserted_scalars": len(inserted),
        "undo_roundtrip_exact": True,
        "unaffected_span_text_exact": True,
        "whole_event": whole,
        "delta_event": delta,
        "raw_size_ratio_whole_over_delta": whole["bytes"] / delta["bytes"],
        "gzip_size_ratio_whole_over_delta": whole["gzip_bytes"] / delta["gzip_bytes"],
        "encode_hash_p50_ratio_whole_over_delta": whole["p50_ms"] / max(delta["p50_ms"], 1e-9),
    }


def main():
    cases = []
    for size in (1_000, 100_000, 2_000_000):
        for position in ("start", "middle", "end"):
            for mode in ("insert", "delete"):
                cases.append(make_case(size, position, mode))

    large = [c for c in cases if c["story_scalars"] == 2_000_000]
    assert all(c["raw_size_ratio_whole_over_delta"] > 1000 for c in large)
    assert all(c["undo_roundtrip_exact"] for c in cases)
    assert all(c["unaffected_span_text_exact"] for c in cases)

    receipt = {
        "receipt_kind": "chaptera.story-range-delta-scale-reference.v1",
        "canonical_private_story_model": False,
        "experiments": ["EXP-TEXT-DELTA-01"],
        "cases": cases,
        "bounded_findings": {
            "whole_story_event_payload_is_O_story_size": True,
            "range_delta_payload_is_O_changed_slice_plus_fixed_metadata": True,
            "unicode_scalar_apply_and_inverse_exact_in_reference_model": True,
            "unaffected_provenance_spans_preserved_in_reference_model": True,
            "whole_story_history_fails_scale_gate_for_tiny_edits": True,
        },
        "guardrail": (
            "Synthetic scalar/span model only. This does not freeze the production Story delta schema, "
            "style-span conflict policy, grapheme UX, canonical codec, or private core implementation."
        ),
    }
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
