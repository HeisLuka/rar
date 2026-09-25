#!/usr/bin/env python3
"""Source-neutral resolved text caret/selection geometry V1.

This module consumes explicit authoritative resolved-line/cluster provenance.
It does not shape text, choose line breaks, inspect DOM/widget layout, or read
PUB carriers.

V1 is horizontal LTR only. It preserves Story-flow order supplied by resolved
layout, including linked-frame adjacency. Caret stops are admitted at cluster
edges plus any explicit internal-caret authority supplied by shaping. Same
canonical scalar may intentionally map to multiple physical stops; callers that
provide only that scalar receive caret_affinity_required rather than an
arbitrary first-line choice.

Selection geometry partitions a canonical range into:
- covered_ranges: placed + geometrically admitted (logical-only CR clusters may
  be covered without a painted rectangle);
- unplaced_ranges: no resolved cluster exists (e.g. overset tail);
- unsupported_ranges: resolved cluster exists but requested interior cluster
  boundary lacks explicit caret authority.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Literal


MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class ResolvedTextCaretMapError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class ScalarRangeV1:
    start_scalar: int
    end_scalar: int


@dataclass(frozen=True)
class InternalCaretStopV1:
    scalar_boundary: int
    page_x_emu: int
    frame_x_emu: int
    authority_source: str = "shaping_explicit_v1"


@dataclass(frozen=True)
class ResolvedClusterV1:
    start_scalar: int
    end_scalar: int
    page_x_start_emu: int
    page_x_end_emu: int
    frame_x_start_emu: int
    frame_x_end_emu: int
    painted: bool = True
    internal_caret_stops: tuple[InternalCaretStopV1, ...] = ()


@dataclass(frozen=True)
class ResolvedLineFragmentV1:
    story_id: str
    page_id: str
    frame_id: str
    line_id: str
    flow_ordinal: int
    previous_line_id: str | None
    next_line_id: str | None
    page_y_top_emu: int
    page_y_bottom_emu: int
    frame_y_top_emu: int
    frame_y_bottom_emu: int
    clusters: tuple[ResolvedClusterV1, ...]


@dataclass(frozen=True)
class CaretStopV1:
    stop_id: str
    story_id: str
    scalar_boundary: int
    page_id: str
    frame_id: str
    line_id: str
    flow_ordinal: int
    page_x_emu: int
    page_y_top_emu: int
    page_y_bottom_emu: int
    frame_x_emu: int
    frame_y_top_emu: int
    frame_y_bottom_emu: int
    affinities: tuple[Literal["upstream", "downstream", "internal"], ...]
    authority_source: str | None = None


@dataclass(frozen=True)
class SelectionRectV1:
    page_id: str
    frame_id: str
    line_id: str
    flow_ordinal: int
    start_scalar: int
    end_scalar: int
    page_x_start_emu: int
    page_x_end_emu: int
    page_y_top_emu: int
    page_y_bottom_emu: int
    frame_x_start_emu: int
    frame_x_end_emu: int
    frame_y_top_emu: int
    frame_y_bottom_emu: int


@dataclass(frozen=True)
class SelectionGeometryV1:
    protocol_version: Literal["chaptera.selection-geometry.v1"]
    story_id: str
    start_scalar: int
    end_scalar: int
    is_empty: bool
    rectangles: tuple[SelectionRectV1, ...]
    covered_ranges: tuple[ScalarRangeV1, ...]
    unplaced_ranges: tuple[ScalarRangeV1, ...]
    unsupported_ranges: tuple[ScalarRangeV1, ...]
    coverage_state: Literal["complete", "partial", "unplaced", "unsupported"]


@dataclass(frozen=True)
class ResolvedTextCaretMapV1:
    protocol_version: Literal["chaptera.resolved-text-caret-map.v1"]
    layout_revision_id: str
    story_id: str
    story_scalar_len: int
    lines: tuple[ResolvedLineFragmentV1, ...]
    caret_stops: tuple[CaretStopV1, ...]
    materialized_ranges: tuple[ScalarRangeV1, ...]


def _fail(code: str, message: str) -> None:
    raise ResolvedTextCaretMapError(code, message)


def _safe_emu(value: int, label: str) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < MIN_SAFE_EMU
        or value > MAX_SAFE_EMU
    ):
        _fail("invalid_geometry", f"{label} must be a JavaScript-safe EMU integer")
    return value


def _validate_range(start: int, end: int, story_len: int, label: str) -> None:
    if (
        not isinstance(start, int)
        or isinstance(start, bool)
        or not isinstance(end, int)
        or isinstance(end, bool)
        or start < 0
        or end < start
        or end > story_len
    ):
        _fail("invalid_story_range", f"{label} lies outside Story scalar extent")


def _merge_ranges(ranges: list[tuple[int, int]]) -> tuple[ScalarRangeV1, ...]:
    if not ranges:
        return ()
    ranges = sorted((a, b) for a, b in ranges if b > a)
    merged = []
    for a, b in ranges:
        if not merged or a > merged[-1][1]:
            merged.append([a, b])
        else:
            merged[-1][1] = max(merged[-1][1], b)
    return tuple(ScalarRangeV1(a, b) for a, b in merged)


def _subtract_ranges(
    start: int,
    end: int,
    covered: tuple[ScalarRangeV1, ...],
) -> tuple[ScalarRangeV1, ...]:
    if start == end:
        return ()
    cursor = start
    out = []
    for item in covered:
        a = max(start, item.start_scalar)
        b = min(end, item.end_scalar)
        if b <= a:
            continue
        if cursor < a:
            out.append((cursor, a))
        cursor = max(cursor, b)
    if cursor < end:
        out.append((cursor, end))
    return _merge_ranges(out)


def _validate_cluster(
    cluster: ResolvedClusterV1,
    *,
    story_len: int,
    line_id: str,
    index: int,
) -> ResolvedClusterV1:
    if not isinstance(cluster, ResolvedClusterV1):
        _fail("invalid_layout", f"{line_id}.clusters[{index}] is malformed")
    _validate_range(
        cluster.start_scalar,
        cluster.end_scalar,
        story_len,
        f"{line_id}.clusters[{index}]",
    )
    if cluster.end_scalar <= cluster.start_scalar:
        _fail("invalid_layout", "resolved cluster must cover one or more scalars")
    for name in (
        "page_x_start_emu",
        "page_x_end_emu",
        "frame_x_start_emu",
        "frame_x_end_emu",
    ):
        _safe_emu(getattr(cluster, name), f"{line_id}.{name}")
    if cluster.page_x_end_emu < cluster.page_x_start_emu:
        _fail("invalid_layout", "horizontal LTR cluster page x must be nondecreasing")
    if cluster.frame_x_end_emu < cluster.frame_x_start_emu:
        _fail("invalid_layout", "horizontal LTR cluster frame x must be nondecreasing")
    if not isinstance(cluster.painted, bool):
        _fail("invalid_layout", "cluster painted flag must be boolean")
    if not isinstance(cluster.internal_caret_stops, tuple):
        _fail("invalid_layout", "internal_caret_stops must be tuple")

    seen = set()
    canonical_internal = []
    for stop in cluster.internal_caret_stops:
        if not isinstance(stop, InternalCaretStopV1):
            _fail("invalid_layout", "internal caret stop is malformed")
        if not (
            cluster.start_scalar
            < stop.scalar_boundary
            < cluster.end_scalar
        ):
            _fail(
                "invalid_layout",
                "internal caret authority must lie strictly inside cluster",
            )
        if stop.scalar_boundary in seen:
            _fail("invalid_layout", "duplicate internal caret scalar boundary")
        seen.add(stop.scalar_boundary)
        _safe_emu(stop.page_x_emu, "internal.page_x_emu")
        _safe_emu(stop.frame_x_emu, "internal.frame_x_emu")
        if not isinstance(stop.authority_source, str) or not stop.authority_source:
            _fail("invalid_layout", "internal caret authority_source is required")
        if not (
            cluster.page_x_start_emu
            <= stop.page_x_emu
            <= cluster.page_x_end_emu
        ):
            _fail("invalid_layout", "internal page caret lies outside cluster advance")
        if not (
            cluster.frame_x_start_emu
            <= stop.frame_x_emu
            <= cluster.frame_x_end_emu
        ):
            _fail("invalid_layout", "internal frame caret lies outside cluster advance")
        canonical_internal.append(stop)
    canonical_internal.sort(key=lambda value: value.scalar_boundary)
    return ResolvedClusterV1(
        start_scalar=cluster.start_scalar,
        end_scalar=cluster.end_scalar,
        page_x_start_emu=cluster.page_x_start_emu,
        page_x_end_emu=cluster.page_x_end_emu,
        frame_x_start_emu=cluster.frame_x_start_emu,
        frame_x_end_emu=cluster.frame_x_end_emu,
        painted=cluster.painted,
        internal_caret_stops=tuple(canonical_internal),
    )


def _validate_line(
    line: ResolvedLineFragmentV1,
    *,
    story_id: str,
    story_len: int,
) -> ResolvedLineFragmentV1:
    if not isinstance(line, ResolvedLineFragmentV1):
        _fail("invalid_layout", "resolved line is malformed")
    for label, value in (
        ("story_id", line.story_id),
        ("page_id", line.page_id),
        ("frame_id", line.frame_id),
        ("line_id", line.line_id),
    ):
        if not isinstance(value, str) or not value:
            _fail("invalid_layout", f"{label} is required")
    if line.story_id != story_id:
        _fail("invalid_layout", "resolved line targets different Story")
    if (
        not isinstance(line.flow_ordinal, int)
        or isinstance(line.flow_ordinal, bool)
        or line.flow_ordinal < 0
    ):
        _fail("invalid_layout", "flow_ordinal must be non-negative integer")
    for name in (
        "page_y_top_emu",
        "page_y_bottom_emu",
        "frame_y_top_emu",
        "frame_y_bottom_emu",
    ):
        _safe_emu(getattr(line, name), f"{line.line_id}.{name}")
    if line.page_y_bottom_emu <= line.page_y_top_emu:
        _fail("invalid_layout", "line page extent must be positive")
    if line.frame_y_bottom_emu <= line.frame_y_top_emu:
        _fail("invalid_layout", "line frame extent must be positive")
    if not isinstance(line.clusters, tuple) or not line.clusters:
        _fail("invalid_layout", "resolved line requires one or more clusters")

    clusters = tuple(
        _validate_cluster(
            cluster,
            story_len=story_len,
            line_id=line.line_id,
            index=index,
        )
        for index, cluster in enumerate(line.clusters)
    )
    clusters = tuple(sorted(clusters, key=lambda c: (c.start_scalar, c.end_scalar)))
    for previous, current in zip(clusters, clusters[1:]):
        if previous.end_scalar > current.start_scalar:
            _fail("invalid_layout", "resolved clusters overlap within one line")
    return ResolvedLineFragmentV1(
        story_id=line.story_id,
        page_id=line.page_id,
        frame_id=line.frame_id,
        line_id=line.line_id,
        flow_ordinal=line.flow_ordinal,
        previous_line_id=line.previous_line_id,
        next_line_id=line.next_line_id,
        page_y_top_emu=line.page_y_top_emu,
        page_y_bottom_emu=line.page_y_bottom_emu,
        frame_y_top_emu=line.frame_y_top_emu,
        frame_y_bottom_emu=line.frame_y_bottom_emu,
        clusters=clusters,
    )


def _cluster_boundary_coordinates(
    cluster: ResolvedClusterV1,
    scalar_boundary: int,
) -> tuple[int, int, str] | None:
    if scalar_boundary == cluster.start_scalar:
        return (
            cluster.page_x_start_emu,
            cluster.frame_x_start_emu,
            "downstream",
        )
    if scalar_boundary == cluster.end_scalar:
        return (
            cluster.page_x_end_emu,
            cluster.frame_x_end_emu,
            "upstream",
        )
    for stop in cluster.internal_caret_stops:
        if stop.scalar_boundary == scalar_boundary:
            return (stop.page_x_emu, stop.frame_x_emu, "internal")
    return None


def _build_caret_stops(
    lines: tuple[ResolvedLineFragmentV1, ...],
) -> tuple[CaretStopV1, ...]:
    stops = []
    for line in lines:
        candidates: dict[
            tuple[int, int, int],
            set[str],
        ] = {}
        internal_authority_by_key: dict[tuple[int, int, int], str] = {}
        for cluster in line.clusters:
            for scalar, page_x, frame_x, affinity in (
                (
                    cluster.start_scalar,
                    cluster.page_x_start_emu,
                    cluster.frame_x_start_emu,
                    "downstream",
                ),
                (
                    cluster.end_scalar,
                    cluster.page_x_end_emu,
                    cluster.frame_x_end_emu,
                    "upstream",
                ),
            ):
                candidates.setdefault(
                    (scalar, page_x, frame_x),
                    set(),
                ).add(affinity)
            for internal in cluster.internal_caret_stops:
                key = (
                    internal.scalar_boundary,
                    internal.page_x_emu,
                    internal.frame_x_emu,
                )
                candidates.setdefault(key, set()).add("internal")
                previous_source = internal_authority_by_key.get(key)
                if (
                    previous_source is not None
                    and previous_source != internal.authority_source
                ):
                    _fail(
                        "invalid_layout",
                        "same internal caret geometry has conflicting authority_source",
                    )
                internal_authority_by_key[key] = internal.authority_source

        ordered = sorted(
            candidates.items(),
            key=lambda item: (
                item[0][0],
                item[0][1],
                item[0][2],
            ),
        )
        for ordinal, ((scalar, page_x, frame_x), affinities) in enumerate(ordered):
            stops.append(
                CaretStopV1(
                    stop_id=f"{line.line_id}:stop:{ordinal}",
                    story_id=line.story_id,
                    scalar_boundary=scalar,
                    page_id=line.page_id,
                    frame_id=line.frame_id,
                    line_id=line.line_id,
                    flow_ordinal=line.flow_ordinal,
                    page_x_emu=page_x,
                    page_y_top_emu=line.page_y_top_emu,
                    page_y_bottom_emu=line.page_y_bottom_emu,
                    frame_x_emu=frame_x,
                    frame_y_top_emu=line.frame_y_top_emu,
                    frame_y_bottom_emu=line.frame_y_bottom_emu,
                    affinities=tuple(
                        value
                        for value in ("upstream", "downstream", "internal")
                        if value in affinities
                    ),
                    authority_source=internal_authority_by_key.get(
                        (scalar, page_x, frame_x)
                    ),
                )
            )
    stops.sort(
        key=lambda stop: (
            stop.flow_ordinal,
            stop.scalar_boundary,
            stop.page_x_emu,
            stop.stop_id,
        )
    )
    return tuple(stops)


def build_resolved_text_caret_map_v1(
    *,
    layout_revision_id: str,
    story_id: str,
    story_scalar_len: int,
    lines: tuple[ResolvedLineFragmentV1, ...],
) -> ResolvedTextCaretMapV1:
    if not isinstance(layout_revision_id, str) or not layout_revision_id:
        _fail("invalid_layout", "layout_revision_id is required")
    if not isinstance(story_id, str) or not story_id:
        _fail("invalid_layout", "story_id is required")
    if (
        not isinstance(story_scalar_len, int)
        or isinstance(story_scalar_len, bool)
        or story_scalar_len < 0
    ):
        _fail("invalid_layout", "story_scalar_len must be non-negative")
    if not isinstance(lines, tuple):
        _fail("invalid_layout", "lines must be an ordered tuple")

    canonical = tuple(
        _validate_line(line, story_id=story_id, story_len=story_scalar_len)
        for line in lines
    )
    canonical = tuple(sorted(canonical, key=lambda line: line.flow_ordinal))
    ids = [line.line_id for line in canonical]
    ordinals = [line.flow_ordinal for line in canonical]
    if len(set(ids)) != len(ids):
        _fail("invalid_layout", "line_id must be unique")
    if len(set(ordinals)) != len(ordinals):
        _fail("invalid_layout", "flow_ordinal must be unique")
    if canonical and ordinals != list(range(ordinals[0], ordinals[0] + len(ordinals))):
        _fail("invalid_layout", "flow_ordinal sequence must be contiguous")

    for index, line in enumerate(canonical):
        expected_prev = None if index == 0 else canonical[index - 1].line_id
        expected_next = None if index + 1 == len(canonical) else canonical[index + 1].line_id
        if line.previous_line_id != expected_prev:
            _fail("invalid_layout", "previous_line_id disagrees with Story-flow order")
        if line.next_line_id != expected_next:
            _fail("invalid_layout", "next_line_id disagrees with Story-flow order")

    all_ranges = []
    for line in canonical:
        for cluster in line.clusters:
            all_ranges.append((cluster.start_scalar, cluster.end_scalar))
    sorted_ranges = sorted(all_ranges)
    for previous, current in zip(sorted_ranges, sorted_ranges[1:]):
        if previous[1] > current[0]:
            _fail(
                "invalid_layout",
                "resolved cluster scalar coverage overlaps across Story-flow lines",
            )

    return ResolvedTextCaretMapV1(
        protocol_version="chaptera.resolved-text-caret-map.v1",
        layout_revision_id=layout_revision_id,
        story_id=story_id,
        story_scalar_len=story_scalar_len,
        lines=canonical,
        caret_stops=_build_caret_stops(canonical),
        materialized_ranges=_merge_ranges(all_ranges),
    )


def _require_layout(
    caret_map: ResolvedTextCaretMapV1,
    expected_layout_revision_id: str | None,
) -> None:
    if not isinstance(caret_map, ResolvedTextCaretMapV1):
        _fail("invalid_caret_map", "ResolvedTextCaretMapV1 is required")
    if (
        expected_layout_revision_id is not None
        and expected_layout_revision_id != caret_map.layout_revision_id
    ):
        _fail("stale_layout_map", "caret map belongs to a different layout revision")


def resolve_story_position_v1(
    *,
    caret_map: ResolvedTextCaretMapV1,
    scalar_boundary: int,
    stop_id: str | None = None,
    expected_layout_revision_id: str | None = None,
) -> CaretStopV1:
    _require_layout(caret_map, expected_layout_revision_id)
    if (
        not isinstance(scalar_boundary, int)
        or isinstance(scalar_boundary, bool)
        or scalar_boundary < 0
        or scalar_boundary > caret_map.story_scalar_len
    ):
        _fail("invalid_story_position", "scalar boundary lies outside Story")

    matches = [
        stop
        for stop in caret_map.caret_stops
        if stop.scalar_boundary == scalar_boundary
    ]
    if stop_id is not None:
        selected = [stop for stop in matches if stop.stop_id == stop_id]
        if len(selected) != 1:
            _fail("invalid_caret_affinity", "requested caret stop is not admitted")
        return selected[0]

    if len(matches) == 1:
        return matches[0]
    if len(matches) > 1:
        # Distinct physical line/geometry stops for one scalar are preserved.
        physical = {
            (
                stop.page_id,
                stop.frame_id,
                stop.line_id,
                stop.page_x_emu,
                stop.page_y_top_emu,
                stop.page_y_bottom_emu,
            )
            for stop in matches
        }
        if len(physical) > 1:
            _fail(
                "caret_affinity_required",
                "one Story scalar maps to multiple physical caret stops",
            )
        return matches[0]

    # Distinguish an unsupported interior cluster boundary from unplaced text.
    for line in caret_map.lines:
        for cluster in line.clusters:
            if cluster.start_scalar < scalar_boundary < cluster.end_scalar:
                _fail(
                    "internal_cluster_unsupported",
                    "cluster interior has no explicit shaping caret authority",
                )
    _fail("unplaced_story_position", "Story position has no resolved caret stop")


def hit_test_story_position_v1(
    *,
    caret_map: ResolvedTextCaretMapV1,
    page_id: str,
    page_x_emu: int,
    page_y_emu: int,
    expected_layout_revision_id: str | None = None,
) -> CaretStopV1:
    _require_layout(caret_map, expected_layout_revision_id)
    if not isinstance(page_id, str) or not page_id:
        _fail("invalid_hit_test", "page_id is required")
    _safe_emu(page_x_emu, "page_x_emu")
    _safe_emu(page_y_emu, "page_y_emu")
    lines = [line for line in caret_map.lines if line.page_id == page_id]
    if not lines:
        _fail("unplaced_hit_test", "page has no resolved Story lines")

    def vertical_distance(line: ResolvedLineFragmentV1) -> int:
        if page_y_emu < line.page_y_top_emu:
            return line.page_y_top_emu - page_y_emu
        if page_y_emu > line.page_y_bottom_emu:
            return page_y_emu - line.page_y_bottom_emu
        return 0

    line = min(
        lines,
        key=lambda value: (
            vertical_distance(value),
            value.flow_ordinal,
        ),
    )
    stops = [stop for stop in caret_map.caret_stops if stop.line_id == line.line_id]
    if not stops:
        _fail("unplaced_hit_test", "resolved line has no admitted caret stops")
    return min(
        stops,
        key=lambda stop: (
            abs(stop.page_x_emu - page_x_emu),
            stop.scalar_boundary,
            stop.stop_id,
        ),
    )


def _boundary_x(
    cluster: ResolvedClusterV1,
    scalar_boundary: int,
) -> tuple[int, int] | None:
    result = _cluster_boundary_coordinates(cluster, scalar_boundary)
    if result is None:
        return None
    return result[0], result[1]


def selection_geometry_v1(
    *,
    caret_map: ResolvedTextCaretMapV1,
    start_scalar: int,
    end_scalar: int,
    expected_layout_revision_id: str | None = None,
) -> SelectionGeometryV1:
    _require_layout(caret_map, expected_layout_revision_id)
    _validate_range(
        start_scalar,
        end_scalar,
        caret_map.story_scalar_len,
        "selection",
    )
    if start_scalar == end_scalar:
        return SelectionGeometryV1(
            protocol_version="chaptera.selection-geometry.v1",
            story_id=caret_map.story_id,
            start_scalar=start_scalar,
            end_scalar=end_scalar,
            is_empty=True,
            rectangles=(),
            covered_ranges=(),
            unplaced_ranges=(),
            unsupported_ranges=(),
            coverage_state="complete",
        )

    rectangles = []
    covered = []
    unsupported = []
    materialized_intersections = []

    for line in caret_map.lines:
        for cluster in line.clusters:
            a = max(start_scalar, cluster.start_scalar)
            b = min(end_scalar, cluster.end_scalar)
            if b <= a:
                continue
            materialized_intersections.append((a, b))
            left = _boundary_x(cluster, a)
            right = _boundary_x(cluster, b)
            if left is None or right is None:
                unsupported.append((a, b))
                continue
            covered.append((a, b))
            if cluster.painted and right[0] != left[0]:
                rectangles.append(
                    SelectionRectV1(
                        page_id=line.page_id,
                        frame_id=line.frame_id,
                        line_id=line.line_id,
                        flow_ordinal=line.flow_ordinal,
                        start_scalar=a,
                        end_scalar=b,
                        page_x_start_emu=min(left[0], right[0]),
                        page_x_end_emu=max(left[0], right[0]),
                        page_y_top_emu=line.page_y_top_emu,
                        page_y_bottom_emu=line.page_y_bottom_emu,
                        frame_x_start_emu=min(left[1], right[1]),
                        frame_x_end_emu=max(left[1], right[1]),
                        frame_y_top_emu=line.frame_y_top_emu,
                        frame_y_bottom_emu=line.frame_y_bottom_emu,
                    )
                )

    materialized = _merge_ranges(materialized_intersections)
    unplaced = _subtract_ranges(start_scalar, end_scalar, materialized)
    covered_ranges = _merge_ranges(covered)
    unsupported_ranges = _merge_ranges(unsupported)
    rectangles.sort(
        key=lambda rect: (
            rect.flow_ordinal,
            rect.start_scalar,
            rect.end_scalar,
            rect.page_x_start_emu,
        )
    )

    if unsupported_ranges:
        state = "unsupported"
    elif unplaced:
        if not covered_ranges:
            state = "unplaced"
        else:
            state = "partial"
    else:
        state = "complete"

    return SelectionGeometryV1(
        protocol_version="chaptera.selection-geometry.v1",
        story_id=caret_map.story_id,
        start_scalar=start_scalar,
        end_scalar=end_scalar,
        is_empty=False,
        rectangles=tuple(rectangles),
        covered_ranges=covered_ranges,
        unplaced_ranges=unplaced,
        unsupported_ranges=unsupported_ranges,
        coverage_state=state,
    )


def _cluster_to_dict(cluster: ResolvedClusterV1) -> dict:
    return {
        "start_scalar": cluster.start_scalar,
        "end_scalar": cluster.end_scalar,
        "page_x_start_emu": cluster.page_x_start_emu,
        "page_x_end_emu": cluster.page_x_end_emu,
        "frame_x_start_emu": cluster.frame_x_start_emu,
        "frame_x_end_emu": cluster.frame_x_end_emu,
        "painted": cluster.painted,
        "internal_caret_stops": [
            {
                "scalar_boundary": stop.scalar_boundary,
                "page_x_emu": stop.page_x_emu,
                "frame_x_emu": stop.frame_x_emu,
                "authority_source": stop.authority_source,
            }
            for stop in cluster.internal_caret_stops
        ],
    }


def caret_map_to_dict_v1(caret_map: ResolvedTextCaretMapV1) -> dict:
    return {
        "protocol_version": caret_map.protocol_version,
        "layout_revision_id": caret_map.layout_revision_id,
        "story_id": caret_map.story_id,
        "story_scalar_len": caret_map.story_scalar_len,
        "lines": [
            {
                "story_id": line.story_id,
                "page_id": line.page_id,
                "frame_id": line.frame_id,
                "line_id": line.line_id,
                "flow_ordinal": line.flow_ordinal,
                "previous_line_id": line.previous_line_id,
                "next_line_id": line.next_line_id,
                "page_y_top_emu": line.page_y_top_emu,
                "page_y_bottom_emu": line.page_y_bottom_emu,
                "frame_y_top_emu": line.frame_y_top_emu,
                "frame_y_bottom_emu": line.frame_y_bottom_emu,
                "clusters": [_cluster_to_dict(cluster) for cluster in line.clusters],
            }
            for line in caret_map.lines
        ],
        "caret_stops": [
            {
                "stop_id": stop.stop_id,
                "scalar_boundary": stop.scalar_boundary,
                "page_id": stop.page_id,
                "frame_id": stop.frame_id,
                "line_id": stop.line_id,
                "flow_ordinal": stop.flow_ordinal,
                "page_x_emu": stop.page_x_emu,
                "page_y_top_emu": stop.page_y_top_emu,
                "page_y_bottom_emu": stop.page_y_bottom_emu,
                "frame_x_emu": stop.frame_x_emu,
                "frame_y_top_emu": stop.frame_y_top_emu,
                "frame_y_bottom_emu": stop.frame_y_bottom_emu,
                "affinities": list(stop.affinities),
                **(
                    {"authority_source": stop.authority_source}
                    if stop.authority_source is not None
                    else {}
                ),
            }
            for stop in caret_map.caret_stops
        ],
        "materialized_ranges": [
            [item.start_scalar, item.end_scalar]
            for item in caret_map.materialized_ranges
        ],
    }


def caret_map_hash_v1(caret_map: ResolvedTextCaretMapV1) -> str:
    return hashlib.sha256(
        json.dumps(
            caret_map_to_dict_v1(caret_map),
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()
