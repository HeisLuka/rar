use crate::{BoundedLayoutProjection, ProjectionSeverity, ResolveBlocked};
use pub_model::{Affine2D, LengthEmu, NodeId, RectEmu, TableCellAddress, TableCellId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedUniformTableMetrics {
    pub table_origin: NodeId,
    pub cell_width: LengthEmu,
    pub row_pitch: LengthEmu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTableCell {
    pub origin: TableCellId,
    pub table_origin: NodeId,
    pub address: TableCellAddress,
    pub bounds: RectEmu,
    pub transform: Affine2D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCellOriginMapping {
    pub cell_origin: TableCellId,
    pub table_origin: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedResolvedTableCells {
    pub cells: Vec<ResolvedTableCell>,
    pub origin_mapping: Vec<TableCellOriginMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedTableResolveError {
    ProjectionBlocked(ResolveBlocked),
    DuplicateMetrics {
        table_origin: NodeId,
    },
    MetricsForUnknownTable {
        table_origin: NodeId,
    },
    MissingMetrics {
        table_origin: NodeId,
    },
    MissingTableGeometry {
        table_origin: NodeId,
    },
    NonPositiveCellWidth {
        table_origin: NodeId,
        value: i64,
    },
    NonPositiveRowPitch {
        table_origin: NodeId,
        value: i64,
    },
    MetricOverflow {
        table_origin: NodeId,
    },
    MetricsExceedTableBounds {
        table_origin: NodeId,
        required_width: i64,
        required_height: i64,
        available_width: i64,
        available_height: i64,
    },
}

impl fmt::Display for BoundedTableResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectionBlocked(error) => write!(
                f,
                "projection blocked table resolution with {} errors",
                error.projection_errors.len()
            ),
            Self::DuplicateMetrics { table_origin } => {
                write!(
                    f,
                    "duplicate metrics for table {}",
                    table_origin.as_canonical()
                )
            }
            Self::MetricsForUnknownTable { table_origin } => {
                write!(
                    f,
                    "metrics supplied for unknown table {}",
                    table_origin.as_canonical()
                )
            }
            Self::MissingMetrics { table_origin } => {
                write!(
                    f,
                    "missing explicit metrics for table {}",
                    table_origin.as_canonical()
                )
            }
            Self::MissingTableGeometry { table_origin } => write!(
                f,
                "table {} has no authored node geometry",
                table_origin.as_canonical()
            ),
            Self::NonPositiveCellWidth {
                table_origin,
                value,
            } => write!(
                f,
                "table {} has non-positive cell width {} EMU",
                table_origin.as_canonical(),
                value
            ),
            Self::NonPositiveRowPitch {
                table_origin,
                value,
            } => write!(
                f,
                "table {} has non-positive row pitch {} EMU",
                table_origin.as_canonical(),
                value
            ),
            Self::MetricOverflow { table_origin } => write!(
                f,
                "table {} cell metric arithmetic overflowed",
                table_origin.as_canonical()
            ),
            Self::MetricsExceedTableBounds {
                table_origin,
                required_width,
                required_height,
                available_width,
                available_height,
            } => write!(
                f,
                "table {} metrics require {}x{} EMU but owner bounds provide {}x{} EMU",
                table_origin.as_canonical(),
                required_width,
                required_height,
                available_width,
                available_height
            ),
        }
    }
}

impl std::error::Error for BoundedTableResolveError {}

/// Resolves physical rectangles for the deliberately narrow uniform simple-table subset.
///
/// Row/column counts never imply equal partitioning. Cell size must be supplied
/// explicitly by the caller from a grounded layout/environment source. The
/// resolver only combines those metrics with semantic cell addresses and the
/// authored TABLE node geometry already present in the layout projection.
pub fn resolve_bounded_uniform_table_cells(
    projection: &BoundedLayoutProjection,
    metrics: &[BoundedUniformTableMetrics],
) -> Result<BoundedResolvedTableCells, BoundedTableResolveError> {
    let projection_errors: Vec<_> = projection
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == ProjectionSeverity::Error)
        .cloned()
        .collect();
    if !projection_errors.is_empty() {
        return Err(BoundedTableResolveError::ProjectionBlocked(
            ResolveBlocked { projection_errors },
        ));
    }

    let table_ids: BTreeSet<_> = projection.tables.iter().map(|table| table.origin).collect();
    let mut metric_map = BTreeMap::new();
    for metric in metrics {
        if !table_ids.contains(&metric.table_origin) {
            return Err(BoundedTableResolveError::MetricsForUnknownTable {
                table_origin: metric.table_origin,
            });
        }
        if metric_map.insert(metric.table_origin, *metric).is_some() {
            return Err(BoundedTableResolveError::DuplicateMetrics {
                table_origin: metric.table_origin,
            });
        }
    }

    let geometry: BTreeMap<_, _> = projection
        .node_geometry
        .iter()
        .map(|node| (node.origin, node))
        .collect();

    let mut cells = Vec::new();
    let mut origin_mapping = Vec::new();

    for table in &projection.tables {
        let metric =
            metric_map
                .get(&table.origin)
                .ok_or(BoundedTableResolveError::MissingMetrics {
                    table_origin: table.origin,
                })?;
        if metric.cell_width.get() <= 0 {
            return Err(BoundedTableResolveError::NonPositiveCellWidth {
                table_origin: table.origin,
                value: metric.cell_width.get(),
            });
        }
        if metric.row_pitch.get() <= 0 {
            return Err(BoundedTableResolveError::NonPositiveRowPitch {
                table_origin: table.origin,
                value: metric.row_pitch.get(),
            });
        }

        let owner =
            geometry
                .get(&table.origin)
                .ok_or(BoundedTableResolveError::MissingTableGeometry {
                    table_origin: table.origin,
                })?;

        let required_width = checked_mul_metric(metric.cell_width, table.columns, table.origin)?;
        let required_height = checked_mul_metric(metric.row_pitch, table.rows, table.origin)?;
        if required_width.get() > owner.bounds.width.get()
            || required_height.get() > owner.bounds.height.get()
        {
            return Err(BoundedTableResolveError::MetricsExceedTableBounds {
                table_origin: table.origin,
                required_width: required_width.get(),
                required_height: required_height.get(),
                available_width: owner.bounds.width.get(),
                available_height: owner.bounds.height.get(),
            });
        }

        for cell in &table.cells {
            let x_offset =
                checked_mul_metric(metric.cell_width, cell.address.column, table.origin)?;
            let y_offset = checked_mul_metric(metric.row_pitch, cell.address.row, table.origin)?;
            let x = owner.bounds.x.checked_add(x_offset).ok_or(
                BoundedTableResolveError::MetricOverflow {
                    table_origin: table.origin,
                },
            )?;
            let y = owner.bounds.y.checked_add(y_offset).ok_or(
                BoundedTableResolveError::MetricOverflow {
                    table_origin: table.origin,
                },
            )?;

            cells.push(ResolvedTableCell {
                origin: cell.origin,
                table_origin: table.origin,
                address: cell.address,
                bounds: RectEmu::new(x, y, metric.cell_width, metric.row_pitch),
                transform: owner.transform.clone(),
            });
            origin_mapping.push(TableCellOriginMapping {
                cell_origin: cell.origin,
                table_origin: table.origin,
            });
        }
    }

    cells.sort_by_key(|cell| {
        (
            cell.table_origin,
            cell.address.row,
            cell.address.column,
            cell.origin,
        )
    });
    origin_mapping.sort_by_key(|mapping| (mapping.table_origin, mapping.cell_origin));

    Ok(BoundedResolvedTableCells {
        cells,
        origin_mapping,
    })
}

fn checked_mul_metric(
    metric: LengthEmu,
    count: u32,
    table_origin: NodeId,
) -> Result<LengthEmu, BoundedTableResolveError> {
    let value = metric
        .get()
        .checked_mul(i64::from(count))
        .ok_or(BoundedTableResolveError::MetricOverflow { table_origin })?;
    Ok(LengthEmu::new(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BoundedAuthoringSlice, BoundedNodeGeometryInput, BoundedTableInput, project_bounded,
    };
    use pub_model::{
        CanonicalId, Page, PageId, SimpleRectangularTable, SimpleTableCell, Size2D,
        TableCellAddress,
    };

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn page_id(byte: u8) -> PageId {
        PageId::from_canonical(id(byte))
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_canonical(id(byte))
    }

    fn cell_id(byte: u8) -> TableCellId {
        TableCellId::from_canonical(id(byte))
    }

    fn projection() -> BoundedLayoutProjection {
        let table = SimpleRectangularTable::new(
            3,
            2,
            vec![
                SimpleTableCell {
                    id: cell_id(10),
                    address: TableCellAddress { row: 0, column: 0 },
                },
                SimpleTableCell {
                    id: cell_id(11),
                    address: TableCellAddress { row: 0, column: 1 },
                },
                SimpleTableCell {
                    id: cell_id(12),
                    address: TableCellAddress { row: 1, column: 0 },
                },
                SimpleTableCell {
                    id: cell_id(13),
                    address: TableCellAddress { row: 1, column: 1 },
                },
                SimpleTableCell {
                    id: cell_id(14),
                    address: TableCellAddress { row: 2, column: 0 },
                },
                SimpleTableCell {
                    id: cell_id(15),
                    address: TableCellAddress { row: 2, column: 1 },
                },
            ],
        )
        .unwrap();

        project_bounded(BoundedAuthoringSlice {
            pages: vec![Page {
                id: page_id(1),
                size: Size2D::new(LengthEmu::new(10_000_000), LengthEmu::new(10_000_000)),
                bleed: None,
                margins: None,
                children: vec![node_id(9)],
                extensions: Vec::new(),
            }],
            node_geometry: vec![BoundedNodeGeometryInput {
                node_id: node_id(9),
                parent_origin: page_id(1).into_canonical(),
                bounds: RectEmu::new(
                    LengthEmu::new(100_000),
                    LengthEmu::new(200_000),
                    LengthEmu::new(3_564_000),
                    LengthEmu::new(657_564),
                ),
                transform: Affine2D::identity(),
            }],
            stories: Vec::new(),
            story_frames: Vec::new(),
            tables: vec![BoundedTableInput {
                node_id: node_id(9),
                table,
            }],
            guides: Vec::new(),
            unknown_layout_state: Vec::new(),
        })
    }

    fn sample_metrics() -> Vec<BoundedUniformTableMetrics> {
        vec![BoundedUniformTableMetrics {
            table_origin: node_id(9),
            cell_width: LengthEmu::new(1_782_000),
            row_pitch: LengthEmu::new(219_188),
        }]
    }

    #[test]
    fn grounded_sample_metrics_resolve_exact_3x2_rectangles() {
        let resolved =
            resolve_bounded_uniform_table_cells(&projection(), &sample_metrics()).unwrap();

        assert_eq!(resolved.cells.len(), 6);
        assert_eq!(resolved.cells[0].origin, cell_id(10));
        assert_eq!(
            resolved.cells[0].bounds,
            RectEmu::new(
                LengthEmu::new(100_000),
                LengthEmu::new(200_000),
                LengthEmu::new(1_782_000),
                LengthEmu::new(219_188),
            )
        );
        assert_eq!(
            resolved.cells[5].bounds,
            RectEmu::new(
                LengthEmu::new(1_882_000),
                LengthEmu::new(638_376),
                LengthEmu::new(1_782_000),
                LengthEmu::new(219_188),
            )
        );
    }

    #[test]
    fn table_cell_identity_is_preserved_without_fake_node_ids() {
        let resolved =
            resolve_bounded_uniform_table_cells(&projection(), &sample_metrics()).unwrap();

        assert_eq!(
            resolved.origin_mapping[0],
            TableCellOriginMapping {
                cell_origin: cell_id(10),
                table_origin: node_id(9),
            }
        );
    }

    #[test]
    fn missing_metrics_fail_closed_instead_of_equal_split_inference() {
        assert_eq!(
            resolve_bounded_uniform_table_cells(&projection(), &[]),
            Err(BoundedTableResolveError::MissingMetrics {
                table_origin: node_id(9),
            })
        );
    }

    #[test]
    fn metrics_may_not_exceed_authored_table_bounds() {
        let metrics = vec![BoundedUniformTableMetrics {
            table_origin: node_id(9),
            cell_width: LengthEmu::new(1_782_001),
            row_pitch: LengthEmu::new(219_188),
        }];

        assert!(matches!(
            resolve_bounded_uniform_table_cells(&projection(), &metrics),
            Err(BoundedTableResolveError::MetricsExceedTableBounds { .. })
        ));
    }

    #[test]
    fn fixed_projection_and_metrics_are_deterministic() {
        let projection = projection();
        let metrics = sample_metrics();

        let left = resolve_bounded_uniform_table_cells(&projection, &metrics).unwrap();
        let right = resolve_bounded_uniform_table_cells(&projection, &metrics).unwrap();

        assert_eq!(left, right);
    }
}
