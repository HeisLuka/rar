use crate::Decimal;
use serde::{Deserialize, Serialize};

pub const EMU_PER_INCH: i64 = 914_400;
pub const EMU_PER_POINT: i64 = 12_700;
pub const EMU_PER_MILLIMETER: i64 = 36_000;
pub const EMU_PER_CSS_PIXEL_96_DPI: i64 = 9_525;

/// Signed 64-bit physical length in English Metric Units.
///
/// Отрицательные значения разрешены: canonical coordinate space допускает
/// off-page content, bleed/crop offsets и другие координаты вне page origin.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LengthEmu(pub i64);

impl LengthEmu {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }
}

/// Прямоугольник в canonical EMU coordinate space.
///
/// Primitive не навязывает positivity width/height: конкретные сущности
/// (например Page.size) валидируют свои более сильные invariants отдельно.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectEmu {
    pub x: LengthEmu,
    pub y: LengthEmu,
    pub width: LengthEmu,
    pub height: LengthEmu,
}

impl RectEmu {
    pub const fn new(x: LengthEmu, y: LengthEmu, width: LengthEmu, height: LengthEmu) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> Option<LengthEmu> {
        self.x.checked_add(self.width)
    }

    pub fn bottom(self) -> Option<LengthEmu> {
        self.y.checked_add(self.height)
    }
}

/// Affine transform из local node coordinates в parent coordinates.
///
/// Коэффициенты linear part используют exact Decimal storage; translation
/// хранится в EMU. Ни один компонент transform не использует f32/f64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Affine2D {
    pub a: Decimal,
    pub b: Decimal,
    pub c: Decimal,
    pub d: Decimal,
    pub tx: LengthEmu,
    pub ty: LengthEmu,
}

impl Affine2D {
    pub fn identity() -> Self {
        Self {
            a: Decimal::one(),
            b: Decimal::zero(),
            c: Decimal::zero(),
            d: Decimal::one(),
            tx: LengthEmu::ZERO,
            ty: LengthEmu::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_unit_constants_match_cdm_contract() {
        assert_eq!(EMU_PER_INCH, 914_400);
        assert_eq!(EMU_PER_POINT, 12_700);
        assert_eq!(EMU_PER_MILLIMETER, 36_000);
        assert_eq!(EMU_PER_CSS_PIXEL_96_DPI, 9_525);
    }

    #[test]
    fn length_emu_is_signed_and_checked() {
        assert_eq!(LengthEmu::new(-10).get(), -10);
        assert_eq!(
            LengthEmu::new(100).checked_add(LengthEmu::new(23)),
            Some(LengthEmu::new(123))
        );
        assert_eq!(
            LengthEmu::new(i64::MAX).checked_add(LengthEmu::new(1)),
            None
        );
    }

    #[test]
    fn rect_supports_negative_origin_without_floating_point() {
        let rect = RectEmu::new(
            LengthEmu::new(-36_000),
            LengthEmu::new(-36_000),
            LengthEmu::new(914_400),
            LengthEmu::new(1_828_800),
        );

        assert_eq!(rect.right(), Some(LengthEmu::new(878_400)));
        assert_eq!(rect.bottom(), Some(LengthEmu::new(1_792_800)));
    }

    #[test]
    fn affine_identity_uses_exact_decimal_coefficients() {
        let transform = Affine2D::identity();

        assert_eq!(transform.a.as_str(), "1");
        assert_eq!(transform.b.as_str(), "0");
        assert_eq!(transform.c.as_str(), "0");
        assert_eq!(transform.d.as_str(), "1");
        assert_eq!(transform.tx, LengthEmu::ZERO);
        assert_eq!(transform.ty, LengthEmu::ZERO);
    }
}
