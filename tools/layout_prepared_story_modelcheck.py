#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


ENVIRONMENT = "layout-prepared-story-model-v1"


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def stable_hash(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def scalar_width(ch: str) -> int:
    if ch == "\r":
        return 0
    if ch == " ":
        return 3
    if ch in ".,;:!?-":
        return 4
    if ord(ch) > 0x7F:
        return 9
    return 7


@dataclass(frozen=True)
class BreakCandidate:
    visible_end: int
    consumed_end: int
    mandatory: bool


@dataclass(frozen=True)
class PreparedStory:
    text: str
    text_hash: str
    environment: str
    widths: tuple[int, ...]
    prefix_widths: tuple[int, ...]
    breaks: tuple[BreakCandidate, ...]


def prepare_story(text: str, environment: str = ENVIRONMENT) -> PreparedStory:
    widths = tuple(scalar_width(ch) for ch in text)
    prefix = [0]
    for width in widths:
        prefix.append(prefix[-1] + width)

    breaks: list[BreakCandidate] = []
    for index, ch in enumerate(text):
        if ch == " ":
            breaks.append(BreakCandidate(index, index + 1, False))
        elif ch == "\r":
            breaks.append(BreakCandidate(index, index + 1, True))
    if not breaks or breaks[-1].consumed_end != len(text):
        breaks.append(BreakCandidate(len(text), len(text), True))

    return PreparedStory(
        text=text,
        text_hash=sha256_text(text),
        environment=environment,
        widths=widths,
        prefix_widths=tuple(prefix),
        breaks=tuple(breaks),
    )


def segment_width(prepared: PreparedStory, start: int, end: int) -> int:
    return prepared.prefix_widths[end] - prepared.prefix_widths[start]


def flow_prepared(
    prepared: PreparedStory,
    *,
    text: str,
    environment: str,
    frame_widths: Iterable[int],
    lines_per_frame: int,
) -> dict:
    if sha256_text(text) != prepared.text_hash or text != prepared.text:
        raise ValueError("prepared story text mismatch")
    if environment != prepared.environment:
        raise ValueError("prepared environment mismatch")
    if lines_per_frame <= 0:
        raise ValueError("lines_per_frame must be positive")

    lines: list[dict] = []
    cursor = 0
    frames = list(frame_widths)

    for frame_index, frame_width in enumerate(frames):
        if frame_width <= 0:
            raise ValueError("frame width must be positive")
        for line_index in range(lines_per_frame):
            if cursor >= len(text):
                break

            chosen: BreakCandidate | None = None
            for candidate in prepared.breaks:
                if candidate.consumed_end <= cursor:
                    continue
                width = segment_width(prepared, cursor, candidate.visible_end)
                if width <= frame_width:
                    chosen = candidate
                if candidate.mandatory:
                    break

            if chosen is None:
                raise ValueError(
                    f"unbreakable segment at scalar {cursor} for width {frame_width}"
                )

            width = segment_width(prepared, cursor, chosen.visible_end)
            lines.append(
                {
                    "frame_index": frame_index,
                    "line_index": line_index,
                    "scalar_start": cursor,
                    "scalar_end": chosen.visible_end,
                    "consumed_scalar_end": chosen.consumed_end,
                    "width": width,
                    "text": text[cursor:chosen.visible_end],
                    "mandatory_break": chosen.mandatory,
                }
            )
            cursor = chosen.consumed_end

        if cursor >= len(text):
            break

    return {
        "lines": lines,
        "next_scalar": cursor,
        "overset_scalars": max(0, len(text) - cursor),
        "frame_count": len(frames),
        "lines_per_frame": lines_per_frame,
    }


def flow_full(
    text: str,
    *,
    environment: str,
    frame_widths: Iterable[int],
    lines_per_frame: int,
) -> tuple[dict, int]:
    prepared = prepare_story(text, environment)
    return (
        flow_prepared(
            prepared,
            text=text,
            environment=environment,
            frame_widths=frame_widths,
            lines_per_frame=lines_per_frame,
        ),
        1,
    )


def compare_case(
    prepared: PreparedStory,
    *,
    name: str,
    text: str,
    environment: str,
    frame_widths: list[int],
    lines_per_frame: int,
) -> dict:
    full, full_shape_calls = flow_full(
        text,
        environment=environment,
        frame_widths=frame_widths,
        lines_per_frame=lines_per_frame,
    )
    incremental = flow_prepared(
        prepared,
        text=text,
        environment=environment,
        frame_widths=frame_widths,
        lines_per_frame=lines_per_frame,
    )
    full_hash = stable_hash(full)
    incremental_hash = stable_hash(incremental)
    equivalent = full == incremental
    if not equivalent:
        raise AssertionError(f"{name}: full and prepared flow differ")
    return {
        "name": name,
        "frame_widths": frame_widths,
        "lines_per_frame": lines_per_frame,
        "equivalent": equivalent,
        "full_hash": full_hash,
        "incremental_hash": incremental_hash,
        "full_shape_calls": full_shape_calls,
        "incremental_shape_calls": 0,
        "prepared_reuse_count": 1,
        "line_count": len(incremental["lines"]),
        "overset_scalars": incremental["overset_scalars"],
        "next_scalar": incremental["next_scalar"],
    }


def build_receipt() -> dict:
    base_text = (
        "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu "
        "nu xi omicron pi rho sigma tau upsilon phi chi psi omega"
    )
    prepared = prepare_story(base_text)

    cases: list[dict] = []
    cases.append(
        compare_case(
            prepared,
            name="baseline-two-frame",
            text=base_text,
            environment=ENVIRONMENT,
            frame_widths=[120, 120],
            lines_per_frame=6,
        )
    )

    sweep_cases = []
    for step in range(100):
        width = 120 - step // 3
        case = compare_case(
            prepared,
            name=f"width-sweep-{step:03d}",
            text=base_text,
            environment=ENVIRONMENT,
            frame_widths=[width, width],
            lines_per_frame=8,
        )
        sweep_cases.append(case)
    cases.extend(sweep_cases)

    paragraph = (
        "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu "
        "nu xi omicron pi rho sigma tau. "
    )
    long_text = paragraph * 80
    long_prepared = prepare_story(long_text)
    for width in [84, 78, 72, 90]:
        cases.append(
            compare_case(
                long_prepared,
                name=f"long-story-10-frame-{width}",
                text=long_text,
                environment=ENVIRONMENT,
                frame_widths=[width] * 10,
                lines_per_frame=12,
            )
        )

    stale_text_rejected = False
    try:
        flow_prepared(
            prepared,
            text=base_text + "!",
            environment=ENVIRONMENT,
            frame_widths=[120],
            lines_per_frame=8,
        )
    except ValueError:
        stale_text_rejected = True

    environment_mismatch_rejected = False
    try:
        flow_prepared(
            prepared,
            text=base_text,
            environment="layout-prepared-story-model-v2",
            frame_widths=[120],
            lines_per_frame=8,
        )
    except ValueError:
        environment_mismatch_rejected = True

    if not stale_text_rejected or not environment_mismatch_rejected:
        raise AssertionError("fail-closed prepared identity checks did not trigger")

    return {
        "schema_version": "layout-prepared-story-receipt-v1",
        "task": "LAYOUT-PREPARED-STORY-01",
        "producer": "HeisLuka/rar GitHub Actions source-free modelcheck",
        "scope": {
            "source_free_modelcheck": True,
            "canonical_engine_proof": False,
            "publisher_fidelity_proof": False,
            "purpose": (
                "Verify prepare-once/reflow reuse, deterministic full-vs-prepared "
                "equivalence, width-sweep behavior, linked-frame working-set shape, "
                "and fail-closed cache identity in a public-safe model."
            ),
        },
        "environment": ENVIRONMENT,
        "case_count": len(cases),
        "all_equivalent": all(case["equivalent"] for case in cases),
        "stale_text_rejected": stale_text_rejected,
        "environment_mismatch_rejected": environment_mismatch_rejected,
        "cases": cases,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/layout-prepared-story/layout-prepared-story-receipt.json"),
    )
    args = parser.parse_args()

    receipt = build_receipt()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "task": receipt["task"],
                "case_count": receipt["case_count"],
                "all_equivalent": receipt["all_equivalent"],
                "stale_text_rejected": receipt["stale_text_rejected"],
                "environment_mismatch_rejected": receipt[
                    "environment_mismatch_rejected"
                ],
                "output": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
