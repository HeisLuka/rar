use serde::{Deserialize, Serialize};

use crate::{
    ShapePaintProvenanceV1, ShapePaintV1, ShapePaintValidationError, SolidFillV1, SolidStrokeV1,
    validate_shape_paint_v1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetFillV1 {
    pub node_id: String,
    pub before: SolidFillV1,
    pub after: SolidFillV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetStrokeV1 {
    pub node_id: String,
    pub before: SolidStrokeV1,
    pub after: SolidStrokeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapePaintOperationV1 {
    SetFill(SetFillV1),
    SetStroke(SetStrokeV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapePaintOperationError {
    InvalidCurrentPaint(ShapePaintValidationError),
    EmptyNodeId,
    SourceBackedTargetNotEnabled,
    MissingCurrentFill,
    MissingCurrentStroke,
    StaleFillBefore,
    StaleStrokeBefore,
    NoOp,
    InvalidAfterPaint(ShapePaintValidationError),
}

pub fn apply_shape_paint_operation_v1(
    current: &ShapePaintV1,
    operation: &ShapePaintOperationV1,
) -> Result<ShapePaintV1, ShapePaintOperationError> {
    validate_shape_paint_v1(current).map_err(ShapePaintOperationError::InvalidCurrentPaint)?;

    if !matches!(current.provenance, ShapePaintProvenanceV1::AuthorCreated) {
        return Err(ShapePaintOperationError::SourceBackedTargetNotEnabled);
    }

    match operation {
        ShapePaintOperationV1::SetFill(operation) => {
            validate_node_id(&operation.node_id)?;
            if operation.before == operation.after {
                return Err(ShapePaintOperationError::NoOp);
            }
            let Some(current_fill) = current.fill.as_ref() else {
                return Err(ShapePaintOperationError::MissingCurrentFill);
            };
            if current_fill != &operation.before {
                return Err(ShapePaintOperationError::StaleFillBefore);
            }

            let mut next = current.clone();
            next.fill = Some(operation.after.clone());
            validate_shape_paint_v1(&next).map_err(ShapePaintOperationError::InvalidAfterPaint)?;
            Ok(next)
        }
        ShapePaintOperationV1::SetStroke(operation) => {
            validate_node_id(&operation.node_id)?;
            if operation.before == operation.after {
                return Err(ShapePaintOperationError::NoOp);
            }
            let Some(current_stroke) = current.stroke.as_ref() else {
                return Err(ShapePaintOperationError::MissingCurrentStroke);
            };
            if current_stroke != &operation.before {
                return Err(ShapePaintOperationError::StaleStrokeBefore);
            }

            if operation.after.width_emu <= 0 {
                return Err(ShapePaintOperationError::InvalidAfterPaint(
                    ShapePaintValidationError::NonPositiveStrokeWidth {
                        width_emu: operation.after.width_emu,
                    },
                ));
            }

            let mut next = current.clone();
            next.stroke = Some(operation.after.clone());
            validate_shape_paint_v1(&next).map_err(ShapePaintOperationError::InvalidAfterPaint)?;
            Ok(next)
        }
    }
}

pub fn inverse_shape_paint_operation_v1(
    operation: &ShapePaintOperationV1,
) -> ShapePaintOperationV1 {
    match operation {
        ShapePaintOperationV1::SetFill(operation) => ShapePaintOperationV1::SetFill(SetFillV1 {
            node_id: operation.node_id.clone(),
            before: operation.after.clone(),
            after: operation.before.clone(),
        }),
        ShapePaintOperationV1::SetStroke(operation) => {
            ShapePaintOperationV1::SetStroke(SetStrokeV1 {
                node_id: operation.node_id.clone(),
                before: operation.after.clone(),
                after: operation.before.clone(),
            })
        }
    }
}

fn validate_node_id(node_id: &str) -> Result<(), ShapePaintOperationError> {
    if node_id.is_empty() {
        Err(ShapePaintOperationError::EmptyNodeId)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Srgb8, author_created_shape_paint_v1};

    fn fill(r: u8, g: u8, b: u8, visible: bool) -> SolidFillV1 {
        SolidFillV1 {
            visible,
            color: Srgb8 { r, g, b },
        }
    }

    fn stroke(r: u8, g: u8, b: u8, width_emu: i64, visible: bool) -> SolidStrokeV1 {
        SolidStrokeV1 {
            visible,
            color: Srgb8 { r, g, b },
            width_emu,
        }
    }

    fn authored() -> ShapePaintV1 {
        author_created_shape_paint_v1(
            Some(fill(1, 2, 3, true)),
            Some(stroke(4, 5, 6, 12_700, true)),
        )
        .expect("authored paint")
    }

    #[test]
    fn set_fill_requires_exact_before_and_replays_deterministically() {
        let before = authored();
        let operation = ShapePaintOperationV1::SetFill(SetFillV1 {
            node_id: "shape:1".to_owned(),
            before: fill(1, 2, 3, true),
            after: fill(9, 8, 7, false),
        });

        let after = apply_shape_paint_operation_v1(&before, &operation).expect("set fill");
        assert_eq!(after.fill, Some(fill(9, 8, 7, false)));
        assert_eq!(
            apply_shape_paint_operation_v1(&before, &operation).expect("replay"),
            after
        );

        let stale = apply_shape_paint_operation_v1(&after, &operation);
        assert_eq!(stale, Err(ShapePaintOperationError::StaleFillBefore));
    }

    #[test]
    fn inverse_restores_exact_fill() {
        let before = authored();
        let operation = ShapePaintOperationV1::SetFill(SetFillV1 {
            node_id: "shape:1".to_owned(),
            before: fill(1, 2, 3, true),
            after: fill(20, 30, 40, false),
        });
        let after = apply_shape_paint_operation_v1(&before, &operation).expect("apply");
        let restored =
            apply_shape_paint_operation_v1(&after, &inverse_shape_paint_operation_v1(&operation))
                .expect("inverse");
        assert_eq!(restored, before);
    }

    #[test]
    fn set_stroke_requires_positive_width_and_exact_before() {
        let before = authored();
        let operation = ShapePaintOperationV1::SetStroke(SetStrokeV1 {
            node_id: "shape:1".to_owned(),
            before: stroke(4, 5, 6, 12_700, true),
            after: stroke(7, 8, 9, 25_400, false),
        });
        let after = apply_shape_paint_operation_v1(&before, &operation).expect("set stroke");
        assert_eq!(after.stroke, Some(stroke(7, 8, 9, 25_400, false)));

        let invalid = ShapePaintOperationV1::SetStroke(SetStrokeV1 {
            node_id: "shape:1".to_owned(),
            before: stroke(4, 5, 6, 12_700, true),
            after: stroke(7, 8, 9, 0, false),
        });
        assert_eq!(
            apply_shape_paint_operation_v1(&before, &invalid),
            Err(ShapePaintOperationError::InvalidAfterPaint(
                ShapePaintValidationError::NonPositiveStrokeWidth { width_emu: 0 }
            ))
        );
    }

    #[test]
    fn no_op_and_empty_node_id_fail_closed() {
        let before = authored();
        let no_op = ShapePaintOperationV1::SetFill(SetFillV1 {
            node_id: "shape:1".to_owned(),
            before: fill(1, 2, 3, true),
            after: fill(1, 2, 3, true),
        });
        assert_eq!(
            apply_shape_paint_operation_v1(&before, &no_op),
            Err(ShapePaintOperationError::NoOp)
        );

        let empty_id = ShapePaintOperationV1::SetFill(SetFillV1 {
            node_id: String::new(),
            before: fill(1, 2, 3, true),
            after: fill(2, 3, 4, true),
        });
        assert_eq!(
            apply_shape_paint_operation_v1(&before, &empty_id),
            Err(ShapePaintOperationError::EmptyNodeId)
        );
    }

    #[test]
    fn source_backed_edit_is_explicitly_not_enabled_in_v1() {
        let mut source = authored();
        source.provenance = ShapePaintProvenanceV1::SourceBacked {
            source_ref: crate::SourceRefV1 {
                format: "pub".to_owned(),
                adapter_version: "pub-rs/0.1".to_owned(),
                source_hash_hex: "11".repeat(32),
                carrier: "/Escher/EscherStm".to_owned(),
                object_key: Some("shape/1".to_owned()),
                path: Some("SpContainer/FOPT".to_owned()),
                role: crate::SourceRoleV1::Semantic,
                authority: crate::AuthorityClassV1::Authoritative,
                confidence: crate::ReadConfidenceV1::Exact,
            },
        };
        let operation = ShapePaintOperationV1::SetFill(SetFillV1 {
            node_id: "shape:source".to_owned(),
            before: fill(1, 2, 3, true),
            after: fill(9, 9, 9, true),
        });
        assert_eq!(
            apply_shape_paint_operation_v1(&source, &operation),
            Err(ShapePaintOperationError::SourceBackedTargetNotEnabled)
        );
    }
}
