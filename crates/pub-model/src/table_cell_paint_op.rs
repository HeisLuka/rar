use serde::{Deserialize, Serialize};

use crate::{
    SolidFillV1, SolidStrokeV1, TableCellBorderSideV1, TableCellPaintV1,
    TableCellPaintValidationError, TableCellId, validate_table_cell_paint_v1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTableCellFillV1 {
    pub table_cell_id: TableCellId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<SolidFillV1>,
    pub after: SolidFillV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearTableCellFillV1 {
    pub table_cell_id: TableCellId,
    pub before: SolidFillV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTableCellBorderSideV1 {
    pub table_cell_id: TableCellId,
    pub side: TableCellBorderSideV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<SolidStrokeV1>,
    pub after: SolidStrokeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearTableCellBorderSideV1 {
    pub table_cell_id: TableCellId,
    pub side: TableCellBorderSideV1,
    pub before: SolidStrokeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableCellPaintOperationV1 {
    SetTableCellFill(SetTableCellFillV1),
    ClearTableCellFill(ClearTableCellFillV1),
    SetTableCellBorderSide(SetTableCellBorderSideV1),
    ClearTableCellBorderSide(ClearTableCellBorderSideV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableCellPaintOperationError {
    InvalidCurrentPaint(TableCellPaintValidationError),
    TargetCellMismatch,
    StaleFillBefore,
    StaleBorderBefore { side: TableCellBorderSideV1 },
    NoOp,
    InvalidAfterPaint(TableCellPaintValidationError),
}

pub fn apply_table_cell_paint_operation_v1(
    current: &TableCellPaintV1,
    operation: &TableCellPaintOperationV1,
) -> Result<TableCellPaintV1, TableCellPaintOperationError> {
    validate_table_cell_paint_v1(current)
        .map_err(TableCellPaintOperationError::InvalidCurrentPaint)?;

    let target = operation_table_cell_id(operation);
    if target != &current.cell_id {
        return Err(TableCellPaintOperationError::TargetCellMismatch);
    }

    let mut next = current.clone();
    match operation {
        TableCellPaintOperationV1::SetTableCellFill(operation) => {
            if current.fill != operation.before {
                return Err(TableCellPaintOperationError::StaleFillBefore);
            }
            if operation.before.as_ref() == Some(&operation.after) {
                return Err(TableCellPaintOperationError::NoOp);
            }
            next.fill = Some(operation.after.clone());
        }
        TableCellPaintOperationV1::ClearTableCellFill(operation) => {
            if current.fill.as_ref() != Some(&operation.before) {
                return Err(TableCellPaintOperationError::StaleFillBefore);
            }
            next.fill = None;
        }
        TableCellPaintOperationV1::SetTableCellBorderSide(operation) => {
            if current.borders.side(operation.side) != operation.before.as_ref() {
                return Err(TableCellPaintOperationError::StaleBorderBefore {
                    side: operation.side,
                });
            }
            if operation.before.as_ref() == Some(&operation.after) {
                return Err(TableCellPaintOperationError::NoOp);
            }
            next.borders
                .set_side(operation.side, Some(operation.after.clone()));
        }
        TableCellPaintOperationV1::ClearTableCellBorderSide(operation) => {
            if current.borders.side(operation.side) != Some(&operation.before) {
                return Err(TableCellPaintOperationError::StaleBorderBefore {
                    side: operation.side,
                });
            }
            next.borders.set_side(operation.side, None);
        }
    }

    validate_table_cell_paint_v1(&next)
        .map_err(TableCellPaintOperationError::InvalidAfterPaint)?;
    Ok(next)
}

pub fn inverse_table_cell_paint_operation_v1(
    operation: &TableCellPaintOperationV1,
) -> TableCellPaintOperationV1 {
    match operation {
        TableCellPaintOperationV1::SetTableCellFill(operation) => {
            match &operation.before {
                Some(before) => TableCellPaintOperationV1::SetTableCellFill(
                    SetTableCellFillV1 {
                        table_cell_id: operation.table_cell_id.clone(),
                        before: Some(operation.after.clone()),
                        after: before.clone(),
                    },
                ),
                None => TableCellPaintOperationV1::ClearTableCellFill(
                    ClearTableCellFillV1 {
                        table_cell_id: operation.table_cell_id.clone(),
                        before: operation.after.clone(),
                    },
                ),
            }
        }
        TableCellPaintOperationV1::ClearTableCellFill(operation) => {
            TableCellPaintOperationV1::SetTableCellFill(SetTableCellFillV1 {
                table_cell_id: operation.table_cell_id.clone(),
                before: None,
                after: operation.before.clone(),
            })
        }
        TableCellPaintOperationV1::SetTableCellBorderSide(operation) => {
            match &operation.before {
                Some(before) => TableCellPaintOperationV1::SetTableCellBorderSide(
                    SetTableCellBorderSideV1 {
                        table_cell_id: operation.table_cell_id.clone(),
                        side: operation.side,
                        before: Some(operation.after.clone()),
                        after: before.clone(),
                    },
                ),
                None => TableCellPaintOperationV1::ClearTableCellBorderSide(
                    ClearTableCellBorderSideV1 {
                        table_cell_id: operation.table_cell_id.clone(),
                        side: operation.side,
                        before: operation.after.clone(),
                    },
                ),
            }
        }
        TableCellPaintOperationV1::ClearTableCellBorderSide(operation) => {
            TableCellPaintOperationV1::SetTableCellBorderSide(SetTableCellBorderSideV1 {
                table_cell_id: operation.table_cell_id.clone(),
                side: operation.side,
                before: None,
                after: operation.before.clone(),
            })
        }
    }
}

fn operation_table_cell_id(operation: &TableCellPaintOperationV1) -> &TableCellId {
    match operation {
        TableCellPaintOperationV1::SetTableCellFill(operation) => &operation.table_cell_id,
        TableCellPaintOperationV1::ClearTableCellFill(operation) => &operation.table_cell_id,
        TableCellPaintOperationV1::SetTableCellBorderSide(operation) => &operation.table_cell_id,
        TableCellPaintOperationV1::ClearTableCellBorderSide(operation) => &operation.table_cell_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Srgb8, TABLE_CELL_PAINT_SCHEMA_V1, TableCellBordersV1, TableCellClassV1,
    };

    fn id(byte: u8) -> TableCellId {
        TableCellId::parse(&format!(
            "{0:02x}{0:02x}{0:02x}{0:02x}-{0:02x}{0:02x}-{0:02x}{0:02x}-{0:02x}{0:02x}-{0:02x}{0:02x}{0:02x}{0:02x}{0:02x}{0:02x}",
            byte
        ))
        .expect("id")
    }

    fn fill(r: u8) -> SolidFillV1 {
        SolidFillV1 {
            visible: true,
            color: Srgb8 { r, g: 2, b: 3 },
        }
    }

    fn border(width_emu: i64) -> SolidStrokeV1 {
        SolidStrokeV1 {
            visible: true,
            color: Srgb8 { r: 4, g: 5, b: 6 },
            width_emu,
        }
    }

    fn paint() -> TableCellPaintV1 {
        TableCellPaintV1 {
            schema_version: TABLE_CELL_PAINT_SCHEMA_V1.to_owned(),
            cell_id: id(1),
            table_class: TableCellClassV1::SimpleUnmergedRectangular,
            fill: Some(fill(1)),
            borders: TableCellBordersV1 {
                top: Some(border(12_700)),
                right: None,
                bottom: None,
                left: None,
            },
        }
    }

    #[test]
    fn set_and_clear_fill_use_exact_before_and_inverse() {
        let before = paint();
        let set = TableCellPaintOperationV1::SetTableCellFill(SetTableCellFillV1 {
            table_cell_id: id(1),
            before: Some(fill(1)),
            after: fill(9),
        });
        let after = apply_table_cell_paint_operation_v1(&before, &set).expect("set");
        assert_eq!(after.fill, Some(fill(9)));
        assert_eq!(
            apply_table_cell_paint_operation_v1(&after, &set),
            Err(TableCellPaintOperationError::StaleFillBefore)
        );

        let restored = apply_table_cell_paint_operation_v1(
            &after,
            &inverse_table_cell_paint_operation_v1(&set),
        )
        .expect("inverse");
        assert_eq!(restored, before);

        let clear = TableCellPaintOperationV1::ClearTableCellFill(ClearTableCellFillV1 {
            table_cell_id: id(1),
            before: fill(1),
        });
        let cleared = apply_table_cell_paint_operation_v1(&before, &clear).expect("clear");
        assert!(cleared.fill.is_none());
        assert_eq!(
            apply_table_cell_paint_operation_v1(
                &cleared,
                &inverse_table_cell_paint_operation_v1(&clear)
            )
            .expect("restore clear"),
            before
        );
    }

    #[test]
    fn set_from_absent_and_clear_border_roundtrip() {
        let before = paint();
        let set =
            TableCellPaintOperationV1::SetTableCellBorderSide(SetTableCellBorderSideV1 {
                table_cell_id: id(1),
                side: TableCellBorderSideV1::Right,
                before: None,
                after: border(25_400),
            });
        let after = apply_table_cell_paint_operation_v1(&before, &set).expect("set right");
        assert_eq!(
            after.borders.right.as_ref().map(|value| value.width_emu),
            Some(25_400)
        );
        let restored = apply_table_cell_paint_operation_v1(
            &after,
            &inverse_table_cell_paint_operation_v1(&set),
        )
        .expect("inverse");
        assert_eq!(restored, before);
    }

    #[test]
    fn four_sides_are_mutated_independently() {
        let mut current = TableCellPaintV1 {
            schema_version: TABLE_CELL_PAINT_SCHEMA_V1.to_owned(),
            cell_id: id(1),
            table_class: TableCellClassV1::SimpleUnmergedRectangular,
            fill: None,
            borders: TableCellBordersV1::default(),
        };
        for (index, side) in [
            TableCellBorderSideV1::Top,
            TableCellBorderSideV1::Right,
            TableCellBorderSideV1::Bottom,
            TableCellBorderSideV1::Left,
        ]
        .into_iter()
        .enumerate()
        {
            current = apply_table_cell_paint_operation_v1(
                &current,
                &TableCellPaintOperationV1::SetTableCellBorderSide(
                    SetTableCellBorderSideV1 {
                        table_cell_id: id(1),
                        side,
                        before: None,
                        after: border((index as i64 + 1) * 100),
                    },
                ),
            )
            .expect("set side");
        }
        assert_eq!(
            [
                current.borders.top.as_ref().unwrap().width_emu,
                current.borders.right.as_ref().unwrap().width_emu,
                current.borders.bottom.as_ref().unwrap().width_emu,
                current.borders.left.as_ref().unwrap().width_emu,
            ],
            [100, 200, 300, 400]
        );
    }

    #[test]
    fn target_stale_noop_and_invalid_width_fail_closed() {
        let before = paint();
        let wrong_target =
            TableCellPaintOperationV1::SetTableCellFill(SetTableCellFillV1 {
                table_cell_id: id(2),
                before: Some(fill(1)),
                after: fill(2),
            });
        assert_eq!(
            apply_table_cell_paint_operation_v1(&before, &wrong_target),
            Err(TableCellPaintOperationError::TargetCellMismatch)
        );

        let no_op = TableCellPaintOperationV1::SetTableCellFill(SetTableCellFillV1 {
            table_cell_id: id(1),
            before: Some(fill(1)),
            after: fill(1),
        });
        assert_eq!(
            apply_table_cell_paint_operation_v1(&before, &no_op),
            Err(TableCellPaintOperationError::NoOp)
        );

        let invalid =
            TableCellPaintOperationV1::SetTableCellBorderSide(SetTableCellBorderSideV1 {
                table_cell_id: id(1),
                side: TableCellBorderSideV1::Right,
                before: None,
                after: border(0),
            });
        assert_eq!(
            apply_table_cell_paint_operation_v1(&before, &invalid),
            Err(TableCellPaintOperationError::InvalidAfterPaint(
                TableCellPaintValidationError::NonPositiveBorderWidth {
                    side: TableCellBorderSideV1::Right,
                    width_emu: 0,
                }
            ))
        );
    }
}
