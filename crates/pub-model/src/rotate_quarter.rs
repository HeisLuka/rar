use serde::{Deserialize, Serialize};

use crate::{
    AuthoredShapeV1, EntityProvenanceV1, ShapeKindV1, ShapeTransformV1,
    validate_rect_emu_v1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactAffineV1 {
    pub a: String,
    pub b: String,
    pub c: String,
    pub d: String,
    pub tx: String,
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotateQuarterResultV1 {
    pub node_id: String,
    pub before: ExactAffineV1,
    pub after: ExactAffineV1,
    pub pivot_x: String,
    pub pivot_y: String,
    pub quarter_turns: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotateQuarterError {
    UnsupportedTarget,
    InvalidBounds,
    UnsupportedTransform,
    NonCanonicalAffine,
    FlippedTransform,
    StaleExpectedBefore,
    FullTurnNoOp,
    ArithmeticOverflow,
}

fn checked_add(left: i128, right: i128) -> Result<i128, RotateQuarterError> {
    left.checked_add(right)
        .ok_or(RotateQuarterError::ArithmeticOverflow)
}

fn checked_sub(left: i128, right: i128) -> Result<i128, RotateQuarterError> {
    left.checked_sub(right)
        .ok_or(RotateQuarterError::ArithmeticOverflow)
}

fn checked_mul(left: i128, right: i128) -> Result<i128, RotateQuarterError> {
    left.checked_mul(right)
        .ok_or(RotateQuarterError::ArithmeticOverflow)
}

fn parse_half_scaled2(value: &str) -> Result<i128, RotateQuarterError> {
    if value.is_empty() || value.starts_with('+') {
        return Err(RotateQuarterError::NonCanonicalAffine);
    }

    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    if unsigned.is_empty() {
        return Err(RotateQuarterError::NonCanonicalAffine);
    }

    let (integer_part, half) = match unsigned.strip_suffix(".5") {
        Some(prefix) => (prefix, true),
        None => (unsigned, false),
    };
    if integer_part.is_empty()
        || !integer_part.bytes().all(|byte| byte.is_ascii_digit())
        || (integer_part.len() > 1 && integer_part.starts_with('0'))
    {
        return Err(RotateQuarterError::NonCanonicalAffine);
    }

    let integer = integer_part
        .parse::<i128>()
        .map_err(|_| RotateQuarterError::ArithmeticOverflow)?;
    let mut scaled = checked_mul(integer, 2)?;
    if half {
        scaled = checked_add(scaled, 1)?;
    }
    if negative {
        scaled = scaled
            .checked_neg()
            .ok_or(RotateQuarterError::ArithmeticOverflow)?;
    }

    if format_half_scaled2(scaled)? != value {
        return Err(RotateQuarterError::NonCanonicalAffine);
    }
    Ok(scaled)
}

fn format_half_scaled2(value: i128) -> Result<String, RotateQuarterError> {
    if value % 2 == 0 {
        return Ok((value / 2).to_string());
    }

    let negative = value < 0;
    let magnitude = value
        .checked_abs()
        .ok_or(RotateQuarterError::ArithmeticOverflow)?;
    let whole = magnitude / 2;
    Ok(format!(
        "{}{}.5",
        if negative { "-" } else { "" },
        whole
    ))
}

fn affine_scaled2(
    value: &ExactAffineV1,
) -> Result<(i128, i128, i128, i128, i128, i128), RotateQuarterError> {
    let a = parse_half_scaled2(&value.a)?;
    let b = parse_half_scaled2(&value.b)?;
    let c = parse_half_scaled2(&value.c)?;
    let d = parse_half_scaled2(&value.d)?;
    let tx = parse_half_scaled2(&value.tx)?;
    let ty = parse_half_scaled2(&value.ty)?;

    if a % 2 != 0 || b % 2 != 0 || c % 2 != 0 || d % 2 != 0 {
        return Err(RotateQuarterError::UnsupportedTransform);
    }
    let linear = (a / 2, b / 2, c / 2, d / 2);
    if !matches!(
        linear,
        (1, 0, 0, 1) | (0, 1, -1, 0) | (-1, 0, 0, -1) | (0, -1, 1, 0)
    ) {
        let determinant = linear.0 * linear.3 - linear.1 * linear.2;
        if determinant == -1 {
            return Err(RotateQuarterError::FlippedTransform);
        }
        return Err(RotateQuarterError::UnsupportedTransform);
    }
    if linear.0 * linear.3 - linear.1 * linear.2 != 1 {
        return Err(RotateQuarterError::FlippedTransform);
    }

    Ok((a, b, c, d, tx, ty))
}

pub fn validate_exact_affine_v1(
    value: &ExactAffineV1,
) -> Result<(), RotateQuarterError> {
    affine_scaled2(value).map(|_| ())
}

pub fn canonical_shape_affine_v1(
    transform: &ShapeTransformV1,
) -> Result<ExactAffineV1, RotateQuarterError> {
    let affine = match transform {
        ShapeTransformV1::Identity => ExactAffineV1 {
            a: "1".to_owned(),
            b: "0".to_owned(),
            c: "0".to_owned(),
            d: "1".to_owned(),
            tx: "0".to_owned(),
            ty: "0".to_owned(),
        },
        ShapeTransformV1::Affine { a, b, c, d, tx, ty } => ExactAffineV1 {
            a: a.clone(),
            b: b.clone(),
            c: c.clone(),
            d: d.clone(),
            tx: tx.clone(),
            ty: ty.clone(),
        },
    };
    validate_exact_affine_v1(&affine)?;
    Ok(affine)
}

fn affine_from_scaled2(
    values: (i128, i128, i128, i128, i128, i128),
) -> Result<ExactAffineV1, RotateQuarterError> {
    let result = ExactAffineV1 {
        a: format_half_scaled2(values.0)?,
        b: format_half_scaled2(values.1)?,
        c: format_half_scaled2(values.2)?,
        d: format_half_scaled2(values.3)?,
        tx: format_half_scaled2(values.4)?,
        ty: format_half_scaled2(values.5)?,
    };
    validate_exact_affine_v1(&result)?;
    Ok(result)
}

fn scaled_product(left: i128, right: i128) -> Result<i128, RotateQuarterError> {
    let product = checked_mul(left, right)?;
    if product % 2 != 0 {
        return Err(RotateQuarterError::UnsupportedTransform);
    }
    Ok(product / 2)
}

fn compose_affine_v1(
    left: &ExactAffineV1,
    right: &ExactAffineV1,
) -> Result<ExactAffineV1, RotateQuarterError> {
    let (la, lb, lc, ld, ltx, lty) = affine_scaled2(left)?;
    let (ra, rb, rc, rd, rtx, rty) = affine_scaled2(right)?;

    let a = checked_add(scaled_product(la, ra)?, scaled_product(lc, rb)?)?;
    let b = checked_add(scaled_product(lb, ra)?, scaled_product(ld, rb)?)?;
    let c = checked_add(scaled_product(la, rc)?, scaled_product(lc, rd)?)?;
    let d = checked_add(scaled_product(lb, rc)?, scaled_product(ld, rd)?)?;
    let tx = checked_add(
        checked_add(scaled_product(la, rtx)?, scaled_product(lc, rty)?)?,
        ltx,
    )?;
    let ty = checked_add(
        checked_add(scaled_product(lb, rtx)?, scaled_product(ld, rty)?)?,
        lty,
    )?;

    affine_from_scaled2((a, b, c, d, tx, ty))
}

fn quarter_turn_affine_v1(
    pivot_x2: i128,
    pivot_y2: i128,
    quarter_turns: u8,
) -> Result<ExactAffineV1, RotateQuarterError> {
    match quarter_turns {
        1 => affine_from_scaled2((
            0,
            2,
            -2,
            0,
            checked_add(pivot_x2, pivot_y2)?,
            checked_sub(pivot_y2, pivot_x2)?,
        )),
        2 => affine_from_scaled2((
            -2,
            0,
            0,
            -2,
            checked_mul(pivot_x2, 2)?,
            checked_mul(pivot_y2, 2)?,
        )),
        3 => affine_from_scaled2((
            0,
            -2,
            2,
            0,
            checked_sub(pivot_x2, pivot_y2)?,
            checked_add(pivot_y2, pivot_x2)?,
        )),
        _ => Err(RotateQuarterError::FullTurnNoOp),
    }
}

pub fn apply_authored_shape_quarter_turn_v1(
    shape: &AuthoredShapeV1,
    expected_before: &ExactAffineV1,
    quarter_turns: i32,
) -> Result<(AuthoredShapeV1, RotateQuarterResultV1), RotateQuarterError> {
    if shape.shape_kind != ShapeKindV1::Rectangle
        || shape.provenance != EntityProvenanceV1::AuthorCreated
        || shape.parent_id != shape.page_id
    {
        return Err(RotateQuarterError::UnsupportedTarget);
    }
    validate_rect_emu_v1(shape.bounds).map_err(|_| RotateQuarterError::InvalidBounds)?;
    validate_exact_affine_v1(expected_before)?;

    let before = canonical_shape_affine_v1(&shape.transform)?;
    if &before != expected_before {
        return Err(RotateQuarterError::StaleExpectedBefore);
    }

    let normalized = quarter_turns.rem_euclid(4) as u8;
    if normalized == 0 {
        return Err(RotateQuarterError::FullTurnNoOp);
    }

    let pivot_x2 = checked_add(checked_mul(i128::from(shape.bounds.x), 2)?, i128::from(shape.bounds.width))?;
    let pivot_y2 = checked_add(checked_mul(i128::from(shape.bounds.y), 2)?, i128::from(shape.bounds.height))?;
    let turn = quarter_turn_affine_v1(pivot_x2, pivot_y2, normalized)?;
    let after = compose_affine_v1(&turn, &before)?;
    if after == before {
        return Err(RotateQuarterError::FullTurnNoOp);
    }

    let mut resulting = shape.clone();
    resulting.transform = ShapeTransformV1::Affine {
        a: after.a.clone(),
        b: after.b.clone(),
        c: after.c.clone(),
        d: after.d.clone(),
        tx: after.tx.clone(),
        ty: after.ty.clone(),
    };

    let result = RotateQuarterResultV1 {
        node_id: shape.node_id.clone(),
        before,
        after,
        pivot_x: format_half_scaled2(pivot_x2)?,
        pivot_y: format_half_scaled2(pivot_y2)?,
        quarter_turns: normalized,
    };
    Ok((resulting, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RectEmuV1, SolidFillV1, SolidStrokeV1, Srgb8, author_created_shape_paint_v1,
    };

    fn identity() -> ExactAffineV1 {
        ExactAffineV1 {
            a: "1".into(),
            b: "0".into(),
            c: "0".into(),
            d: "1".into(),
            tx: "0".into(),
            ty: "0".into(),
        }
    }

    fn shape() -> AuthoredShapeV1 {
        AuthoredShapeV1 {
            node_id: "01890f47-0c00-7abc-8def-0123456789ab".into(),
            page_id: "page:1".into(),
            parent_id: "page:1".into(),
            shape_kind: ShapeKindV1::Rectangle,
            bounds: RectEmuV1 {
                x: 10,
                y: 20,
                width: 101,
                height: 51,
            },
            transform: ShapeTransformV1::Identity,
            paint: author_created_shape_paint_v1(
                Some(SolidFillV1 {
                    visible: true,
                    color: Srgb8 { r: 1, g: 2, b: 3 },
                }),
                Some(SolidStrokeV1 {
                    visible: true,
                    color: Srgb8 { r: 4, g: 5, b: 6 },
                    width_emu: 12_700,
                }),
            )
            .unwrap(),
            provenance: EntityProvenanceV1::AuthorCreated,
        }
    }

    #[test]
    fn clockwise_quarter_turn_uses_exact_half_emu_center() {
        let (rotated, receipt) =
            apply_authored_shape_quarter_turn_v1(&shape(), &identity(), 1).unwrap();
        assert_eq!(receipt.pivot_x, "60.5");
        assert_eq!(receipt.pivot_y, "45.5");
        assert_eq!(receipt.quarter_turns, 1);
        assert_eq!(
            receipt.after,
            ExactAffineV1 {
                a: "0".into(),
                b: "1".into(),
                c: "-1".into(),
                d: "0".into(),
                tx: "106".into(),
                ty: "-15".into(),
            }
        );
        assert_eq!(canonical_shape_affine_v1(&rotated.transform).unwrap(), receipt.after);
    }

    #[test]
    fn signed_negative_turn_canonicalizes_to_three() {
        let (_, receipt) =
            apply_authored_shape_quarter_turn_v1(&shape(), &identity(), -1).unwrap();
        assert_eq!(receipt.quarter_turns, 3);
        assert_eq!(
            receipt.after,
            ExactAffineV1 {
                a: "0".into(),
                b: "-1".into(),
                c: "1".into(),
                d: "0".into(),
                tx: "15".into(),
                ty: "106".into(),
            }
        );
    }

    #[test]
    fn four_successive_quarters_cycle_to_explicit_identity() {
        let mut current = shape();
        let mut expected = identity();
        for _ in 0..4 {
            let (next, receipt) =
                apply_authored_shape_quarter_turn_v1(&current, &expected, 1).unwrap();
            current = next;
            expected = receipt.after;
        }
        assert_eq!(expected, identity());
        assert!(matches!(current.transform, ShapeTransformV1::Affine { .. }));
    }

    #[test]
    fn stale_precondition_full_turn_group_parent_and_flip_fail_closed() {
        let mut stale = identity();
        stale.tx = "1".into();
        assert_eq!(
            apply_authored_shape_quarter_turn_v1(&shape(), &stale, 1),
            Err(RotateQuarterError::StaleExpectedBefore)
        );
        assert_eq!(
            apply_authored_shape_quarter_turn_v1(&shape(), &identity(), 4),
            Err(RotateQuarterError::FullTurnNoOp)
        );

        let mut grouped = shape();
        grouped.parent_id = "group:1".into();
        assert_eq!(
            apply_authored_shape_quarter_turn_v1(&grouped, &identity(), 1),
            Err(RotateQuarterError::UnsupportedTarget)
        );

        let mut flipped = shape();
        flipped.transform = ShapeTransformV1::Affine {
            a: "-1".into(),
            b: "0".into(),
            c: "0".into(),
            d: "1".into(),
            tx: "0".into(),
            ty: "0".into(),
        };
        assert_eq!(
            canonical_shape_affine_v1(&flipped.transform),
            Err(RotateQuarterError::FlippedTransform)
        );
    }
}
