#!/usr/bin/env python3
"""Source-neutral deterministic imposition planner V1.

Logical document pages remain identities. Physical output is represented only
as sheet/side/slot placements that reference those logical page IDs.

V1 covers:
- one logical page per physical sheet side;
- repeated copies in a row-major N-up grid;
- distinct logical pages in a row-major N-up grid;
- duplex front/back pairing with explicit flip policy;
- standard saddle-stitch booklet imposition for page counts divisible by four.

Rendering, printer drivers, paper margins, bleed, crop marks and device color
are deliberately outside this planner.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


SideV1 = Literal["front", "back"]
DuplexFlipV1 = Literal["long_edge", "short_edge", "none"]

MAX_GRID_AXIS_V1 = 64
MAX_PLACEMENTS_V1 = 4096


class ImpositionPlanError(ValueError):
    pass


@dataclass(frozen=True)
class PlacementV1:
    logical_page_id: str
    sheet_index: int
    side: SideV1
    slot_index: int
    row: int
    column: int
    rotation_quarter_turns: int = 0


@dataclass(frozen=True)
class SheetSidePlanV1:
    sheet_index: int
    side: SideV1
    rows: int
    columns: int
    placements: tuple[PlacementV1, ...]


@dataclass(frozen=True)
class PhysicalSheetV1:
    sheet_index: int
    front: SheetSidePlanV1
    back: SheetSidePlanV1 | None
    duplex_flip: DuplexFlipV1


@dataclass(frozen=True)
class ImpositionPlanV1:
    protocol_version: Literal["chaptera.imposition-plan.v1"]
    mode: str
    logical_page_ids: tuple[str, ...]
    sheets: tuple[PhysicalSheetV1, ...]

    @property
    def placement_count(self) -> int:
        count = 0
        for sheet in self.sheets:
            count += len(sheet.front.placements)
            if sheet.back is not None:
                count += len(sheet.back.placements)
        return count


def _fail(message: str) -> None:
    raise ImpositionPlanError(message)


def _page_id(value: str, label: str) -> str:
    if not isinstance(value, str) or not value:
        _fail(f"{label} is required")
    return value


def _grid(rows: int, columns: int) -> int:
    for value, label in ((rows, "rows"), (columns, "columns")):
        if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
            _fail(f"{label} must be a positive integer")
        if value > MAX_GRID_AXIS_V1:
            _fail(f"{label} exceeds V1 grid bound")
    capacity = rows * columns
    if capacity > MAX_PLACEMENTS_V1:
        _fail("grid capacity exceeds V1 placement bound")
    return capacity


def _unique_pages(page_ids: tuple[str, ...], label: str) -> tuple[str, ...]:
    if not isinstance(page_ids, tuple) or not page_ids:
        _fail(f"{label} must be a non-empty ordered tuple")
    normalized = tuple(_page_id(page_id, f"{label}[{index}]") for index, page_id in enumerate(page_ids))
    if len(set(normalized)) != len(normalized):
        _fail(f"{label} must contain distinct logical page IDs")
    return normalized


def _side_plan(
    *,
    sheet_index: int,
    side: SideV1,
    rows: int,
    columns: int,
    page_ids: tuple[str, ...],
    rotations: tuple[int, ...] | None = None,
) -> SheetSidePlanV1:
    capacity = _grid(rows, columns)
    if len(page_ids) > capacity:
        _fail("side placement count exceeds grid capacity")
    if rotations is not None and len(rotations) != len(page_ids):
        _fail("rotation list length must match placements")

    placements = []
    for slot_index, page_id in enumerate(page_ids):
        _page_id(page_id, f"placement[{slot_index}].logical_page_id")
        rotation = 0 if rotations is None else rotations[slot_index]
        if rotation not in {0, 1, 2, 3}:
            _fail("rotation_quarter_turns must be 0..3")
        row, column = divmod(slot_index, columns)
        placements.append(
            PlacementV1(
                logical_page_id=page_id,
                sheet_index=sheet_index,
                side=side,
                slot_index=slot_index,
                row=row,
                column=column,
                rotation_quarter_turns=rotation,
            )
        )
    return SheetSidePlanV1(
        sheet_index=sheet_index,
        side=side,
        rows=rows,
        columns=columns,
        placements=tuple(placements),
    )


def _plan(
    *,
    mode: str,
    logical_page_ids: tuple[str, ...],
    sheets: list[PhysicalSheetV1],
) -> ImpositionPlanV1:
    if not sheets:
        _fail("imposition plan must contain at least one physical sheet")
    if len(sheets) > MAX_PLACEMENTS_V1:
        _fail("physical sheet count exceeds V1 bound")

    expected = list(range(len(sheets)))
    actual = [sheet.sheet_index for sheet in sheets]
    if actual != expected:
        _fail("physical sheet indices must be contiguous from zero")

    plan = ImpositionPlanV1(
        protocol_version="chaptera.imposition-plan.v1",
        mode=mode,
        logical_page_ids=logical_page_ids,
        sheets=tuple(sheets),
    )
    if plan.placement_count > MAX_PLACEMENTS_V1:
        _fail("imposition placement count exceeds V1 bound")
    return plan


def plan_one_page_per_sheet_v1(
    *, logical_page_ids: tuple[str, ...]
) -> ImpositionPlanV1:
    pages = _unique_pages(logical_page_ids, "logical_page_ids")
    sheets = []
    for index, page_id in enumerate(pages):
        sheets.append(
            PhysicalSheetV1(
                sheet_index=index,
                front=_side_plan(
                    sheet_index=index,
                    side="front",
                    rows=1,
                    columns=1,
                    page_ids=(page_id,),
                ),
                back=None,
                duplex_flip="none",
            )
        )
    return _plan(mode="one_page_per_sheet", logical_page_ids=pages, sheets=sheets)


def plan_repeated_n_up_v1(
    *,
    logical_page_id: str,
    copies: int,
    rows: int,
    columns: int,
) -> ImpositionPlanV1:
    page_id = _page_id(logical_page_id, "logical_page_id")
    capacity = _grid(rows, columns)
    if not isinstance(copies, int) or isinstance(copies, bool) or copies <= 0:
        _fail("copies must be a positive integer")
    if copies > MAX_PLACEMENTS_V1:
        _fail("copies exceeds V1 placement bound")

    sheets = []
    remaining = copies
    sheet_index = 0
    while remaining:
        count = min(remaining, capacity)
        sheets.append(
            PhysicalSheetV1(
                sheet_index=sheet_index,
                front=_side_plan(
                    sheet_index=sheet_index,
                    side="front",
                    rows=rows,
                    columns=columns,
                    page_ids=tuple(page_id for _ in range(count)),
                ),
                back=None,
                duplex_flip="none",
            )
        )
        remaining -= count
        sheet_index += 1
    return _plan(
        mode="repeated_n_up",
        logical_page_ids=(page_id,),
        sheets=sheets,
    )


def plan_distinct_n_up_v1(
    *,
    logical_page_ids: tuple[str, ...],
    rows: int,
    columns: int,
) -> ImpositionPlanV1:
    pages = _unique_pages(logical_page_ids, "logical_page_ids")
    capacity = _grid(rows, columns)
    sheets = []
    for offset in range(0, len(pages), capacity):
        sheet_index = len(sheets)
        chunk = pages[offset : offset + capacity]
        sheets.append(
            PhysicalSheetV1(
                sheet_index=sheet_index,
                front=_side_plan(
                    sheet_index=sheet_index,
                    side="front",
                    rows=rows,
                    columns=columns,
                    page_ids=chunk,
                ),
                back=None,
                duplex_flip="none",
            )
        )
    return _plan(mode="distinct_n_up", logical_page_ids=pages, sheets=sheets)


def plan_duplex_pairs_v1(
    *,
    front_page_ids: tuple[str, ...],
    back_page_ids: tuple[str, ...],
    flip: DuplexFlipV1,
    back_rotation_quarter_turns: int = 0,
) -> ImpositionPlanV1:
    fronts = _unique_pages(front_page_ids, "front_page_ids")
    backs = _unique_pages(back_page_ids, "back_page_ids")
    if len(fronts) != len(backs):
        _fail("duplex front/back page counts must match")
    if flip not in {"long_edge", "short_edge"}:
        _fail("duplex flip must be long_edge or short_edge")
    if back_rotation_quarter_turns not in {0, 1, 2, 3}:
        _fail("back rotation must be 0..3 quarter turns")

    combined = fronts + backs
    if len(set(combined)) != len(combined):
        _fail("duplex logical page IDs must be distinct across both sides")

    sheets = []
    for index, (front_id, back_id) in enumerate(zip(fronts, backs)):
        sheets.append(
            PhysicalSheetV1(
                sheet_index=index,
                front=_side_plan(
                    sheet_index=index,
                    side="front",
                    rows=1,
                    columns=1,
                    page_ids=(front_id,),
                ),
                back=_side_plan(
                    sheet_index=index,
                    side="back",
                    rows=1,
                    columns=1,
                    page_ids=(back_id,),
                    rotations=(back_rotation_quarter_turns,),
                ),
                duplex_flip=flip,
            )
        )
    return _plan(mode="duplex_pairs", logical_page_ids=combined, sheets=sheets)


def plan_booklet_v1(
    *,
    logical_page_ids: tuple[str, ...],
    flip: DuplexFlipV1 = "short_edge",
) -> ImpositionPlanV1:
    """Plan standard two-up saddle-stitch booklet sheets.

    For eight ordered logical pages [1..8], placements are:
      sheet 0 front: [8,1], back: [2,7]
      sheet 1 front: [6,3], back: [4,5]
    """

    pages = _unique_pages(logical_page_ids, "logical_page_ids")
    if len(pages) < 4 or len(pages) % 4 != 0:
        _fail("booklet page count must be a positive multiple of four")
    if flip not in {"long_edge", "short_edge"}:
        _fail("booklet flip must be long_edge or short_edge")

    sheets = []
    left = 0
    right = len(pages) - 1
    sheet_index = 0
    while left < right:
        front_pair = (pages[right], pages[left])
        left += 1
        right -= 1
        back_pair = (pages[left], pages[right])
        left += 1
        right -= 1

        sheets.append(
            PhysicalSheetV1(
                sheet_index=sheet_index,
                front=_side_plan(
                    sheet_index=sheet_index,
                    side="front",
                    rows=1,
                    columns=2,
                    page_ids=front_pair,
                ),
                back=_side_plan(
                    sheet_index=sheet_index,
                    side="back",
                    rows=1,
                    columns=2,
                    page_ids=back_pair,
                ),
                duplex_flip=flip,
            )
        )
        sheet_index += 1

    return _plan(mode="booklet", logical_page_ids=pages, sheets=sheets)


def fixture_edu_label_16up_v1() -> ImpositionPlanV1:
    return plan_repeated_n_up_v1(
        logical_page_id="FIX-EDU-LABEL-16UP-01:page:1",
        copies=16,
        rows=4,
        columns=4,
    )


def fixture_booklet_08p_v1() -> ImpositionPlanV1:
    return plan_booklet_v1(
        logical_page_ids=tuple(
            f"FIX-BOOKLET-08P-01:page:{index}" for index in range(1, 9)
        ),
        flip="short_edge",
    )
