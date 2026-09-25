use pub_model::{CanonicalId, EMU_PER_POINT, LengthEmu, StoryId, TableCellId};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdmlTableCell {
    pub id: TableCellId,
    pub row: u32,
    pub column: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdmlSimpleTable {
    pub story_id: StoryId,
    pub rows: u32,
    pub columns: u32,
    pub column_width: LengthEmu,
    pub row_height: LengthEmu,
    pub cells: Vec<IdmlTableCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdmlTableError {
    ZeroRows,
    ZeroColumns,
    NonPositiveColumnWidth,
    NonPositiveRowHeight,
    CellCountOverflow,
    WrongCellCount {
        expected: u64,
        actual: usize,
    },
    CellOutOfBounds {
        id: TableCellId,
        row: u32,
        column: u32,
    },
    DuplicateCellId {
        id: TableCellId,
    },
    DuplicateAddress {
        row: u32,
        column: u32,
    },
    InvalidXmlCharacter {
        id: TableCellId,
        scalar: u32,
    },
}

impl fmt::Display for IdmlTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRows => formatter.write_str("IDML simple table requires at least one row"),
            Self::ZeroColumns => {
                formatter.write_str("IDML simple table requires at least one column")
            }
            Self::NonPositiveColumnWidth => {
                formatter.write_str("IDML simple table column width must be positive")
            }
            Self::NonPositiveRowHeight => {
                formatter.write_str("IDML simple table row height must be positive")
            }
            Self::CellCountOverflow => formatter.write_str("IDML simple table cell count overflow"),
            Self::WrongCellCount { expected, actual } => write!(
                formatter,
                "IDML simple table expected {expected} cells, got {actual}"
            ),
            Self::CellOutOfBounds { id, row, column } => write!(
                formatter,
                "IDML table cell {} address {column}:{row} is outside the grid",
                id.as_canonical()
            ),
            Self::DuplicateCellId { id } => write!(
                formatter,
                "duplicate IDML table cell identity {}",
                id.as_canonical()
            ),
            Self::DuplicateAddress { row, column } => {
                write!(
                    formatter,
                    "duplicate IDML table cell address {column}:{row}"
                )
            }
            Self::InvalidXmlCharacter { id, scalar } => write!(
                formatter,
                "IDML table cell {} contains invalid XML 1.0 scalar U+{scalar:04X}",
                id.as_canonical()
            ),
        }
    }
}

impl std::error::Error for IdmlTableError {}

impl IdmlSimpleTable {
    pub fn validate(&self) -> Result<(), IdmlTableError> {
        if self.rows == 0 {
            return Err(IdmlTableError::ZeroRows);
        }
        if self.columns == 0 {
            return Err(IdmlTableError::ZeroColumns);
        }
        if self.column_width.get() <= 0 {
            return Err(IdmlTableError::NonPositiveColumnWidth);
        }
        if self.row_height.get() <= 0 {
            return Err(IdmlTableError::NonPositiveRowHeight);
        }

        let expected = u64::from(self.rows)
            .checked_mul(u64::from(self.columns))
            .ok_or(IdmlTableError::CellCountOverflow)?;
        if expected != self.cells.len() as u64 {
            return Err(IdmlTableError::WrongCellCount {
                expected,
                actual: self.cells.len(),
            });
        }

        let mut ids = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        for cell in &self.cells {
            if cell.row >= self.rows || cell.column >= self.columns {
                return Err(IdmlTableError::CellOutOfBounds {
                    id: cell.id,
                    row: cell.row,
                    column: cell.column,
                });
            }
            if !ids.insert(cell.id) {
                return Err(IdmlTableError::DuplicateCellId { id: cell.id });
            }
            if !addresses.insert((cell.row, cell.column)) {
                return Err(IdmlTableError::DuplicateAddress {
                    row: cell.row,
                    column: cell.column,
                });
            }
            validate_xml_text(cell)?;
        }

        Ok(())
    }

    pub(crate) fn write_story_body(
        &self,
        table_owner: CanonicalId,
    ) -> Result<String, IdmlTableError> {
        self.validate()?;

        let table_self = idml_self("ut", table_owner);
        let row_height = format_emu_points(self.row_height);
        let column_width = format_emu_points(self.column_width);

        let mut cells = self.cells.iter().collect::<Vec<_>>();
        cells.sort_by_key(|cell| (cell.row, cell.column, cell.id));

        let mut xml = String::new();
        xml.push_str(
            "    <ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\">\n",
        );
        xml.push_str(
            "      <CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\n",
        );
        writeln!(
            xml,
            "        <Table Self=\"{table_self}\" HeaderRowCount=\"0\" FooterRowCount=\"0\" BodyRowCount=\"{}\" ColumnCount=\"{}\" AppliedTableStyle=\"TableStyle/$ID/[Basic Table]\" TableDirection=\"LeftToRightDirection\">",
            self.rows, self.columns
        )
        .unwrap();

        for row in 0..self.rows {
            writeln!(
                xml,
                "          <Row Self=\"{table_self}Row{row}\" Name=\"{row}\" SingleRowHeight=\"{row_height}\"/>"
            )
            .unwrap();
        }
        for column in 0..self.columns {
            writeln!(
                xml,
                "          <Column Self=\"{table_self}Column{column}\" Name=\"{column}\" SingleColumnWidth=\"{column_width}\"/>"
            )
            .unwrap();
        }

        for cell in cells {
            let cell_self = idml_self("utc", cell.id.into_canonical());
            writeln!(
                xml,
                "          <Cell Self=\"{cell_self}\" Name=\"{}:{}\" RowSpan=\"1\" ColumnSpan=\"1\">",
                cell.column, cell.row
            )
            .unwrap();
            xml.push_str(
                "            <ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\">\n",
            );
            xml.push_str(
                "              <CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\n",
            );
            let escaped = escape_xml_text(cell)?;
            writeln!(xml, "                <Content>{escaped}</Content>").unwrap();
            xml.push_str("              </CharacterStyleRange>\n");
            xml.push_str("            </ParagraphStyleRange>\n");
            xml.push_str("          </Cell>\n");
        }

        xml.push_str("        </Table>\n");
        xml.push_str("      </CharacterStyleRange>\n");
        xml.push_str("    </ParagraphStyleRange>\n");
        Ok(xml)
    }
}

fn validate_xml_text(cell: &IdmlTableCell) -> Result<(), IdmlTableError> {
    for character in cell.text.chars() {
        let scalar = u32::from(character);
        if !is_xml_10_scalar(scalar) {
            return Err(IdmlTableError::InvalidXmlCharacter {
                id: cell.id,
                scalar,
            });
        }
    }
    Ok(())
}

fn escape_xml_text(cell: &IdmlTableCell) -> Result<String, IdmlTableError> {
    validate_xml_text(cell)?;
    let mut escaped = String::with_capacity(cell.text.len());
    for character in cell.text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    Ok(escaped)
}

fn is_xml_10_scalar(value: u32) -> bool {
    matches!(
        value,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
}

fn idml_self(prefix: &str, id: CanonicalId) -> String {
    let mut value = String::with_capacity(prefix.len() + 32);
    value.push_str(prefix);
    for byte in id.into_bytes() {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

fn format_emu_points(value: LengthEmu) -> String {
    let numerator = i128::from(value.get());
    let denominator = i128::from(EMU_PER_POINT);
    let whole = numerator / denominator;
    let mut remainder = numerator % denominator;
    if remainder == 0 {
        return whole.to_string();
    }

    let mut result = format!("{whole}.");
    for _ in 0..15 {
        remainder *= 10;
        let digit = remainder / denominator;
        result.push(char::from(
            b'0' + u8::try_from(digit).expect("decimal digit"),
        ));
        remainder %= denominator;
        if remainder == 0 {
            break;
        }
    }
    while result.ends_with('0') {
        result.pop();
    }
    if result.ends_with('.') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::TableCellId;

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn cell(byte: u8, row: u32, column: u32, text: &str) -> IdmlTableCell {
        IdmlTableCell {
            id: TableCellId::from_canonical(id(byte)),
            row,
            column,
            text: text.into(),
        }
    }

    #[test]
    fn structured_table_orders_cells_by_visual_address_not_input_order() {
        let table = IdmlSimpleTable {
            story_id: StoryId::from_canonical(id(9)),
            rows: 2,
            columns: 2,
            column_width: LengthEmu::new(1_782_000),
            row_height: LengthEmu::new(219_188),
            cells: vec![
                cell(3, 1, 0, "bottom left"),
                cell(1, 0, 0, "top left"),
                cell(4, 1, 1, "bottom right"),
                cell(2, 0, 1, "top right"),
            ],
        };

        let xml = table
            .write_story_body(id(7))
            .expect("simple table should serialize");
        let p00 = xml.find("Name=\"0:0\"").unwrap();
        let p10 = xml.find("Name=\"1:0\"").unwrap();
        let p01 = xml.find("Name=\"0:1\"").unwrap();
        let p11 = xml.find("Name=\"1:1\"").unwrap();
        assert!(p00 < p10 && p10 < p01 && p01 < p11);
        assert!(xml.contains("BodyRowCount=\"2\" ColumnCount=\"2\""));
        assert!(xml.contains("SingleColumnWidth=\"140.314960629921259\""));
        assert!(xml.contains("SingleRowHeight=\"17.258897637795275\""));
    }

    #[test]
    fn incomplete_grid_is_rejected_instead_of_flattened() {
        let table = IdmlSimpleTable {
            story_id: StoryId::from_canonical(id(9)),
            rows: 1,
            columns: 2,
            column_width: LengthEmu::new(100),
            row_height: LengthEmu::new(100),
            cells: vec![cell(1, 0, 0, "one")],
        };

        assert!(matches!(
            table.validate(),
            Err(IdmlTableError::WrongCellCount {
                expected: 2,
                actual: 1
            })
        ));
    }
}
