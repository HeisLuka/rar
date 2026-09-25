use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{SolidFillV1, SolidStrokeV1, TableCellId};

pub const TABLE_CELL_PAINT_SCHEMA_V1: &str = "chaptera.table-cell-paint.v1";
pub const PUBLISHER16_TABLE_CELL_PAINT_AUTHORITY_V1: &str = "pub-t-595/run522";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableCellClassV1 {
    SimpleUnmergedRectangular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableCellBorderSideV1 {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TableCellBordersV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<SolidStrokeV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<SolidStrokeV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<SolidStrokeV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<SolidStrokeV1>,
}

impl TableCellBordersV1 {
    pub fn side(&self, side: TableCellBorderSideV1) -> Option<&SolidStrokeV1> {
        match side {
            TableCellBorderSideV1::Top => self.top.as_ref(),
            TableCellBorderSideV1::Right => self.right.as_ref(),
            TableCellBorderSideV1::Bottom => self.bottom.as_ref(),
            TableCellBorderSideV1::Left => self.left.as_ref(),
        }
    }

    pub(crate) fn set_side(&mut self, side: TableCellBorderSideV1, value: Option<SolidStrokeV1>) {
        match side {
            TableCellBorderSideV1::Top => self.top = value,
            TableCellBorderSideV1::Right => self.right = value,
            TableCellBorderSideV1::Bottom => self.bottom = value,
            TableCellBorderSideV1::Left => self.left = value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCellPaintV1 {
    pub schema_version: String,
    pub cell_id: TableCellId,
    pub table_class: TableCellClassV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<SolidFillV1>,
    #[serde(default)]
    pub borders: TableCellBordersV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publisher16TableCellPaintObservationV1 {
    pub authority: String,
    pub simple_unmerged_rectangular: bool,
    pub cell_id: TableCellId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<SolidFillV1>,
    #[serde(default)]
    pub borders: TableCellBordersV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableCellPaintValidationError {
    InvalidSchema,
    NonPositiveBorderWidth {
        side: TableCellBorderSideV1,
        width_emu: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Publisher16TableCellPaintImportError {
    WrongAuthority,
    UnsupportedTableClass,
    InvalidPaint(TableCellPaintValidationError),
}

pub fn validate_table_cell_paint_v1(
    paint: &TableCellPaintV1,
) -> Result<(), TableCellPaintValidationError> {
    if paint.schema_version != TABLE_CELL_PAINT_SCHEMA_V1 {
        return Err(TableCellPaintValidationError::InvalidSchema);
    }

    for side in [
        TableCellBorderSideV1::Top,
        TableCellBorderSideV1::Right,
        TableCellBorderSideV1::Bottom,
        TableCellBorderSideV1::Left,
    ] {
        if let Some(border) = paint.borders.side(side)
            && border.width_emu <= 0
        {
            return Err(TableCellPaintValidationError::NonPositiveBorderWidth {
                side,
                width_emu: border.width_emu,
            });
        }
    }

    Ok(())
}

pub fn promote_publisher16_table_cell_paint_v1(
    source: &Publisher16TableCellPaintObservationV1,
) -> Result<TableCellPaintV1, Publisher16TableCellPaintImportError> {
    if source.authority != PUBLISHER16_TABLE_CELL_PAINT_AUTHORITY_V1 {
        return Err(Publisher16TableCellPaintImportError::WrongAuthority);
    }
    if !source.simple_unmerged_rectangular {
        return Err(Publisher16TableCellPaintImportError::UnsupportedTableClass);
    }

    let paint = TableCellPaintV1 {
        schema_version: TABLE_CELL_PAINT_SCHEMA_V1.to_owned(),
        cell_id: source.cell_id.clone(),
        table_class: TableCellClassV1::SimpleUnmergedRectangular,
        fill: source.fill.clone(),
        borders: source.borders.clone(),
    };
    validate_table_cell_paint_v1(&paint)
        .map_err(Publisher16TableCellPaintImportError::InvalidPaint)?;
    Ok(paint)
}

pub fn canonical_table_cell_paint_hash_v1(
    paint: &TableCellPaintV1,
) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(paint)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Srgb8;

    fn id() -> TableCellId {
        TableCellId::parse("00112233-4455-6677-8899-aabbccddeeff").expect("id")
    }

    fn stroke(width_emu: i64) -> SolidStrokeV1 {
        SolidStrokeV1 {
            visible: true,
            color: Srgb8 { r: 4, g: 5, b: 6 },
            width_emu,
        }
    }

    #[test]
    fn bounded_publisher16_observation_promotes_without_raw_carriers() {
        let source = Publisher16TableCellPaintObservationV1 {
            authority: PUBLISHER16_TABLE_CELL_PAINT_AUTHORITY_V1.to_owned(),
            simple_unmerged_rectangular: true,
            cell_id: id(),
            fill: Some(SolidFillV1 {
                visible: true,
                color: Srgb8 { r: 1, g: 2, b: 3 },
            }),
            borders: TableCellBordersV1 {
                top: Some(stroke(12_700)),
                right: Some(stroke(25_400)),
                bottom: None,
                left: None,
            },
        };

        let canonical = promote_publisher16_table_cell_paint_v1(&source).expect("promotion");
        assert_eq!(canonical.cell_id, id());
        let json = serde_json::to_string(&canonical).expect("serialize");
        assert!(!json.contains("FOPT"));
        assert!(!json.contains("MCLD"));
        assert!(!json.contains("0x0181"));
        assert!(!json.contains(PUBLISHER16_TABLE_CELL_PAINT_AUTHORITY_V1));
    }

    #[test]
    fn unsupported_table_class_and_unapproved_authority_fail_closed() {
        let mut source = Publisher16TableCellPaintObservationV1 {
            authority: PUBLISHER16_TABLE_CELL_PAINT_AUTHORITY_V1.to_owned(),
            simple_unmerged_rectangular: false,
            cell_id: id(),
            fill: None,
            borders: TableCellBordersV1::default(),
        };
        assert_eq!(
            promote_publisher16_table_cell_paint_v1(&source),
            Err(Publisher16TableCellPaintImportError::UnsupportedTableClass)
        );

        source.simple_unmerged_rectangular = true;
        source.authority = "forged".to_owned();
        assert_eq!(
            promote_publisher16_table_cell_paint_v1(&source),
            Err(Publisher16TableCellPaintImportError::WrongAuthority)
        );
    }

    #[test]
    fn all_four_border_sides_are_independent_and_require_positive_width() {
        let mut paint = TableCellPaintV1 {
            schema_version: TABLE_CELL_PAINT_SCHEMA_V1.to_owned(),
            cell_id: id(),
            table_class: TableCellClassV1::SimpleUnmergedRectangular,
            fill: None,
            borders: TableCellBordersV1 {
                top: Some(stroke(1)),
                right: Some(stroke(2)),
                bottom: Some(stroke(3)),
                left: Some(stroke(4)),
            },
        };
        assert_eq!(validate_table_cell_paint_v1(&paint), Ok(()));
        assert_eq!(
            [
                paint.borders.top.as_ref().unwrap().width_emu,
                paint.borders.right.as_ref().unwrap().width_emu,
                paint.borders.bottom.as_ref().unwrap().width_emu,
                paint.borders.left.as_ref().unwrap().width_emu,
            ],
            [1, 2, 3, 4]
        );

        paint.borders.left = Some(stroke(0));
        assert_eq!(
            validate_table_cell_paint_v1(&paint),
            Err(TableCellPaintValidationError::NonPositiveBorderWidth {
                side: TableCellBorderSideV1::Left,
                width_emu: 0,
            })
        );
    }

    #[test]
    fn canonical_hash_roundtrips_and_is_value_sensitive() {
        let paint = TableCellPaintV1 {
            schema_version: TABLE_CELL_PAINT_SCHEMA_V1.to_owned(),
            cell_id: id(),
            table_class: TableCellClassV1::SimpleUnmergedRectangular,
            fill: Some(SolidFillV1 {
                visible: false,
                color: Srgb8 { r: 7, g: 8, b: 9 },
            }),
            borders: TableCellBordersV1::default(),
        };
        let bytes = serde_json::to_vec(&paint).expect("serialize");
        let reopened: TableCellPaintV1 = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(paint, reopened);
        assert_eq!(
            canonical_table_cell_paint_hash_v1(&paint).expect("hash"),
            canonical_table_cell_paint_hash_v1(&reopened).expect("hash")
        );

        let mut changed = paint.clone();
        changed.fill.as_mut().unwrap().visible = true;
        assert_ne!(
            canonical_table_cell_paint_hash_v1(&paint).expect("hash"),
            canonical_table_cell_paint_hash_v1(&changed).expect("hash")
        );
    }
}
