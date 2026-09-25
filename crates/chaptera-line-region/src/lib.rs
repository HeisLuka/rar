//! Source-neutral deterministic line-band region solver.
//!
//! This crate owns geometry only. It consumes already-effective wrap semantics;
//! it does not inspect Publisher storage carriers, shape text, or mutate Story
//! state.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectEmuV1 {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl RectEmuV1 {
    pub fn right(self) -> Result<i64, LineRegionErrorV1> {
        self.x
            .checked_add(self.width)
            .ok_or(LineRegionErrorV1::CoordinateOverflow)
    }

    pub fn bottom(self) -> Result<i64, LineRegionErrorV1> {
        self.y
            .checked_add(self.height)
            .ok_or(LineRegionErrorV1::CoordinateOverflow)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsetsEmuV1 {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRegionV1 {
    pub frame_id: String,
    pub page_id: String,
    pub bounds: RectEmuV1,
    pub insets: InsetsEmuV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnsV1 {
    pub count: u32,
    pub gutter_emu: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapDistancesV1 {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectiveWrapModeV1 {
    None,
    Rectangle {
        rect: RectEmuV1,
        distances: WrapDistancesV1,
    },
    Unsupported {
        semantic_kind: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveWrapObstacleV1 {
    pub obstacle_id: String,
    pub mode: EffectiveWrapModeV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineBandRequestV1 {
    pub top_emu: i64,
    pub bottom_emu: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineIntervalV1 {
    pub column_index: u32,
    pub x0_emu: i64,
    pub x1_emu: i64,
}

impl LineIntervalV1 {
    pub fn width_emu(self) -> i64 {
        self.x1_emu - self.x0_emu
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRegionDiagnosticV1 {
    pub code: &'static str,
    pub obstacle_id: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBandSlotsV1 {
    pub requested: LineBandRequestV1,
    pub clipped_top_emu: i64,
    pub clipped_bottom_emu: i64,
    pub intervals: Vec<LineIntervalV1>,
    pub diagnostics: Vec<LineRegionDiagnosticV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntervalConsumptionPolicyV1 {
    LargestOnly,
    AllIntervalsLeftToRight,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineRegionErrorV1 {
    InvalidFrame,
    InvalidInsets,
    InvalidColumns,
    InvalidBand,
    InvalidWrapDistance { obstacle_id: String },
    InvalidObstacle { obstacle_id: String },
    CoordinateOverflow,
}

fn validate_rect(rect: RectEmuV1) -> Result<(), LineRegionErrorV1> {
    if rect.width <= 0 || rect.height <= 0 {
        return Err(LineRegionErrorV1::InvalidFrame);
    }
    let _ = rect.right()?;
    let _ = rect.bottom()?;
    Ok(())
}

fn validate_nonnegative(value: i64) -> bool {
    value >= 0
}

fn interior_rect(frame: &FrameRegionV1) -> Result<RectEmuV1, LineRegionErrorV1> {
    validate_rect(frame.bounds)?;
    let insets = frame.insets;
    if ![insets.left, insets.top, insets.right, insets.bottom]
        .into_iter()
        .all(validate_nonnegative)
    {
        return Err(LineRegionErrorV1::InvalidInsets);
    }

    let horizontal = insets
        .left
        .checked_add(insets.right)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let vertical = insets
        .top
        .checked_add(insets.bottom)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let width = frame
        .bounds
        .width
        .checked_sub(horizontal)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let height = frame
        .bounds
        .height
        .checked_sub(vertical)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    if width <= 0 || height <= 0 {
        return Err(LineRegionErrorV1::InvalidInsets);
    }
    let x = frame
        .bounds
        .x
        .checked_add(insets.left)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let y = frame
        .bounds
        .y
        .checked_add(insets.top)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let result = RectEmuV1 {
        x,
        y,
        width,
        height,
    };
    validate_rect(result)?;
    Ok(result)
}

pub fn resolve_columns_v1(
    frame: &FrameRegionV1,
    columns: ColumnsV1,
) -> Result<Vec<RectEmuV1>, LineRegionErrorV1> {
    let interior = interior_rect(frame)?;
    if columns.count == 0 || columns.gutter_emu < 0 {
        return Err(LineRegionErrorV1::InvalidColumns);
    }

    let count = i64::from(columns.count);
    let gaps = count - 1;
    let gutter_total = columns
        .gutter_emu
        .checked_mul(gaps)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let usable = interior
        .width
        .checked_sub(gutter_total)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    if usable < count {
        return Err(LineRegionErrorV1::InvalidColumns);
    }

    let base_width = usable / count;
    let remainder = usable % count;
    let mut result = Vec::with_capacity(columns.count as usize);
    let mut x = interior.x;

    for index in 0..columns.count {
        let extra = if index < u32::try_from(remainder).expect("remainder < count") {
            1
        } else {
            0
        };
        let width = base_width + extra;
        let rect = RectEmuV1 {
            x,
            y: interior.y,
            width,
            height: interior.height,
        };
        validate_rect(rect)?;
        result.push(rect);
        x = rect
            .right()?
            .checked_add(if index + 1 < columns.count {
                columns.gutter_emu
            } else {
                0
            })
            .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    }

    if result.last().map(|rect| rect.right()).transpose()? != Some(interior.right()?) {
        return Err(LineRegionErrorV1::InvalidColumns);
    }
    Ok(result)
}

fn validate_distances(
    obstacle_id: &str,
    distances: WrapDistancesV1,
) -> Result<(), LineRegionErrorV1> {
    if ![
        distances.left,
        distances.top,
        distances.right,
        distances.bottom,
    ]
    .into_iter()
    .all(validate_nonnegative)
    {
        return Err(LineRegionErrorV1::InvalidWrapDistance {
            obstacle_id: obstacle_id.to_owned(),
        });
    }
    Ok(())
}

fn expanded_obstacle(
    obstacle_id: &str,
    rect: RectEmuV1,
    distances: WrapDistancesV1,
) -> Result<RectEmuV1, LineRegionErrorV1> {
    validate_rect(rect).map_err(|_| LineRegionErrorV1::InvalidObstacle {
        obstacle_id: obstacle_id.to_owned(),
    })?;
    validate_distances(obstacle_id, distances)?;
    let x = rect
        .x
        .checked_sub(distances.left)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let y = rect
        .y
        .checked_sub(distances.top)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let horizontal = distances
        .left
        .checked_add(distances.right)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let vertical = distances
        .top
        .checked_add(distances.bottom)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let width = rect
        .width
        .checked_add(horizontal)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let height = rect
        .height
        .checked_add(vertical)
        .ok_or(LineRegionErrorV1::CoordinateOverflow)?;
    let result = RectEmuV1 {
        x,
        y,
        width,
        height,
    };
    validate_rect(result).map_err(|_| LineRegionErrorV1::InvalidObstacle {
        obstacle_id: obstacle_id.to_owned(),
    })?;
    Ok(result)
}

fn subtract_interval(
    intervals: &[LineIntervalV1],
    cut_x0: i64,
    cut_x1: i64,
) -> Vec<LineIntervalV1> {
    let mut output = Vec::new();
    for interval in intervals {
        if cut_x1 <= interval.x0_emu || cut_x0 >= interval.x1_emu {
            output.push(*interval);
            continue;
        }
        if cut_x0 > interval.x0_emu {
            output.push(LineIntervalV1 {
                column_index: interval.column_index,
                x0_emu: interval.x0_emu,
                x1_emu: cut_x0.min(interval.x1_emu),
            });
        }
        if cut_x1 < interval.x1_emu {
            output.push(LineIntervalV1 {
                column_index: interval.column_index,
                x0_emu: cut_x1.max(interval.x0_emu),
                x1_emu: interval.x1_emu,
            });
        }
    }
    output
}

pub fn resolve_line_band_v1(
    frame: &FrameRegionV1,
    columns: ColumnsV1,
    obstacles: &[EffectiveWrapObstacleV1],
    band: LineBandRequestV1,
) -> Result<LineBandSlotsV1, LineRegionErrorV1> {
    if band.bottom_emu <= band.top_emu {
        return Err(LineRegionErrorV1::InvalidBand);
    }

    let column_rects = resolve_columns_v1(frame, columns)?;
    let interior = interior_rect(frame)?;
    let clipped_top = band.top_emu.max(interior.y);
    let clipped_bottom = band.bottom_emu.min(interior.bottom()?);
    let mut diagnostics = Vec::new();

    if clipped_bottom <= clipped_top {
        diagnostics.push(LineRegionDiagnosticV1 {
            code: "line_region.band_outside_frame",
            obstacle_id: None,
            detail: None,
        });
        return Ok(LineBandSlotsV1 {
            requested: band,
            clipped_top_emu: clipped_top,
            clipped_bottom_emu: clipped_top,
            intervals: Vec::new(),
            diagnostics,
        });
    }

    let mut intervals = column_rects
        .iter()
        .enumerate()
        .map(|(index, column)| LineIntervalV1 {
            column_index: u32::try_from(index).expect("column count is u32"),
            x0_emu: column.x,
            x1_emu: column.right().expect("validated column"),
        })
        .collect::<Vec<_>>();

    for obstacle in obstacles {
        match &obstacle.mode {
            EffectiveWrapModeV1::None => {}
            EffectiveWrapModeV1::Unsupported { semantic_kind } => {
                diagnostics.push(LineRegionDiagnosticV1 {
                    code: "line_region.unsupported_wrap_semantics",
                    obstacle_id: Some(obstacle.obstacle_id.clone()),
                    detail: Some(semantic_kind.clone()),
                });
            }
            EffectiveWrapModeV1::Rectangle { rect, distances } => {
                let exclusion = expanded_obstacle(&obstacle.obstacle_id, *rect, *distances)?;
                if exclusion.y < clipped_bottom && exclusion.bottom()? > clipped_top {
                    let cut_x0 = exclusion.x.max(interior.x);
                    let cut_x1 = exclusion.right()?.min(interior.right()?);
                    if cut_x1 > cut_x0 {
                        intervals = subtract_interval(&intervals, cut_x0, cut_x1);
                    }
                }
            }
        }
    }

    intervals.retain(|interval| interval.x1_emu > interval.x0_emu);
    intervals.sort_by_key(|interval| (interval.column_index, interval.x0_emu, interval.x1_emu));

    Ok(LineBandSlotsV1 {
        requested: band,
        clipped_top_emu: clipped_top,
        clipped_bottom_emu: clipped_bottom,
        intervals,
        diagnostics,
    })
}

pub fn resolve_line_bands_v1(
    frame: &FrameRegionV1,
    columns: ColumnsV1,
    obstacles: &[EffectiveWrapObstacleV1],
    bands: &[LineBandRequestV1],
) -> Result<Vec<LineBandSlotsV1>, LineRegionErrorV1> {
    bands
        .iter()
        .copied()
        .map(|band| resolve_line_band_v1(frame, columns, obstacles, band))
        .collect()
}

pub fn select_line_intervals_v1(
    slots: &LineBandSlotsV1,
    policy: IntervalConsumptionPolicyV1,
) -> Vec<LineIntervalV1> {
    match policy {
        IntervalConsumptionPolicyV1::AllIntervalsLeftToRight => slots.intervals.clone(),
        IntervalConsumptionPolicyV1::LargestOnly => slots
            .intervals
            .iter()
            .copied()
            .max_by(|left, right| {
                left.width_emu()
                    .cmp(&right.width_emu())
                    .then_with(|| right.x0_emu.cmp(&left.x0_emu))
            })
            .into_iter()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> FrameRegionV1 {
        FrameRegionV1 {
            frame_id: "f1".into(),
            page_id: "p1".into(),
            bounds: RectEmuV1 {
                x: 100,
                y: 200,
                width: 1000,
                height: 800,
            },
            insets: InsetsEmuV1 {
                left: 100,
                top: 50,
                right: 100,
                bottom: 50,
            },
        }
    }

    fn band() -> LineBandRequestV1 {
        LineBandRequestV1 {
            top_emu: 300,
            bottom_emu: 340,
        }
    }

    fn rect_obstacle(id: &str, rect: RectEmuV1) -> EffectiveWrapObstacleV1 {
        EffectiveWrapObstacleV1 {
            obstacle_id: id.into(),
            mode: EffectiveWrapModeV1::Rectangle {
                rect,
                distances: WrapDistancesV1 {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
            },
        }
    }

    #[test]
    fn no_obstacles_returns_full_interior_interval() {
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[],
            band(),
        )
        .expect("slots");
        assert_eq!(
            slots.intervals,
            vec![LineIntervalV1 {
                column_index: 0,
                x0_emu: 200,
                x1_emu: 1000,
            }]
        );
    }

    #[test]
    fn middle_rectangle_splits_into_two_ordered_intervals() {
        let obstacle = rect_obstacle(
            "o1",
            RectEmuV1 {
                x: 450,
                y: 280,
                width: 200,
                height: 100,
            },
        );
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[obstacle],
            band(),
        )
        .expect("slots");
        assert_eq!(
            slots.intervals,
            vec![
                LineIntervalV1 {
                    column_index: 0,
                    x0_emu: 200,
                    x1_emu: 450,
                },
                LineIntervalV1 {
                    column_index: 0,
                    x0_emu: 650,
                    x1_emu: 1000,
                },
            ]
        );
    }

    #[test]
    fn overlapping_obstacles_and_fully_blocked_band_are_deterministic() {
        let obstacles = vec![
            rect_obstacle(
                "a",
                RectEmuV1 {
                    x: 100,
                    y: 250,
                    width: 500,
                    height: 200,
                },
            ),
            rect_obstacle(
                "b",
                RectEmuV1 {
                    x: 500,
                    y: 250,
                    width: 700,
                    height: 200,
                },
            ),
        ];
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &obstacles,
            band(),
        )
        .expect("slots");
        assert!(slots.intervals.is_empty());
        let again = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &obstacles,
            band(),
        )
        .expect("again");
        assert_eq!(slots, again);
    }

    #[test]
    fn wrap_distances_expand_exclusion() {
        let obstacle = EffectiveWrapObstacleV1 {
            obstacle_id: "o".into(),
            mode: EffectiveWrapModeV1::Rectangle {
                rect: RectEmuV1 {
                    x: 500,
                    y: 310,
                    width: 100,
                    height: 10,
                },
                distances: WrapDistancesV1 {
                    left: 20,
                    top: 20,
                    right: 30,
                    bottom: 20,
                },
            },
        };
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[obstacle],
            band(),
        )
        .expect("slots");
        assert_eq!(slots.intervals[0].x1_emu, 480);
        assert_eq!(slots.intervals[1].x0_emu, 630);
    }

    #[test]
    fn obstacle_outside_frame_has_no_effect() {
        let obstacle = rect_obstacle(
            "outside",
            RectEmuV1 {
                x: 5000,
                y: 300,
                width: 100,
                height: 100,
            },
        );
        let base = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[],
            band(),
        )
        .expect("base");
        let with = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[obstacle],
            band(),
        )
        .expect("with");
        assert_eq!(base.intervals, with.intervals);
    }

    #[test]
    fn columns_and_gutter_are_semantic_geometry_inputs() {
        let columns = resolve_columns_v1(
            &frame(),
            ColumnsV1 {
                count: 3,
                gutter_emu: 20,
            },
        )
        .expect("columns");
        assert_eq!(3, columns.len());
        assert_eq!(200, columns[0].x);
        assert_eq!(1000, columns[2].right().expect("right"));
        assert_eq!(20, columns[1].x - columns[0].right().expect("right"));
        assert_eq!(20, columns[2].x - columns[1].right().expect("right"));

        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 3,
                gutter_emu: 20,
            },
            &[],
            band(),
        )
        .expect("slots");
        assert_eq!(
            vec![0, 1, 2],
            slots
                .intervals
                .iter()
                .map(|v| v.column_index)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unsupported_effective_wrap_is_diagnostic_not_approximation() {
        let obstacle = EffectiveWrapObstacleV1 {
            obstacle_id: "contour".into(),
            mode: EffectiveWrapModeV1::Unsupported {
                semantic_kind: "contour-left-only".into(),
            },
        };
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[obstacle],
            band(),
        )
        .expect("slots");
        assert_eq!(1, slots.intervals.len());
        assert_eq!(1, slots.diagnostics.len());
        assert_eq!(
            "line_region.unsupported_wrap_semantics",
            slots.diagnostics[0].code
        );
    }

    #[test]
    fn band_is_clipped_to_frame_interior() {
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[],
            LineBandRequestV1 {
                top_emu: 0,
                bottom_emu: 300,
            },
        )
        .expect("slots");
        assert_eq!(250, slots.clipped_top_emu);
        assert_eq!(300, slots.clipped_bottom_emu);
    }

    #[test]
    fn outside_band_returns_zero_intervals_with_diagnostic() {
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[],
            LineBandRequestV1 {
                top_emu: 10,
                bottom_emu: 20,
            },
        )
        .expect("slots");
        assert!(slots.intervals.is_empty());
        assert_eq!("line_region.band_outside_frame", slots.diagnostics[0].code);
    }

    #[test]
    fn consumption_policy_is_explicit_and_not_publisher_side_semantics() {
        let obstacle = rect_obstacle(
            "middle",
            RectEmuV1 {
                x: 450,
                y: 280,
                width: 200,
                height: 100,
            },
        );
        let slots = resolve_line_band_v1(
            &frame(),
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[obstacle],
            band(),
        )
        .expect("slots");

        let all =
            select_line_intervals_v1(&slots, IntervalConsumptionPolicyV1::AllIntervalsLeftToRight);
        let largest = select_line_intervals_v1(&slots, IntervalConsumptionPolicyV1::LargestOnly);
        assert_eq!(2, all.len());
        assert_eq!(1, largest.len());
        assert_eq!(650, largest[0].x0_emu);
    }

    #[test]
    fn invalid_geometry_and_overflow_fail_closed() {
        let mut bad = frame();
        bad.insets.left = 900;
        bad.insets.right = 900;
        assert_eq!(
            Err(LineRegionErrorV1::InvalidInsets),
            resolve_columns_v1(
                &bad,
                ColumnsV1 {
                    count: 1,
                    gutter_emu: 0,
                }
            )
        );

        let overflow = FrameRegionV1 {
            frame_id: "f".into(),
            page_id: "p".into(),
            bounds: RectEmuV1 {
                x: i64::MAX - 2,
                y: 0,
                width: 10,
                height: 10,
            },
            insets: InsetsEmuV1 {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
        };
        assert_eq!(
            Err(LineRegionErrorV1::CoordinateOverflow),
            resolve_columns_v1(
                &overflow,
                ColumnsV1 {
                    count: 1,
                    gutter_emu: 0,
                }
            )
        );
    }
}
