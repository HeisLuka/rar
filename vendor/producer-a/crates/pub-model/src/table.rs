use serde::{Deserialize, Serialize};

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
