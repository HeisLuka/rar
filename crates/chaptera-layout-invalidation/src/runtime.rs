use crate::{
    ArtifactDependencyV1, ArtifactKeyV1, ArtifactReceiptV1, DependencyEdgeV1, DependencyKindV1,
    DependencyNodeV1, ExpectedUpstreamV1, FingerprintV1, fingerprint_v1,
};

pub const PREPARED_METRICS_STAGE_V1: &str = "prepared-metrics-v1";
pub const LINE_REGION_STAGE_V1: &str = "line-region-v1";
pub const STORY_FLOW_STAGE_V1: &str = "story-flow-v1";
pub const SCENE_SHARD_STAGE_V1: &str = "scene-shard-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedScalarMetricV1 {
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub advance_emu: i64,
    pub break_after: bool,
    pub semantic_fingerprint: FingerprintV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMetricsUnitV1 {
    pub story_id: String,
    pub unit_id: String,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub dependency_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub scalars: Vec<ResolvedScalarMetricV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareErrorV1 {
    EmptyUnit,
    NonContiguousScalarRange,
    NonPositiveAdvance { scalar_start: u32 },
}

pub fn prepare_from_resolved_metrics_v1(
    story_id: &str,
    unit_id: &str,
    metrics: Vec<ResolvedScalarMetricV1>,
    environment_fingerprint: FingerprintV1,
    shaping_policy_fingerprint: FingerprintV1,
) -> Result<PreparedMetricsUnitV1, PrepareErrorV1> {
    if metrics.is_empty() {
        return Err(PrepareErrorV1::EmptyUnit);
    }

    for metric in &metrics {
        if metric.advance_emu <= 0 {
            return Err(PrepareErrorV1::NonPositiveAdvance {
                scalar_start: metric.scalar_start,
            });
        }
        if metric.scalar_end <= metric.scalar_start {
            return Err(PrepareErrorV1::NonContiguousScalarRange);
        }
    }

    for pair in metrics.windows(2) {
        if pair[0].scalar_end != pair[1].scalar_start {
            return Err(PrepareErrorV1::NonContiguousScalarRange);
        }
    }

    let scalar_start = metrics.first().expect("checked non-empty").scalar_start;
    let scalar_end = metrics.last().expect("checked non-empty").scalar_end;

    let mut input = Vec::new();
    push_bytes(&mut input, story_id.as_bytes());
    push_bytes(&mut input, unit_id.as_bytes());
    input.extend_from_slice(&environment_fingerprint);
    input.extend_from_slice(&shaping_policy_fingerprint);

    let mut output = Vec::new();
    for metric in &metrics {
        push_u32(&mut input, metric.scalar_start);
        push_u32(&mut input, metric.scalar_end);
        push_i64(&mut input, metric.advance_emu);
        input.push(u8::from(metric.break_after));
        input.extend_from_slice(&metric.semantic_fingerprint);

        push_u32(&mut output, metric.scalar_start);
        push_u32(&mut output, metric.scalar_end);
        push_i64(&mut output, metric.advance_emu);
        output.push(u8::from(metric.break_after));
        output.extend_from_slice(&metric.semantic_fingerprint);
    }

    Ok(PreparedMetricsUnitV1 {
        story_id: story_id.to_owned(),
        unit_id: unit_id.to_owned(),
        scalar_start,
        scalar_end,
        dependency_fingerprint: fingerprint_v1(PREPARED_METRICS_STAGE_V1, &[&input]),
        output_fingerprint: fingerprint_v1("prepared-metrics-output-v1", &[&output]),
        scalars: metrics,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameGeometryV1 {
    pub frame_id: String,
    pub page_id: String,
    pub width_emu: i64,
    pub height_emu: i64,
    pub line_height_emu: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RectObstacleV1 {
    pub obstacle_id: String,
    pub x0_emu: i64,
    pub y0_emu: i64,
    pub x1_emu: i64,
    pub y1_emu: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalV1 {
    pub x0_emu: i64,
    pub x1_emu: i64,
}

impl IntervalV1 {
    pub fn width_emu(self) -> i64 {
        self.x1_emu - self.x0_emu
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBandV1 {
    pub row_index: u32,
    pub y0_emu: i64,
    pub y1_emu: i64,
    pub intervals: Vec<IntervalV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRegionV1 {
    pub frame_id: String,
    pub dependency_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub bands: Vec<LineBandV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineRegionFlowBridgeErrorV1 {
    EmptyBandSet,
    NonMonotonicBandOrder,
    MultiIntervalConsumptionUnsupported,
}

pub fn bridge_line_region_slots_v1(
    frame_id: &str,
    slots: &[chaptera_line_region::LineBandSlotsV1],
    policy: chaptera_line_region::IntervalConsumptionPolicyV1,
) -> Result<LineRegionV1, LineRegionFlowBridgeErrorV1> {
    if slots.is_empty() {
        return Err(LineRegionFlowBridgeErrorV1::EmptyBandSet);
    }
    if policy == chaptera_line_region::IntervalConsumptionPolicyV1::AllIntervalsLeftToRight {
        return Err(LineRegionFlowBridgeErrorV1::MultiIntervalConsumptionUnsupported);
    }

    let mut previous_bottom = None;
    let mut bands = Vec::with_capacity(slots.len());
    let mut dependency = Vec::new();
    push_bytes(&mut dependency, frame_id.as_bytes());
    let mut output = Vec::new();

    for (index, slot) in slots.iter().enumerate() {
        if let Some(bottom) = previous_bottom {
            if slot.clipped_top_emu < bottom {
                return Err(LineRegionFlowBridgeErrorV1::NonMonotonicBandOrder);
            }
        }
        previous_bottom = Some(slot.clipped_bottom_emu);

        push_i64(&mut dependency, slot.requested.top_emu);
        push_i64(&mut dependency, slot.requested.bottom_emu);
        push_i64(&mut dependency, slot.clipped_top_emu);
        push_i64(&mut dependency, slot.clipped_bottom_emu);
        for interval in &slot.intervals {
            push_u32(&mut dependency, interval.column_index);
            push_i64(&mut dependency, interval.x0_emu);
            push_i64(&mut dependency, interval.x1_emu);
        }
        for diagnostic in &slot.diagnostics {
            push_bytes(&mut dependency, diagnostic.code.as_bytes());
            if let Some(obstacle_id) = &diagnostic.obstacle_id {
                push_bytes(&mut dependency, obstacle_id.as_bytes());
            }
            if let Some(detail) = &diagnostic.detail {
                push_bytes(&mut dependency, detail.as_bytes());
            }
        }

        let selected = chaptera_line_region::select_line_intervals_v1(slot, policy);
        let intervals = selected
            .into_iter()
            .map(|interval| IntervalV1 {
                x0_emu: interval.x0_emu,
                x1_emu: interval.x1_emu,
            })
            .collect::<Vec<_>>();
        let row_index = u32::try_from(index).expect("slot count is bounded by slice length");
        push_u32(&mut output, row_index);
        push_i64(&mut output, slot.clipped_top_emu);
        push_i64(&mut output, slot.clipped_bottom_emu);
        push_u32(
            &mut output,
            u32::try_from(intervals.len()).expect("selected interval count is bounded"),
        );
        for interval in &intervals {
            push_i64(&mut output, interval.x0_emu);
            push_i64(&mut output, interval.x1_emu);
        }

        bands.push(LineBandV1 {
            row_index,
            y0_emu: slot.clipped_top_emu,
            y1_emu: slot.clipped_bottom_emu,
            intervals,
        });
    }

    Ok(LineRegionV1 {
        frame_id: frame_id.to_owned(),
        dependency_fingerprint: fingerprint_v1(
            "chaptera-line-region-flow-bridge-dependency-v1",
            &[&dependency],
        ),
        output_fingerprint: fingerprint_v1(
            "chaptera-line-region-flow-bridge-output-v1",
            &[&output],
        ),
        bands,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionErrorV1 {
    NonPositiveFrameGeometry,
    InvalidObstacle { obstacle_id: String },
    RowCountOverflow,
}

pub fn resolve_line_regions_v1(
    frame: &FrameGeometryV1,
    obstacles: &[RectObstacleV1],
) -> Result<LineRegionV1, RegionErrorV1> {
    if frame.width_emu <= 0 || frame.height_emu <= 0 || frame.line_height_emu <= 0 {
        return Err(RegionErrorV1::NonPositiveFrameGeometry);
    }
    for obstacle in obstacles {
        if obstacle.x1_emu <= obstacle.x0_emu || obstacle.y1_emu <= obstacle.y0_emu {
            return Err(RegionErrorV1::InvalidObstacle {
                obstacle_id: obstacle.obstacle_id.clone(),
            });
        }
    }

    let row_count_i64 = (frame.height_emu + frame.line_height_emu - 1) / frame.line_height_emu;
    let row_count = u32::try_from(row_count_i64).map_err(|_| RegionErrorV1::RowCountOverflow)?;
    let mut bands = Vec::with_capacity(row_count as usize);

    for row_index in 0..row_count {
        let y0 = i64::from(row_index) * frame.line_height_emu;
        let y1 = (y0 + frame.line_height_emu).min(frame.height_emu);
        let mut intervals = vec![IntervalV1 {
            x0_emu: 0,
            x1_emu: frame.width_emu,
        }];

        for obstacle in obstacles {
            if obstacle.y0_emu < y1 && obstacle.y1_emu > y0 {
                let clip_x0 = obstacle.x0_emu.max(0).min(frame.width_emu);
                let clip_x1 = obstacle.x1_emu.max(0).min(frame.width_emu);
                if clip_x1 > clip_x0 {
                    intervals = subtract_interval_set(&intervals, clip_x0, clip_x1);
                }
            }
        }

        bands.push(LineBandV1 {
            row_index,
            y0_emu: y0,
            y1_emu: y1,
            intervals,
        });
    }

    let mut dependency = Vec::new();
    push_bytes(&mut dependency, frame.frame_id.as_bytes());
    push_bytes(&mut dependency, frame.page_id.as_bytes());
    push_i64(&mut dependency, frame.width_emu);
    push_i64(&mut dependency, frame.height_emu);
    push_i64(&mut dependency, frame.line_height_emu);
    for obstacle in obstacles {
        push_bytes(&mut dependency, obstacle.obstacle_id.as_bytes());
        push_i64(&mut dependency, obstacle.x0_emu);
        push_i64(&mut dependency, obstacle.y0_emu);
        push_i64(&mut dependency, obstacle.x1_emu);
        push_i64(&mut dependency, obstacle.y1_emu);
    }

    let mut output = Vec::new();
    for band in &bands {
        push_u32(&mut output, band.row_index);
        push_i64(&mut output, band.y0_emu);
        push_i64(&mut output, band.y1_emu);
        push_u32(
            &mut output,
            u32::try_from(band.intervals.len()).expect("bounded interval count"),
        );
        for interval in &band.intervals {
            push_i64(&mut output, interval.x0_emu);
            push_i64(&mut output, interval.x1_emu);
        }
    }

    Ok(LineRegionV1 {
        frame_id: frame.frame_id.clone(),
        dependency_fingerprint: fingerprint_v1(LINE_REGION_STAGE_V1, &[&dependency]),
        output_fingerprint: fingerprint_v1("line-region-output-v1", &[&output]),
        bands,
    })
}

fn subtract_interval_set(intervals: &[IntervalV1], cut_x0: i64, cut_x1: i64) -> Vec<IntervalV1> {
    let mut output = Vec::new();
    for interval in intervals {
        if cut_x1 <= interval.x0_emu || cut_x0 >= interval.x1_emu {
            output.push(*interval);
            continue;
        }
        if cut_x0 > interval.x0_emu {
            output.push(IntervalV1 {
                x0_emu: interval.x0_emu,
                x1_emu: cut_x0.min(interval.x1_emu),
            });
        }
        if cut_x1 < interval.x1_emu {
            output.push(IntervalV1 {
                x0_emu: cut_x1.max(interval.x0_emu),
                x1_emu: interval.x1_emu,
            });
        }
    }
    output
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntervalPolicyV1 {
    LargestOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowLineV1 {
    pub row_index: u32,
    pub x0_emu: i64,
    pub x1_emu: i64,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub measured_width_emu: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryFlowV1 {
    pub story_id: String,
    pub frame_id: String,
    pub dependency_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub lines: Vec<FlowLineV1>,
    pub next_scalar: u32,
    pub overset: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowErrorV1 {
    MissingPreparedUnits,
    NonContiguousPreparedUnits,
    NoUsableInterval { row_index: u32 },
    UnbreakableAt { scalar_start: u32 },
}

pub fn resolve_story_flow_v1(
    story_id: &str,
    prepared_units: &[PreparedMetricsUnitV1],
    regions: &LineRegionV1,
    policy: IntervalPolicyV1,
) -> Result<StoryFlowV1, FlowErrorV1> {
    if prepared_units.is_empty() {
        return Err(FlowErrorV1::MissingPreparedUnits);
    }

    let mut units = prepared_units.to_vec();
    units.sort_by_key(|unit| unit.scalar_start);
    for pair in units.windows(2) {
        if pair[0].scalar_end != pair[1].scalar_start {
            return Err(FlowErrorV1::NonContiguousPreparedUnits);
        }
    }

    let scalars = units
        .iter()
        .flat_map(|unit| unit.scalars.iter().cloned())
        .collect::<Vec<_>>();

    for pair in scalars.windows(2) {
        if pair[0].scalar_end != pair[1].scalar_start {
            return Err(FlowErrorV1::NonContiguousPreparedUnits);
        }
    }

    let mut lines = Vec::new();
    let mut cursor = 0usize;

    for band in &regions.bands {
        if cursor == scalars.len() {
            break;
        }

        let interval =
            choose_interval(&band.intervals, policy).ok_or(FlowErrorV1::NoUsableInterval {
                row_index: band.row_index,
            })?;
        let capacity = interval.width_emu();

        let mut width = 0_i64;
        let mut probe = cursor;
        let mut last_break = None;

        while probe < scalars.len() {
            let next = width.saturating_add(scalars[probe].advance_emu);
            if next > capacity {
                break;
            }
            width = next;
            probe += 1;
            if scalars[probe - 1].break_after || probe == scalars.len() {
                last_break = Some(probe);
            }
        }

        let end = if probe == scalars.len() {
            probe
        } else {
            last_break.ok_or(FlowErrorV1::UnbreakableAt {
                scalar_start: scalars[cursor].scalar_start,
            })?
        };

        let measured_width_emu = scalars[cursor..end]
            .iter()
            .map(|metric| metric.advance_emu)
            .sum();

        lines.push(FlowLineV1 {
            row_index: band.row_index,
            x0_emu: interval.x0_emu,
            x1_emu: interval.x1_emu,
            scalar_start: scalars[cursor].scalar_start,
            scalar_end: scalars[end - 1].scalar_end,
            measured_width_emu,
        });
        cursor = end;
    }

    let next_scalar = if cursor < scalars.len() {
        scalars[cursor].scalar_start
    } else {
        scalars.last().expect("non-empty").scalar_end
    };
    let overset = cursor < scalars.len();

    let mut dependency = Vec::new();
    for unit in &units {
        dependency.extend_from_slice(&unit.output_fingerprint);
    }
    dependency.extend_from_slice(&regions.output_fingerprint);
    dependency.push(match policy {
        IntervalPolicyV1::LargestOnly => 1,
    });

    let mut output = Vec::new();
    for line in &lines {
        push_u32(&mut output, line.row_index);
        push_i64(&mut output, line.x0_emu);
        push_i64(&mut output, line.x1_emu);
        push_u32(&mut output, line.scalar_start);
        push_u32(&mut output, line.scalar_end);
        push_i64(&mut output, line.measured_width_emu);
        for scalar in scalars.iter().filter(|scalar| {
            scalar.scalar_start >= line.scalar_start && scalar.scalar_end <= line.scalar_end
        }) {
            output.extend_from_slice(&scalar.semantic_fingerprint);
        }
    }
    push_u32(&mut output, next_scalar);
    output.push(u8::from(overset));

    Ok(StoryFlowV1 {
        story_id: story_id.to_owned(),
        frame_id: regions.frame_id.clone(),
        dependency_fingerprint: fingerprint_v1(STORY_FLOW_STAGE_V1, &[&dependency]),
        output_fingerprint: fingerprint_v1("story-flow-output-v1", &[&output]),
        lines,
        next_scalar,
        overset,
    })
}

fn choose_interval(intervals: &[IntervalV1], policy: IntervalPolicyV1) -> Option<IntervalV1> {
    match policy {
        IntervalPolicyV1::LargestOnly => intervals.iter().copied().max_by(|left, right| {
            left.width_emu()
                .cmp(&right.width_emu())
                .then_with(|| right.x0_emu.cmp(&left.x0_emu))
        }),
    }
}

pub fn prepared_receipt_v1(
    unit: &PreparedMetricsUnitV1,
    revision: &str,
    environment_node: &str,
    environment_fingerprint: FingerprintV1,
) -> ArtifactReceiptV1 {
    ArtifactReceiptV1 {
        artifact: ArtifactKeyV1::PreparedTypography {
            story_id: unit.story_id.clone(),
            unit_id: unit.unit_id.clone(),
        },
        observed_revision: revision.to_owned(),
        dependency_fingerprint: unit.dependency_fingerprint,
        output_fingerprint: unit.output_fingerprint,
        stage_version: PREPARED_METRICS_STAGE_V1.to_owned(),
        dependencies: vec![
            DependencyEdgeV1 {
                source: DependencyNodeV1::Story(unit.story_id.clone()),
                kind: DependencyKindV1::Typography,
                observed_fingerprint: unit.dependency_fingerprint,
            },
            DependencyEdgeV1 {
                source: DependencyNodeV1::LayoutEnvironment(environment_node.to_owned()),
                kind: DependencyKindV1::Environment,
                observed_fingerprint: environment_fingerprint,
            },
        ],
        artifact_dependencies: vec![],
    }
}

pub fn line_region_receipt_v1(
    region: &LineRegionV1,
    frame: &FrameGeometryV1,
    revision: &str,
    obstacles: &[RectObstacleV1],
) -> ArtifactReceiptV1 {
    let mut dependencies = vec![DependencyEdgeV1 {
        source: DependencyNodeV1::Node(frame.frame_id.clone()),
        kind: DependencyKindV1::Geometry,
        observed_fingerprint: region.dependency_fingerprint,
    }];
    dependencies.extend(obstacles.iter().map(|obstacle| DependencyEdgeV1 {
        source: DependencyNodeV1::Node(obstacle.obstacle_id.clone()),
        kind: DependencyKindV1::WrapConstraint,
        observed_fingerprint: region.dependency_fingerprint,
    }));

    ArtifactReceiptV1 {
        artifact: ArtifactKeyV1::LineRegion {
            frame_id: frame.frame_id.clone(),
        },
        observed_revision: revision.to_owned(),
        dependency_fingerprint: region.dependency_fingerprint,
        output_fingerprint: region.output_fingerprint,
        stage_version: LINE_REGION_STAGE_V1.to_owned(),
        dependencies,
        artifact_dependencies: vec![],
    }
}

pub fn story_flow_receipt_v1(
    flow: &StoryFlowV1,
    prepared_units: &[PreparedMetricsUnitV1],
    region: &LineRegionV1,
    revision: &str,
) -> ArtifactReceiptV1 {
    let mut artifact_dependencies = prepared_units
        .iter()
        .map(|unit| ArtifactDependencyV1 {
            upstream: ArtifactKeyV1::PreparedTypography {
                story_id: unit.story_id.clone(),
                unit_id: unit.unit_id.clone(),
            },
            kind: DependencyKindV1::Typography,
        })
        .collect::<Vec<_>>();
    artifact_dependencies.push(ArtifactDependencyV1 {
        upstream: ArtifactKeyV1::LineRegion {
            frame_id: region.frame_id.clone(),
        },
        kind: DependencyKindV1::Geometry,
    });

    ArtifactReceiptV1 {
        artifact: ArtifactKeyV1::StoryFlow {
            story_id: flow.story_id.clone(),
            frame_id: flow.frame_id.clone(),
        },
        observed_revision: revision.to_owned(),
        dependency_fingerprint: flow.dependency_fingerprint,
        output_fingerprint: flow.output_fingerprint,
        stage_version: STORY_FLOW_STAGE_V1.to_owned(),
        dependencies: vec![DependencyEdgeV1 {
            source: DependencyNodeV1::Story(flow.story_id.clone()),
            kind: DependencyKindV1::FlowTopology,
            observed_fingerprint: flow.dependency_fingerprint,
        }],
        artifact_dependencies,
    }
}

pub fn flow_expected_upstream_v1(
    prepared_units: &[PreparedMetricsUnitV1],
    region: &LineRegionV1,
) -> Vec<ExpectedUpstreamV1> {
    let mut expected = prepared_units
        .iter()
        .map(|unit| ExpectedUpstreamV1 {
            artifact: ArtifactKeyV1::PreparedTypography {
                story_id: unit.story_id.clone(),
                unit_id: unit.unit_id.clone(),
            },
            output_fingerprint: unit.output_fingerprint,
        })
        .collect::<Vec<_>>();
    expected.push(ExpectedUpstreamV1 {
        artifact: ArtifactKeyV1::LineRegion {
            frame_id: region.frame_id.clone(),
        },
        output_fingerprint: region.output_fingerprint,
    });
    expected
}

pub fn scene_shard_output_fingerprint_v1(page_id: &str, flow: &StoryFlowV1) -> FingerprintV1 {
    fingerprint_v1(
        SCENE_SHARD_STAGE_V1,
        &[page_id.as_bytes(), &flow.output_fingerprint],
    )
}

fn push_bytes(buffer: &mut Vec<u8>, value: &[u8]) {
    let len = u64::try_from(value.len()).expect("bounded runtime input");
    buffer.extend_from_slice(&len.to_be_bytes());
    buffer.extend_from_slice(value);
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

fn push_i64(buffer: &mut Vec<u8>, value: i64) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComputedArtifactV1, InvalidationGraphV1, PublicationChangeV1};

    fn semantic(label: &str) -> FingerprintV1 {
        fingerprint_v1("semantic-v1", &[label.as_bytes()])
    }

    fn env(label: &str) -> FingerprintV1 {
        fingerprint_v1("env-v1", &[label.as_bytes()])
    }

    fn policy() -> FingerprintV1 {
        fingerprint_v1("shape-policy-v1", &[b"fixed"])
    }

    fn unit(unit_id: &str, start: u32, widths: &[i64], labels: &[&str]) -> PreparedMetricsUnitV1 {
        let metrics = widths
            .iter()
            .zip(labels)
            .enumerate()
            .map(|(offset, (advance, label))| ResolvedScalarMetricV1 {
                scalar_start: start + u32::try_from(offset).expect("small fixture"),
                scalar_end: start + u32::try_from(offset + 1).expect("small fixture"),
                advance_emu: *advance,
                break_after: label.ends_with(' '),
                semantic_fingerprint: semantic(label),
            })
            .collect::<Vec<_>>();

        prepare_from_resolved_metrics_v1("s1", unit_id, metrics, env("font-a"), policy())
            .expect("prepare fixture")
    }

    fn frame(width_emu: i64) -> FrameGeometryV1 {
        FrameGeometryV1 {
            frame_id: "f1".into(),
            page_id: "p1".into(),
            width_emu,
            height_emu: 40,
            line_height_emu: 20,
        }
    }

    #[test]
    fn rectangular_obstacle_produces_multiple_ordered_intervals() {
        let region = resolve_line_regions_v1(
            &frame(100),
            &[RectObstacleV1 {
                obstacle_id: "o1".into(),
                x0_emu: 30,
                y0_emu: 0,
                x1_emu: 60,
                y1_emu: 20,
            }],
        )
        .expect("region");

        assert_eq!(
            region.bands[0].intervals,
            vec![
                IntervalV1 {
                    x0_emu: 0,
                    x1_emu: 30
                },
                IntervalV1 {
                    x0_emu: 60,
                    x1_emu: 100
                }
            ]
        );
        assert_eq!(
            region.bands[1].intervals,
            vec![IntervalV1 {
                x0_emu: 0,
                x1_emu: 100
            }]
        );
    }

    #[test]
    fn frame_resize_reuses_prepared_metrics_and_reflows_region_and_story() {
        let prepared = vec![
            unit("u1", 0, &[20, 20, 20], &["A", "B ", "C"]),
            unit("u2", 3, &[20, 20, 20], &["D", "E ", "F"]),
        ];

        let prepared_outputs_before = prepared
            .iter()
            .map(|unit| unit.output_fingerprint)
            .collect::<Vec<_>>();
        let wide_region = resolve_line_regions_v1(&frame(100), &[]).expect("wide region");
        let narrow_region = resolve_line_regions_v1(&frame(60), &[]).expect("narrow region");

        let wide_flow =
            resolve_story_flow_v1("s1", &prepared, &wide_region, IntervalPolicyV1::LargestOnly)
                .expect("wide flow");
        let narrow_flow = resolve_story_flow_v1(
            "s1",
            &prepared,
            &narrow_region,
            IntervalPolicyV1::LargestOnly,
        )
        .expect("narrow flow");

        assert_ne!(
            wide_region.output_fingerprint,
            narrow_region.output_fingerprint
        );
        assert_ne!(wide_flow.output_fingerprint, narrow_flow.output_fingerprint);
        assert_eq!(
            prepared
                .iter()
                .map(|unit| unit.output_fingerprint)
                .collect::<Vec<_>>(),
            prepared_outputs_before
        );
        assert_eq!(wide_flow.lines.len(), 2);
        assert_eq!(narrow_flow.lines.len(), 2);
        assert_ne!(wide_flow.lines, narrow_flow.lines);
    }

    #[test]
    fn story_edit_reprepares_only_changed_unit_and_reuses_line_region() {
        let before_u1 = unit("u1", 0, &[20, 20, 20], &["A", "B ", "C"]);
        let before_u2 = unit("u2", 3, &[20, 20, 20], &["D", "E ", "F"]);
        let region = resolve_line_regions_v1(&frame(80), &[]).expect("region");
        let before_flow = resolve_story_flow_v1(
            "s1",
            &[before_u1.clone(), before_u2.clone()],
            &region,
            IntervalPolicyV1::LargestOnly,
        )
        .expect("before flow");

        let after_u1 = unit("u1", 0, &[20, 30, 20], &["A", "B* ", "C"]);
        let after_u2 = before_u2.clone();
        let after_flow = resolve_story_flow_v1(
            "s1",
            &[after_u1.clone(), after_u2.clone()],
            &region,
            IntervalPolicyV1::LargestOnly,
        )
        .expect("after flow");

        assert_ne!(before_u1.output_fingerprint, after_u1.output_fingerprint);
        assert_eq!(before_u2.output_fingerprint, after_u2.output_fingerprint);
        assert_eq!(region.output_fingerprint, region.output_fingerprint);
        assert_ne!(
            before_flow.output_fingerprint,
            after_flow.output_fingerprint
        );
    }

    #[test]
    fn region_input_can_change_while_public_output_stays_equal() {
        let obstacle = RectObstacleV1 {
            obstacle_id: "o1".into(),
            x0_emu: 80,
            y0_emu: 0,
            x1_emu: 200,
            y1_emu: 40,
        };
        let first =
            resolve_line_regions_v1(&frame(100), std::slice::from_ref(&obstacle)).expect("first");
        let second =
            resolve_line_regions_v1(&frame(120), std::slice::from_ref(&obstacle)).expect("second");

        assert_ne!(first.dependency_fingerprint, second.dependency_fingerprint);
        assert_eq!(first.output_fingerprint, second.output_fingerprint);
    }

    #[test]
    fn graph_wires_prepared_and_region_artifacts_into_flow() {
        let prepared = vec![
            unit("u1", 0, &[20, 20], &["A", "B "]),
            unit("u2", 2, &[20, 20], &["C", "D "]),
        ];
        let frame = frame(80);
        let region = resolve_line_regions_v1(&frame, &[]).expect("region");
        let flow = resolve_story_flow_v1("s1", &prepared, &region, IntervalPolicyV1::LargestOnly)
            .expect("flow");

        let mut graph = InvalidationGraphV1::default();
        for prepared_unit in &prepared {
            graph
                .publish(ComputedArtifactV1 {
                    receipt: prepared_receipt_v1(prepared_unit, "r1", "layout-env", env("font-a")),
                    expected_upstream: vec![],
                    output_comparable: true,
                })
                .expect("publish prepared");
        }
        graph
            .publish(ComputedArtifactV1 {
                receipt: line_region_receipt_v1(&region, &frame, "r1", &[]),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .expect("publish region");
        graph
            .publish(ComputedArtifactV1 {
                receipt: story_flow_receipt_v1(&flow, &prepared, &region, "r1"),
                expected_upstream: flow_expected_upstream_v1(&prepared, &region),
                output_comparable: true,
            })
            .expect("publish flow");

        let flow_key = ArtifactKeyV1::StoryFlow {
            story_id: "s1".into(),
            frame_id: "f1".into(),
        };
        for unit_id in ["u1", "u2"] {
            let prepared_key = ArtifactKeyV1::PreparedTypography {
                story_id: "s1".into(),
                unit_id: unit_id.into(),
            };
            assert!(
                graph
                    .artifact_consumers_of(&prepared_key)
                    .contains(&flow_key)
            );
        }
        let region_key = ArtifactKeyV1::LineRegion {
            frame_id: "f1".into(),
        };
        assert!(graph.artifact_consumers_of(&region_key).contains(&flow_key));
    }

    #[test]
    fn unchanged_region_public_contract_stops_real_stage_propagation() {
        let obstacle = RectObstacleV1 {
            obstacle_id: "o1".into(),
            x0_emu: 80,
            y0_emu: 0,
            x1_emu: 200,
            y1_emu: 40,
        };
        let first_frame = frame(100);
        let second_frame = frame(120);
        let first =
            resolve_line_regions_v1(&first_frame, std::slice::from_ref(&obstacle)).expect("first");
        let second = resolve_line_regions_v1(&second_frame, std::slice::from_ref(&obstacle))
            .expect("second");

        let mut graph = InvalidationGraphV1::default();
        graph
            .publish(ComputedArtifactV1 {
                receipt: line_region_receipt_v1(
                    &first,
                    &first_frame,
                    "r1",
                    std::slice::from_ref(&obstacle),
                ),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .expect("publish first");

        let result = graph
            .publish(ComputedArtifactV1 {
                receipt: line_region_receipt_v1(
                    &second,
                    &second_frame,
                    "r2",
                    std::slice::from_ref(&obstacle),
                ),
                expected_upstream: vec![],
                output_comparable: true,
            })
            .expect("publish second");

        assert_eq!(result.change, PublicationChangeV1::Unchanged);
    }

    #[test]
    fn incremental_resize_equals_clean_recompute() {
        let prepared = vec![
            unit("u1", 0, &[20, 20, 20], &["A", "B ", "C"]),
            unit("u2", 3, &[20, 20, 20], &["D", "E ", "F"]),
        ];
        let resized_region = resolve_line_regions_v1(&frame(60), &[]).expect("region");

        let incremental = resolve_story_flow_v1(
            "s1",
            &prepared,
            &resized_region,
            IntervalPolicyV1::LargestOnly,
        )
        .expect("incremental");

        let clean_prepared = vec![
            unit("u1", 0, &[20, 20, 20], &["A", "B ", "C"]),
            unit("u2", 3, &[20, 20, 20], &["D", "E ", "F"]),
        ];
        let clean_region = resolve_line_regions_v1(&frame(60), &[]).expect("clean region");
        let clean = resolve_story_flow_v1(
            "s1",
            &clean_prepared,
            &clean_region,
            IntervalPolicyV1::LargestOnly,
        )
        .expect("clean");

        assert_eq!(incremental, clean);
        assert_eq!(
            scene_shard_output_fingerprint_v1("p1", &incremental),
            scene_shard_output_fingerprint_v1("p1", &clean)
        );
    }
}


#[cfg(test)]
mod line_region_bridge_tests {
    use super::*;
    use chaptera_line_region::{
        ColumnsV1, EffectiveWrapModeV1, EffectiveWrapObstacleV1, FrameRegionV1,
        InsetsEmuV1, IntervalConsumptionPolicyV1, LineBandRequestV1, RectEmuV1,
        WrapDistancesV1, resolve_line_bands_v1,
    };

    fn slots() -> Vec<chaptera_line_region::LineBandSlotsV1> {
        let frame = FrameRegionV1 {
            frame_id: "f-bridge".into(),
            page_id: "p-bridge".into(),
            bounds: RectEmuV1 {
                x: 0,
                y: 0,
                width: 100,
                height: 40,
            },
            insets: InsetsEmuV1 {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
        };
        let obstacle = EffectiveWrapObstacleV1 {
            obstacle_id: "o".into(),
            mode: EffectiveWrapModeV1::Rectangle {
                rect: RectEmuV1 {
                    x: 30,
                    y: 0,
                    width: 30,
                    height: 20,
                },
                distances: WrapDistancesV1 {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
            },
        };
        resolve_line_bands_v1(
            &frame,
            ColumnsV1 {
                count: 1,
                gutter_emu: 0,
            },
            &[obstacle],
            &[
                LineBandRequestV1 {
                    top_emu: 0,
                    bottom_emu: 20,
                },
                LineBandRequestV1 {
                    top_emu: 20,
                    bottom_emu: 40,
                },
            ],
        )
        .expect("slots")
    }

    #[test]
    fn largest_only_bridge_feeds_existing_story_flow_shape() {
        let bridged = bridge_line_region_slots_v1(
            "f-bridge",
            &slots(),
            IntervalConsumptionPolicyV1::LargestOnly,
        )
        .expect("bridge");
        assert_eq!(2, bridged.bands.len());
        assert_eq!(
            vec![IntervalV1 {
                x0_emu: 60,
                x1_emu: 100,
            }],
            bridged.bands[0].intervals
        );
        assert_eq!(
            vec![IntervalV1 {
                x0_emu: 0,
                x1_emu: 100,
            }],
            bridged.bands[1].intervals
        );
    }

    #[test]
    fn multi_interval_consumption_fails_closed_at_legacy_flow_boundary() {
        assert_eq!(
            Err(LineRegionFlowBridgeErrorV1::MultiIntervalConsumptionUnsupported),
            bridge_line_region_slots_v1(
                "f-bridge",
                &slots(),
                IntervalConsumptionPolicyV1::AllIntervalsLeftToRight,
            )
        );
    }
}
