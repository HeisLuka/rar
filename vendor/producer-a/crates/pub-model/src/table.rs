use crate::{LengthEmu, NodeId, StoryId, TableCellId, TableColumnId, TableRowId};
use serde::{Deserialize, Serialize};

pub const EFFECTIVE_TABLE_GRID_V1: &str = "chaptera.effective-table-grid.v1";

/// Координата ячейки в простом прямоугольном table subset.
///
/// Это не wire-coordinate и не display rectangle. Row/column — семантическая
/// позиция внутри таблицы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TableCellAddress {
    pub row: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleTableCell<CellId> {
    pub id: CellId,
    pub address: TableCellAddress,
}

/// Ограниченная модель простой прямоугольной таблицы без merged cells.
///
/// Тип введён для grounded simple-table fixtures и намеренно не утверждает,
/// что все Publisher tables обязаны иметь такую топологию.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleRectangularTable<CellId> {
    pub rows: u32,
    pub columns: u32,
    pub cells: Vec<SimpleTableCell<CellId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimpleTableError<CellId> {
    ZeroRows,
    ZeroColumns,
    CellCountOverflow,
    WrongCellCount {
        expected: u64,
        actual: usize,
    },
    CellOutOfBounds {
        id: CellId,
        address: TableCellAddress,
    },
    DuplicateCellId {
        id: CellId,
    },
    DuplicateAddress {
        address: TableCellAddress,
    },
}

impl<CellId> SimpleRectangularTable<CellId>
where
    CellId: Clone + Eq,
{
    pub fn new(
        rows: u32,
        columns: u32,
        cells: Vec<SimpleTableCell<CellId>>,
    ) -> Result<Self, SimpleTableError<CellId>> {
        if rows == 0 {
            return Err(SimpleTableError::ZeroRows);
        }
        if columns == 0 {
            return Err(SimpleTableError::ZeroColumns);
        }

        let expected = u64::from(rows)
            .checked_mul(u64::from(columns))
            .ok_or(SimpleTableError::CellCountOverflow)?;
        if expected != cells.len() as u64 {
            return Err(SimpleTableError::WrongCellCount {
                expected,
                actual: cells.len(),
            });
        }

        for (index, cell) in cells.iter().enumerate() {
            if cell.address.row >= rows || cell.address.column >= columns {
                return Err(SimpleTableError::CellOutOfBounds {
                    id: cell.id.clone(),
                    address: cell.address,
                });
            }

            if cells[..index].iter().any(|other| other.id == cell.id) {
                return Err(SimpleTableError::DuplicateCellId {
                    id: cell.id.clone(),
                });
            }

            if cells[..index]
                .iter()
                .any(|other| other.address == cell.address)
            {
                return Err(SimpleTableError::DuplicateAddress {
                    address: cell.address,
                });
            }
        }

        Ok(Self {
            rows,
            columns,
            cells,
        })
    }

    /// Проверяет только текущий порядок массива cells.
    ///
    /// Row-major order подтверждён для конкретного Sample_2010 fixture, но не
    /// объявляется универсальным invariant всех Publisher tables.
    pub fn cells_are_row_major(&self) -> bool {
        self.cells.iter().enumerate().all(|(index, cell)| {
            let index = index as u64;
            let columns = u64::from(self.columns);
            cell.address.row as u64 == index / columns
                && cell.address.column as u64 == index % columns
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(id: u8, row: u32, column: u32) -> SimpleTableCell<u8> {
        SimpleTableCell {
            id,
            address: TableCellAddress { row, column },
        }
    }

    #[test]
    fn sample_3x2_table_can_be_represented_as_grounded_simple_subset() {
        let table = SimpleRectangularTable::new(
            3,
            2,
            vec![
                cell(0, 0, 0),
                cell(1, 0, 1),
                cell(2, 1, 0),
                cell(3, 1, 1),
                cell(4, 2, 0),
                cell(5, 2, 1),
            ],
        )
        .expect("полная таблица 3x2 должна быть допустима");

        assert!(table.cells_are_row_major());
    }

    #[test]
    fn row_major_is_observation_not_constructor_requirement() {
        let table = SimpleRectangularTable::new(
            2,
            2,
            vec![cell(2, 1, 0), cell(0, 0, 0), cell(3, 1, 1), cell(1, 0, 1)],
        )
        .expect("семантическая сетка не обязана храниться в row-major порядке");

        assert!(!table.cells_are_row_major());
    }

    #[test]
    fn duplicate_cell_address_is_rejected() {
        let error = SimpleRectangularTable::new(1, 2, vec![cell(0, 0, 0), cell(1, 0, 0)])
            .expect_err("один semantic address не должен принадлежать двум cells");

        assert_eq!(
            error,
            SimpleTableError::DuplicateAddress {
                address: TableCellAddress { row: 0, column: 0 },
            }
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveTableTrackV1<Id> {
    pub id: Id,
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extent: Option<LengthEmu>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveTableCellV1 {
    pub id: TableCellId,
    pub row_id: TableRowId,
    pub column_id: TableColumnId,
    pub address: TableCellAddress,
    pub row_span: u32,
    pub column_span: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_id: Option<StoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utf16_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utf16_end: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveTableGridV1 {
    pub version: String,
    pub table_id: NodeId,
    pub rows: Vec<EffectiveTableTrackV1<TableRowId>>,
    pub columns: Vec<EffectiveTableTrackV1<TableColumnId>>,
    pub cells: Vec<EffectiveTableCellV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveTableGridError {
    WrongVersion,
    EmptyRows,
    EmptyColumns,
    NonContiguousRowIndex,
    NonContiguousColumnIndex,
    DuplicateRowId,
    DuplicateColumnId,
    WrongCellCount {
        expected: u64,
        actual: usize,
    },
    CellOutOfBounds {
        id: TableCellId,
        address: TableCellAddress,
    },
    DuplicateCellId {
        id: TableCellId,
    },
    DuplicateAddress {
        address: TableCellAddress,
    },
    WrongTrackReference {
        id: TableCellId,
    },
    UnsupportedSpan {
        id: TableCellId,
        row_span: u32,
        column_span: u32,
    },
    IncompleteStoryRange {
        id: TableCellId,
    },
    InvalidStoryRange {
        id: TableCellId,
        start: u32,
        end: u32,
    },
    NonPositiveRowExtent {
        id: TableRowId,
        value: i64,
    },
    NonPositiveColumnExtent {
        id: TableColumnId,
        value: i64,
    },
}

impl EffectiveTableGridV1 {
    pub fn validate(&self) -> Result<(), EffectiveTableGridError> {
        if self.version != EFFECTIVE_TABLE_GRID_V1 {
            return Err(EffectiveTableGridError::WrongVersion);
        }
        if self.rows.is_empty() {
            return Err(EffectiveTableGridError::EmptyRows);
        }
        if self.columns.is_empty() {
            return Err(EffectiveTableGridError::EmptyColumns);
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.index != u32::try_from(index).expect("bounded Vec index") {
                return Err(EffectiveTableGridError::NonContiguousRowIndex);
            }
            if self.rows[..index].iter().any(|other| other.id == row.id) {
                return Err(EffectiveTableGridError::DuplicateRowId);
            }
            if let Some(extent) = row.extent {
                if extent.get() <= 0 {
                    return Err(EffectiveTableGridError::NonPositiveRowExtent {
                        id: row.id,
                        value: extent.get(),
                    });
                }
            }
        }
        for (index, column) in self.columns.iter().enumerate() {
            if column.index != u32::try_from(index).expect("bounded Vec index") {
                return Err(EffectiveTableGridError::NonContiguousColumnIndex);
            }
            if self.columns[..index]
                .iter()
                .any(|other| other.id == column.id)
            {
                return Err(EffectiveTableGridError::DuplicateColumnId);
            }
            if let Some(extent) = column.extent {
                if extent.get() <= 0 {
                    return Err(EffectiveTableGridError::NonPositiveColumnExtent {
                        id: column.id,
                        value: extent.get(),
                    });
                }
            }
        }

        let expected = u64::try_from(self.rows.len()).expect("row count fits u64")
            * u64::try_from(self.columns.len()).expect("column count fits u64");
        if expected != self.cells.len() as u64 {
            return Err(EffectiveTableGridError::WrongCellCount {
                expected,
                actual: self.cells.len(),
            });
        }

        for (index, cell) in self.cells.iter().enumerate() {
            let row = usize::try_from(cell.address.row)
                .ok()
                .and_then(|i| self.rows.get(i));
            let column = usize::try_from(cell.address.column)
                .ok()
                .and_then(|i| self.columns.get(i));
            let (Some(row), Some(column)) = (row, column) else {
                return Err(EffectiveTableGridError::CellOutOfBounds {
                    id: cell.id,
                    address: cell.address,
                });
            };
            if row.id != cell.row_id || column.id != cell.column_id {
                return Err(EffectiveTableGridError::WrongTrackReference { id: cell.id });
            }
            if cell.row_span != 1 || cell.column_span != 1 {
                return Err(EffectiveTableGridError::UnsupportedSpan {
                    id: cell.id,
                    row_span: cell.row_span,
                    column_span: cell.column_span,
                });
            }
            if self.cells[..index].iter().any(|other| other.id == cell.id) {
                return Err(EffectiveTableGridError::DuplicateCellId { id: cell.id });
            }
            if self.cells[..index]
                .iter()
                .any(|other| other.address == cell.address)
            {
                return Err(EffectiveTableGridError::DuplicateAddress {
                    address: cell.address,
                });
            }
            if cell.story_id.is_none() && (cell.utf16_start.is_some() || cell.utf16_end.is_some()) {
                return Err(EffectiveTableGridError::StoryRangeWithoutStory { id: cell.id });
            }
            match (cell.utf16_start, cell.utf16_end) {
                (None, None) => {}
                (Some(start), Some(end)) if start <= end => {}
                (Some(start), Some(end)) => {
                    return Err(EffectiveTableGridError::InvalidStoryRange {
                        id: cell.id,
                        start,
                        end,
                    });
                }
                _ => return Err(EffectiveTableGridError::IncompleteStoryRange { id: cell.id }),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod effective_grid_tests {
    use super::*;
    use crate::{CanonicalId, StoryId};

    fn row_id(byte: u8) -> TableRowId {
        TableRowId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    fn column_id(byte: u8) -> TableColumnId {
        TableColumnId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    fn cell_id(byte: u8) -> TableCellId {
        TableCellId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    fn story_id(byte: u8) -> StoryId {
        StoryId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    fn grid(
        row_extent: Option<LengthEmu>,
        column_extent: Option<LengthEmu>,
    ) -> EffectiveTableGridV1 {
        let rows = vec![
            EffectiveTableTrackV1 {
                id: row_id(1),
                index: 0,
                extent: row_extent,
            },
            EffectiveTableTrackV1 {
                id: row_id(2),
                index: 1,
                extent: row_extent,
            },
        ];
        let columns = vec![
            EffectiveTableTrackV1 {
                id: column_id(3),
                index: 0,
                extent: column_extent,
            },
            EffectiveTableTrackV1 {
                id: column_id(4),
                index: 1,
                extent: column_extent,
            },
        ];
        let cells = vec![
            EffectiveTableCellV1 {
                id: cell_id(10),
                row_id: rows[0].id,
                column_id: columns[0].id,
                address: TableCellAddress { row: 0, column: 0 },
                row_span: 1,
                column_span: 1,
                story_id: Some(story_id(9)),
                utf16_start: Some(0),
                utf16_end: Some(2),
            },
            EffectiveTableCellV1 {
                id: cell_id(11),
                row_id: rows[0].id,
                column_id: columns[1].id,
                address: TableCellAddress { row: 0, column: 1 },
                row_span: 1,
                column_span: 1,
                story_id: Some(story_id(9)),
                utf16_start: Some(2),
                utf16_end: Some(4),
            },
            EffectiveTableCellV1 {
                id: cell_id(12),
                row_id: rows[1].id,
                column_id: columns[0].id,
                address: TableCellAddress { row: 1, column: 0 },
                row_span: 1,
                column_span: 1,
                story_id: Some(story_id(9)),
                utf16_start: Some(4),
                utf16_end: Some(6),
            },
            EffectiveTableCellV1 {
                id: cell_id(13),
                row_id: rows[1].id,
                column_id: columns[1].id,
                address: TableCellAddress { row: 1, column: 1 },
                row_span: 1,
                column_span: 1,
                story_id: Some(story_id(9)),
                utf16_start: Some(6),
                utf16_end: Some(8),
            },
        ];
        EffectiveTableGridV1 {
            version: EFFECTIVE_TABLE_GRID_V1.into(),
            table_id: node_id(7),
            rows,
            columns,
            cells,
        }
    }

    #[test]
    fn valid_grid_preserves_stable_track_and_cell_identity() {
        let grid = grid(Some(LengthEmu::new(200)), Some(LengthEmu::new(300)));
        assert_eq!(grid.validate(), Ok(()));
        assert_eq!(grid.rows[0].id, row_id(1));
        assert_eq!(grid.columns[1].id, column_id(4));
        assert_eq!(grid.cells[2].id, cell_id(12));
    }

    #[test]
    fn unknown_metrics_are_valid_state_not_inferred_values() {
        let grid = grid(None, None);
        assert_eq!(grid.validate(), Ok(()));
        assert!(grid.rows.iter().all(|row| row.extent.is_none()));
        assert!(grid.columns.iter().all(|column| column.extent.is_none()));
    }

    #[test]
    fn shifted_coordinate_must_keep_matching_track_identity() {
        let mut grid = grid(None, None);
        grid.cells[0].address.row = 1;
        assert!(matches!(
            grid.validate(),
            Err(EffectiveTableGridError::WrongTrackReference { id }) if id == cell_id(10)
        ));
    }

    #[test]
    fn content_range_requires_story_identity() {
        let mut grid = grid(None, None);
        grid.cells[0].story_id = None;
        assert!(matches!(
            grid.validate(),
            Err(EffectiveTableGridError::StoryRangeWithoutStory { id }) if id == cell_id(10)
        ));
    }

    #[test]
    fn v1_rejects_merged_cells() {
        let mut grid = grid(None, None);
        grid.cells[0].column_span = 2;
        assert!(matches!(
            grid.validate(),
            Err(EffectiveTableGridError::UnsupportedSpan {
                id,
                row_span: 1,
                column_span: 2,
            }) if id == cell_id(10)
        ));
    }
}
